//! `xtask corpus pack` — build / refresh a single-file corpus archive
//! (L-5b of ADR-0020).
//!
//! The pack step walks a directory tree of `.txt` Aozora source files
//! and writes a single binary archive. Four variants:
//!
//! ```text
//! xtask corpus pack <SRC> <OUT>             # raw SJIS, no compression
//! xtask corpus pack <SRC> <OUT> --utf8      # pre-decoded UTF-8
//! xtask corpus pack <SRC> <OUT> --zstd      # raw SJIS, zstd-compressed
//! xtask corpus pack <SRC> <OUT> --utf8 --zstd  # the trifecta
//! ```
//!
//! ## Incremental rebuild (L-5c)
//!
//! If `<OUT>` already exists and parses as a valid archive with the
//! same flags, the pack is **incremental**: each source file's
//! `mtime_ns` is compared with the previous archive's record, and a
//! per-file `blake3` hash is computed only when `mtime` says "may
//! have changed". Unchanged entries are copied verbatim from the
//! previous archive (already-compressed payload bytes flow through
//! without re-encoding).
//!
//! Reported as `(reused / new / removed)` so the operator can tell
//! at a glance how much work the pack actually did.

#![allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::fn_params_excessive_bools,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::cast_precision_loss,
    clippy::absolute_paths,
    clippy::str_to_string,
    reason = "xtask CLI module — pack flow is intrinsically a 5-arg pipeline (src, out, utf8, zstd, level) and reads top-down; splitting it into helpers would obscure the staged measurement-printing layout that mirrors the bench harness's per-phase split. Casts and bool flags mirror the on-disk archive format documented in `aozora_corpus::archive`."
)]

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use clap::{Args, Subcommand};
use rayon::prelude::*;

use aozora_corpus::{Archive, ArchiveBuilder, EntryMeta, FilesystemCorpus, archive};
use aozora_encoding::decode_sjis;

#[derive(Args)]
pub(crate) struct CorpusArgs {
    #[command(subcommand)]
    pub(crate) target: CorpusTarget,
}

#[derive(Subcommand)]
pub(crate) enum CorpusTarget {
    /// Build / refresh a single-file binary archive from a directory
    /// of `.txt` files. Incremental: reuses unchanged entries from a
    /// previous archive at `<OUT>` if one exists with matching flags.
    Pack {
        /// Source directory — typically the Aozora corpus checkout
        /// (e.g. `~/aozora-corpus/aozorabunko_text-master/cards`).
        src: PathBuf,
        /// Output archive path. Conventional extensions: `.aozc`
        /// (raw), `.aozc.utf8`, `.aozc.zst`, `.aozc.utf8.zst`.
        out: PathBuf,
        /// Pre-decode Shift-JIS source bytes into UTF-8 before
        /// packing. Eliminates the runtime `decode_sjis` cost
        /// entirely; archive becomes ~50 % larger on disk because
        /// SJIS-Japanese is denser than UTF-8.
        #[arg(long)]
        utf8: bool,
        /// zstd-compress each entry's payload. Combine with
        /// `--utf8` for the smallest total disk + smallest runtime
        /// load wall (single read + parallel decompress).
        #[arg(long)]
        zstd: bool,
        /// zstd compression level (1..=22). Default 9 — high ratio
        /// with reasonable build wall. Level 19 is the long-mode
        /// max but ~10× slower to encode. Has no effect without
        /// `--zstd`.
        #[arg(long, default_value_t = 9)]
        zstd_level: i32,
    },
    /// Inspect an existing archive — header flags, entry count,
    /// total bytes, sample of labels.
    Stat {
        /// Archive path.
        archive: PathBuf,
    },
}

pub(crate) fn dispatch(args: &CorpusArgs) -> Result<(), String> {
    match &args.target {
        CorpusTarget::Pack {
            src,
            out,
            utf8,
            zstd,
            zstd_level,
        } => pack(src, out, *utf8, *zstd, *zstd_level),
        CorpusTarget::Stat { archive } => stat(archive),
    }
}

fn pack(src: &Path, out: &Path, utf8: bool, zstd: bool, zstd_level: i32) -> Result<(), String> {
    if !src.is_dir() {
        return Err(format!("source is not a directory: {}", src.display()));
    }
    let flags =
        (if zstd { archive::FLAG_ZSTD } else { 0 }) | (if utf8 { archive::FLAG_UTF8 } else { 0 });

    eprintln!(
        "xtask corpus pack: src={} out={} flags={}{}",
        src.display(),
        out.display(),
        if utf8 { "UTF8 " } else { "SJIS " },
        if zstd { "ZSTD" } else { "RAW" },
    );

    let total_start = Instant::now();

    // Step 1: enumerate source files.
    let walk_start = Instant::now();
    let corpus = FilesystemCorpus::new(src.to_path_buf())
        .map_err(|e| format!("invalid corpus root: {e:?}"))?;
    let paths: Vec<PathBuf> = corpus.walk_paths().filter_map(Result::ok).collect();
    eprintln!(
        "  walkdir : {:>5} files in {:>5.2} s",
        paths.len(),
        walk_start.elapsed().as_secs_f64()
    );

    // Step 2: load previous archive (incremental cache lookup) if
    // present and the flags match.
    let prev: Option<PrevArchive> = match Archive::open(out) {
        Ok(arc) if arc.flags() == flags => {
            eprintln!(
                "  prev    : reusing {} entries from existing archive (matching flags)",
                arc.len()
            );
            Some(PrevArchive::from(arc))
        }
        Ok(arc) => {
            eprintln!(
                "  prev    : found existing archive but flags differ ({} vs {flags}); rebuilding from scratch",
                arc.flags()
            );
            None
        }
        Err(_) => {
            eprintln!("  prev    : no existing archive at output path; building from scratch");
            None
        }
    };

    // Step 3: per-source-file decision (reuse vs re-pack), in
    // parallel via rayon. The decision body is pure-CPU + filesystem
    // metadata; it does not touch the in-progress builder, so no
    // shared mutability.
    let scan_start = Instant::now();
    let decisions: Vec<EntryDecision> = paths
        .par_iter()
        .filter_map(|path| classify_entry(path, src, prev.as_ref(), utf8).ok())
        .collect();
    eprintln!(
        "  scan    : {:>5} entries decided in {:>5.2} s",
        decisions.len(),
        scan_start.elapsed().as_secs_f64()
    );
    let reused = decisions
        .iter()
        .filter(|d| matches!(d.action, EntryAction::Reuse))
        .count();
    let fresh = decisions.len() - reused;
    let removed = prev.as_ref().map_or(0, |p| {
        let alive: std::collections::HashSet<&str> =
            decisions.iter().map(|d| d.label.as_str()).collect();
        p.lookup
            .keys()
            .filter(|l| !alive.contains(l.as_str()))
            .count()
    });

    // Step 4: assemble. Two sub-steps so the slow zstd encode runs
    // in parallel — without this, level-9 encoding of 17 k entries
    // serialised through `push_entry` takes minutes; with par
    // encoding it's seconds.
    //
    // 4a. Sort by label for deterministic on-disk layout (helpful
    //     for diff / reproducible-build verification).
    let mut sorted = decisions;
    sorted.sort_by(|a, b| a.label.cmp(&b.label));

    // 4b. Encode all `Encode` entries in parallel — produces
    //     `(label, payload_bytes, decoded_len, mtime_ns, source_blake3)`
    //     tuples ready for sequential append.
    let encode_start = Instant::now();
    let prepared: Vec<PreparedEntry> = sorted
        .into_par_iter()
        .map(|decision| match decision.action {
            EntryAction::Reuse => {
                let prev_arc = prev.as_ref().expect("Reuse only emitted with prev set");
                let (meta, payload) = prev_arc.entry_payload(&decision.label);
                PreparedEntry::Prebuilt {
                    meta,
                    payload: payload.to_vec(),
                }
            }
            EntryAction::Encode {
                payload_bytes,
                mtime_ns,
                source_blake3,
            } => {
                let decoded_len =
                    u32::try_from(payload_bytes.len()).expect("entry larger than u32 unsupported");
                let payload = if flags & archive::FLAG_ZSTD != 0 {
                    let mut compressed = Vec::with_capacity(payload_bytes.len() / 4);
                    zstd::stream::copy_encode(
                        payload_bytes.as_slice(),
                        &mut compressed,
                        zstd_level,
                    )
                    .expect("zstd encode must succeed on valid input");
                    compressed
                } else {
                    payload_bytes
                };
                PreparedEntry::Encoded {
                    label: decision.label,
                    payload,
                    decoded_len,
                    mtime_ns,
                    source_blake3,
                }
            }
        })
        .collect();
    eprintln!(
        "  encode  : {:>5} entries encoded in {:>5.2} s ({} compression)",
        prepared.len(),
        encode_start.elapsed().as_secs_f64(),
        if zstd {
            format!("zstd-{zstd_level}")
        } else {
            "none".to_string()
        },
    );

    // 4c. Sequential append into the builder + write to disk.
    let assemble_start = Instant::now();
    let mut builder = ArchiveBuilder::new(flags);
    builder.zstd_level(zstd_level);
    for entry in prepared {
        match entry {
            PreparedEntry::Prebuilt { meta, payload } => {
                builder.push_prebuilt(meta, &payload);
            }
            PreparedEntry::Encoded {
                label,
                payload,
                decoded_len,
                mtime_ns,
                source_blake3,
            } => {
                builder.push_already_encoded(
                    &label,
                    &payload,
                    decoded_len,
                    mtime_ns,
                    source_blake3,
                );
            }
        }
    }
    let bytes_written = builder
        .finish(out)
        .map_err(|e| format!("write archive: {e}"))?;
    eprintln!(
        "  assemble: {:>6.2} MB written in {:>5.2} s",
        bytes_written as f64 / 1_048_576.0,
        assemble_start.elapsed().as_secs_f64()
    );
    eprintln!(
        "  totals  : {reused} reused / {fresh} fresh / {removed} removed; total wall {:.2} s",
        total_start.elapsed().as_secs_f64()
    );
    Ok(())
}

/// Stat an existing archive — print human-readable header + sample
/// entries.
fn stat(path: &Path) -> Result<(), String> {
    let arc = Archive::open(path).map_err(|e| format!("open: {e}"))?;
    let bytes_on_disk = fs::metadata(path).map_or(0, |m| m.len());
    let total_decoded: u64 = arc.entries().iter().map(|e| u64::from(e.decoded_len)).sum();
    let total_payload: u64 = arc.entries().iter().map(|e| u64::from(e.payload_len)).sum();
    println!("Archive: {}", path.display());
    println!(
        "  flags        : {}{}",
        if arc.is_utf8() { "UTF8 " } else { "SJIS " },
        if arc.is_zstd() { "ZSTD" } else { "RAW" },
    );
    println!("  entries      : {}", arc.len());
    println!(
        "  file size    : {:.2} MB (header + index + payload)",
        bytes_on_disk as f64 / 1_048_576.0
    );
    println!(
        "  payload sum  : {:.2} MB ({:.2} MB on disk)",
        total_decoded as f64 / 1_048_576.0,
        total_payload as f64 / 1_048_576.0
    );
    if total_payload > 0 && total_decoded > total_payload {
        let ratio = total_decoded as f64 / total_payload as f64;
        println!("  zstd ratio   : {ratio:.2}× decoded vs on-disk");
    }
    println!("  sample labels:");
    for entry in arc.entries().iter().take(5) {
        println!(
            "    {:>10} bytes (decoded {:>10})  {}",
            entry.payload_len, entry.decoded_len, entry.label
        );
    }
    if arc.len() > 5 {
        println!("    ... ({} more)", arc.len() - 5);
    }
    Ok(())
}

/// Per-source-file outcome of the incremental scan.
struct EntryDecision {
    label: String,
    action: EntryAction,
}

enum EntryAction {
    /// Keep the previous archive's entry verbatim (mtime + hash
    /// matched).
    Reuse,
    /// Re-encode and re-pack. Carries the loaded payload bytes so
    /// the builder doesn't have to re-read them, plus the
    /// source-file blake3 hash (computed over the on-disk raw
    /// bytes, NOT over the post-decode payload — so incremental
    /// matching is consistent across archive flavours: a `--utf8`
    /// archive's `source_blake3` still hashes the original SJIS
    /// source file).
    Encode {
        payload_bytes: Vec<u8>,
        mtime_ns: i64,
        source_blake3: [u8; 32],
    },
}

/// Output of the parallel encode step (4b in `pack`). Each entry is
/// either copied from a previous archive verbatim, or freshly
/// encoded with the new compression / decode settings.
enum PreparedEntry {
    Prebuilt {
        meta: EntryMeta,
        payload: Vec<u8>,
    },
    Encoded {
        label: String,
        payload: Vec<u8>,
        decoded_len: u32,
        mtime_ns: i64,
        source_blake3: [u8; 32],
    },
}

fn classify_entry(
    path: &Path,
    src_root: &Path,
    prev: Option<&PrevArchive>,
    utf8: bool,
) -> Result<EntryDecision, std::io::Error> {
    let label = path
        .strip_prefix(src_root)
        .map_err(|_| std::io::Error::other("path outside src root"))?
        .display()
        .to_string();

    let mtime_ns = fs::metadata(path)
        .and_then(|m| m.modified())
        .map_or(0, archive::system_time_to_ns);

    let bytes = fs::read(path)?;
    let source_blake3: [u8; 32] = blake3::hash(&bytes).into();

    if let Some(prev) = prev
        && let Some(prev_meta) = prev.lookup.get(&label)
        && prev_meta.source_mtime_ns == mtime_ns
        && prev_meta.source_blake3 == source_blake3
    {
        // mtime + hash match → previous entry's encoded payload is
        // still valid; reuse verbatim.
        return Ok(EntryDecision {
            label,
            action: EntryAction::Reuse,
        });
    }

    // No previous archive, or label unseen, or content drifted —
    // re-encode the payload (decode SJIS → UTF-8 here for utf8
    // archives) but keep `source_blake3` pinned to the raw source
    // bytes so the next incremental pack can match identity.
    let payload_bytes = if utf8 {
        decode_sjis(&bytes).map(String::into_bytes).unwrap_or(bytes)
    } else {
        bytes
    };
    Ok(EntryDecision {
        label,
        action: EntryAction::Encode {
            payload_bytes,
            mtime_ns,
            source_blake3,
        },
    })
}

/// Wrapper around [`Archive`] that exposes `(meta, payload)` lookup by
/// label — needed by the incremental-pack `EntryAction::Reuse` path,
/// which copies pre-encoded payload bytes verbatim into the new
/// archive.
struct PrevArchive {
    arc: Archive,
    /// label → entry index in `arc.entries()`. Built once at open
    /// time so the per-decision lookup is O(1).
    lookup: HashMap<String, EntryMeta>,
    /// label → entry index in `arc.entries()` for `raw_payload`
    /// access.
    by_index: HashMap<String, usize>,
}

impl From<Archive> for PrevArchive {
    fn from(arc: Archive) -> Self {
        let mut lookup = HashMap::with_capacity(arc.len());
        let mut by_index = HashMap::with_capacity(arc.len());
        for (i, entry) in arc.entries().iter().enumerate() {
            lookup.insert(entry.label.clone(), entry.clone());
            by_index.insert(entry.label.clone(), i);
        }
        Self {
            arc,
            lookup,
            by_index,
        }
    }
}

impl PrevArchive {
    fn entry_payload(&self, label: &str) -> (EntryMeta, &[u8]) {
        let i = self.by_index[label];
        let meta = self.arc.entries()[i].clone();
        let payload = self.arc.raw_payload(i);
        (meta, payload)
    }
}

#[allow(
    dead_code,
    reason = "OsString is used by the parent module's command surface; clippy can't see across module boundaries"
)]
fn _unused_marker(_: OsString) {}
