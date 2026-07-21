//! `xtask corpus pack` — build / refresh a single-file corpus archive
//!.
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
//! ## Incremental rebuild
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
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::absolute_paths,
    clippy::str_to_string,
    reason = "xtask CLI module — pack/stat flows mirror the on-disk archive format and the bench harness's per-phase output. Casts (u32 → f64 for KB / MB display, i64 → u64 for mtime → SystemTime, etc.) are intrinsic to the format-display work; replacing them with try_from would clutter the column-aligned output without any safety win at the value ranges involved (corpus byte counts < 4 GB, mtimes well within signed 64-bit range)."
)]
#![allow(
    clippy::naive_bytecount,
    reason = "the audit's line_of counts '\\n' in a one-shot prefix; pulling in the bytecount crate for an offline measurement tool is not worth a new dependency."
)]

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use clap::{Args, Subcommand};
use rayon::prelude::*;

use aozora::AOZORA_CLASSES;
use aozora::decode_auto;
use aozora::{Catalogue, CatalogueMatch, DirectiveClass, NodeKind};
use aozora_corpus::{
    Archive, ArchiveBuilder, CorpusItem, EntryMeta, FilesystemCorpus, archive, par_load_decoded,
};
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::panic::{self, AssertUnwindSafe};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
    /// per-band breakdown (count, mean/median/max bytes, on-disk
    /// payload bytes, compression ratio for zstd archives), top-K
    /// largest entries, mtime distribution.
    Stat {
        /// Archive path.
        archive: PathBuf,
        /// How many of the largest entries to print at the end.
        #[arg(long, default_value_t = 10)]
        top: usize,
    },
    /// Evict a file (or every regular file under a directory tree)
    /// from the kernel page cache via `posix_fadvise(POSIX_FADV_DONTNEED)`.
    /// Used to make cold-cache load benchmarks reproducible without
    /// `sudo drop_caches` (which flushes the *entire* page cache —
    /// disruptive to anything else running on the host).
    ///
    /// Works without elevated privileges for files the user owns
    /// because `posix_fadvise(2)` is per-fd, not system-wide.
    Uncache {
        /// File or directory to evict from the page cache. For
        /// directories, walks recursively and evicts every regular
        /// file. Reports the total bytes evicted.
        path: PathBuf,
    },
    /// Parse every `.txt` in the corpus and report what the parser
    /// *actually* does on real data — the empirical ground truth for
    /// spec-conformance work, independent of any documentation or
    /// hand-written vector.
    ///
    /// Headline output: the distinct `［＃…］` bodies that fall through
    /// to `DirectiveKind::Unknown` (the true unsupported set), with
    /// per-body counts, a first-seen `label:line`, and a normalized
    /// "shape" grouping (`「…」` → `「」`, digit runs → `N`) so families
    /// of unsupported directives surface. Also: per-`NodeKind`
    /// frequency, the `DirectiveKind` breakdown, the 外字 address-form
    /// distribution (+ unresolved count), diagnostics by code, and
    /// decode-error / parse-panic counts.
    ///
    /// The JSON report goes to stdout (or `--out`); the human summary
    /// goes to stderr, so the JSON pipes clean. The report is a
    /// throwaway measurement — it must NEVER be committed back into the
    /// conformance vectors, because feeding parser output into the
    /// vectors is exactly the circularity this audit exists to expose.
    Audit {
        /// Corpus root directory of `.txt` files. Defaults to
        /// `$AOZORA_CORPUS_ROOT`.
        #[arg(long)]
        root: Option<PathBuf>,
        /// Write the JSON report here instead of stdout.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Rows per table in the human (stderr) summary. The JSON
        /// report always carries the full, untruncated lists.
        #[arg(long, default_value_t = 40)]
        top: usize,
        /// Process at most N files (debugging; default: whole corpus).
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Strict corpus conformance gate.
    AuditGate {
        /// Corpus root directory of `.txt` files. Defaults to
        /// `$AOZORA_CORPUS_ROOT`.
        #[arg(long)]
        root: Option<PathBuf>,
    },
    /// Verbatim-provenance gate: fail (exit 1) when any corpus document's
    /// `Snapshot::to_source_verbatim()` no longer equals its decoded original
    /// source. Binary — one byte of drift fails. Needs a corpus
    /// (`$AOZORA_CORPUS_ROOT` or `--root`); gracefully skips (exit 0) when
    /// none is set.
    Verbatim {
        /// Corpus root directory of `.txt` files. Defaults to
        /// `$AOZORA_CORPUS_ROOT`.
        #[arg(long)]
        root: Option<PathBuf>,
    },
    /// Strict render-leak gate.
    RenderLeakGate {
        /// Corpus root directory of `.txt` files. Defaults to
        /// `$AOZORA_CORPUS_ROOT`.
        #[arg(long)]
        root: Option<PathBuf>,
    },
    /// Strict render-correctness gate.
    RenderCorrectnessGate {
        /// Corpus root directory of `.txt` files. Defaults to
        /// `$AOZORA_CORPUS_ROOT`.
        #[arg(long)]
        root: Option<PathBuf>,
    },
    /// Select a stratified, family-diverse set of real works to extend the
    /// `fixtures/works/` golden set (WS-1 / #414). Deterministic greedy
    /// weighted set-cover over the notation families, seeded by the works
    /// already vendored, excluding Unknown-dominated, unclean, and >500 KiB
    /// works. Writes a TOML manifest whose `slug` fields are blank for a human
    /// to fill (romanising the kanji author/title is not automatable).
    SelectWorks {
        /// Corpus root directory of `.txt` files. Defaults to `$AOZORA_CORPUS_ROOT`.
        #[arg(long)]
        root: Option<PathBuf>,
        /// How many works to add on top of the already-vendored set.
        #[arg(long, default_value_t = 66)]
        target: usize,
        /// Selection manifest to write.
        #[arg(
            long,
            default_value = "crates/aozora-conformance/fixtures/works-selection.toml"
        )]
        out: PathBuf,
        /// Directory of already-vendored works — seeds coverage and is excluded
        /// from re-selection (matched by `source.txt` content).
        #[arg(long, default_value = "crates/aozora-conformance/fixtures/works")]
        vendored: PathBuf,
        /// Maximum Unknown-annotation ratio a candidate may carry.
        #[arg(long, default_value_t = 0.25)]
        max_unknown_ratio: f64,
        /// Hard budget on total vendored `source.txt` bytes — the objective is
        /// weighted family coverage *per byte*, so small works that cover rare
        /// families win. Default ~2 MiB keeps the golden set lean.
        #[arg(long, default_value_t = 2_000_000)]
        max_total_source_bytes: usize,
        /// Force-include the smallest clean corpus work exercising each named
        /// family (repeatable), on top of the greedy selection — the direct way
        /// to fill a specific uncovered family. A family no clean work exercises
        /// (e.g. `rubyRetarget`) is reported as unsatisfiable (craft a
        /// `fixtures/render/` fixture instead).
        #[arg(long = "require-family", value_name = "FAMILY")]
        require_family: Vec<String>,
    },
    /// Vendor the works named in a selection manifest into `fixtures/works/`:
    /// decode each corpus file, normalise CRLF→LF, and write `source.txt`.
    /// Skips rows with an empty `slug`. Does NOT write `expected.html` — seed
    /// that with `UPDATE_GOLDEN=1 cargo test -p aozora-conformance --test works_gate`.
    VendorWorks {
        /// Corpus root directory of `.txt` files. Defaults to `$AOZORA_CORPUS_ROOT`.
        #[arg(long)]
        root: Option<PathBuf>,
        /// Selection manifest to read.
        #[arg(
            long,
            default_value = "crates/aozora-conformance/fixtures/works-selection.toml"
        )]
        manifest: PathBuf,
        /// Destination works directory.
        #[arg(long, default_value = "crates/aozora-conformance/fixtures/works")]
        dest: PathBuf,
    },
    /// Classify the corpus `Unknown` residue against the notation-hygiene
    /// catalogues — the reproducible mining worklist for growing them.
    ///
    /// Runs the full audit, then buckets every raw Unknown body by whether
    /// Tier1 (`canonical_directive`), Tier2 (`degraded_directive`), or neither
    /// (the discovery *residue*) resolves it. The residue is shape-aggregated
    /// and occurrence-ranked with a raw example each. Like `audit`, the JSON
    /// report is a throwaway measurement — never committed. Needs a corpus.
    ClassifyUnknown {
        /// Corpus root directory of `.txt` files. Defaults to `$AOZORA_CORPUS_ROOT`.
        #[arg(long)]
        root: Option<PathBuf>,
        /// Write the JSON report here instead of stdout.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Residue shapes to print in the human (stderr) summary.
        #[arg(long, default_value_t = 40)]
        top: usize,
    },
    /// Gate golden family coverage: every notation family must be exercised by
    /// the golden fixtures. Counts the union of the vendored golden works
    /// (`fixtures/works/`) AND the crafted render fixtures (`fixtures/render/`),
    /// so a structurally-rare family with no clean corpus work is covered by a
    /// crafted fixture. Reads only committed fixtures, so it needs no corpus.
    FamilyCoverage {
        /// Vendored golden-works directory to measure.
        #[arg(long, default_value = "crates/aozora-conformance/fixtures/works")]
        vendored: PathBuf,
        /// Crafted render-fixtures directory, also counted toward coverage.
        #[arg(long, default_value = "crates/aozora-conformance/fixtures/render")]
        render: PathBuf,
    },
    /// Probe which of the 43 notation families a single source exercises — the
    /// authoring aid for crafting a `fixtures/render/` fixture that fills a
    /// specific uncovered family (run it on candidate notation until the target
    /// family appears). Uses the same `analyze` walk `family-coverage` counts, so
    /// its answer is exactly what the golden set would gain. Reads no corpus.
    FamilyProbe {
        /// Source text to analyse inline.
        #[arg(long, conflicts_with = "file")]
        text: Option<String>,
        /// Read the source from this file instead of `--text`.
        #[arg(long)]
        file: Option<PathBuf>,
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
        CorpusTarget::Stat { archive, top } => stat(archive, *top),
        CorpusTarget::Uncache { path } => uncache(path),
        CorpusTarget::Audit {
            root,
            out,
            top,
            limit,
        } => audit(root.as_deref(), out.as_deref(), *top, *limit),
        CorpusTarget::AuditGate { root } => audit_gate(root.as_deref()),
        CorpusTarget::Verbatim { root } => verbatim_gate(root.as_deref()),
        CorpusTarget::RenderLeakGate { root } => render_leak_gate(root.as_deref()),
        CorpusTarget::RenderCorrectnessGate { root } => render_correctness_gate(root.as_deref()),
        CorpusTarget::SelectWorks {
            root,
            target,
            out,
            vendored,
            max_unknown_ratio,
            max_total_source_bytes,
            require_family,
        } => select_works(
            root.as_deref(),
            *target,
            out,
            vendored,
            *max_unknown_ratio,
            *max_total_source_bytes,
            require_family,
        ),
        CorpusTarget::VendorWorks {
            root,
            manifest,
            dest,
        } => vendor_works(root.as_deref(), manifest, dest),
        CorpusTarget::ClassifyUnknown { root, out, top } => {
            classify_unknown(root.as_deref(), out.as_deref(), *top)
        }
        CorpusTarget::FamilyCoverage { vendored, render } => family_coverage(vendored, render),
        CorpusTarget::FamilyProbe { text, file } => family_probe(text.as_deref(), file.as_deref()),
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

/// Stat an existing archive — print header summary, per-band
/// breakdown (count, mean / median / max bytes per entry,
/// compression ratio for zstd archives), top-K largest entries by
/// decoded length, and an mtime distribution histogram.
fn stat(path: &Path, top: usize) -> Result<(), String> {
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
        "  payload sum  : {:.2} MB decoded / {:.2} MB on-disk",
        total_decoded as f64 / 1_048_576.0,
        total_payload as f64 / 1_048_576.0
    );
    if total_payload > 0 && total_decoded > total_payload {
        let ratio = total_decoded as f64 / total_payload as f64;
        println!("  zstd ratio   : {ratio:.2}× decoded ÷ on-disk (overall)");
    }

    // Per-band breakdown — bucket by decoded_len matching
    // `aozora-bench::SizeBand` thresholds. Index buckets are kept
    // separate so we can compute median per band.
    println!();
    println!("  per-band breakdown (by decoded byte length):");
    print_band_header(arc.is_zstd());
    let mut bucketed: [Vec<&EntryMeta>; 4] = Default::default();
    for entry in arc.entries() {
        let slot = band_slot(entry.decoded_len);
        bucketed[slot].push(entry);
    }
    for (slot, label) in [
        (0usize, "<50KB"),
        (1, "50KB-500KB"),
        (2, "500KB-2MB"),
        (3, ">2MB"),
    ] {
        print_band_row(label, &bucketed[slot], arc.is_zstd());
    }

    // mtime distribution — by year. Useful sanity check that the
    // archive carries source mtimes (for incremental diff) and
    // gives a quick "what era of corpus is this" answer.
    println!();
    println!("  mtime distribution (by source-file year):");
    let mut by_year: HashMap<i32, usize> = HashMap::new();
    for entry in arc.entries() {
        if entry.source_mtime_ns <= 0 {
            continue;
        }
        let secs = (entry.source_mtime_ns / 1_000_000_000).clamp(0, i64::MAX / 2) as u64;
        let st = UNIX_EPOCH + Duration::from_secs(secs);
        let year = year_of(st);
        *by_year.entry(year).or_insert(0) += 1;
    }
    let mut years: Vec<_> = by_year.into_iter().collect();
    years.sort_by_key(|&(y, _)| y);
    for (year, count) in years {
        println!("    {year:>4}  {count:>5}  {}", bar(count, arc.len(), 40));
    }

    // Top-K largest entries by decoded bytes — useful for spotting
    // pathological docs at a glance.
    if top > 0 {
        println!();
        println!("  top {top} largest entries (by decoded bytes):");
        let mut sorted: Vec<&EntryMeta> = arc.entries().iter().collect();
        sorted.sort_by_key(|e| Reverse(e.decoded_len));
        for entry in sorted.iter().take(top) {
            let on_disk_kb = entry.payload_len as f64 / 1024.0;
            let decoded_kb = entry.decoded_len as f64 / 1024.0;
            let ratio = if entry.payload_len > 0 {
                decoded_kb / on_disk_kb
            } else {
                1.0
            };
            println!(
                "    {decoded_kb:>10.1} KB decoded / {on_disk_kb:>10.1} KB on-disk ({ratio:>4.1}×)  {}",
                entry.label
            );
        }
    }

    Ok(())
}

const SMALL_MAX: u32 = 50 * 1024;
const MEDIUM_MAX: u32 = 500 * 1024;
const LARGE_MAX: u32 = 2 * 1024 * 1024;

fn band_slot(decoded_len: u32) -> usize {
    if decoded_len < SMALL_MAX {
        0
    } else if decoded_len < MEDIUM_MAX {
        1
    } else if decoded_len < LARGE_MAX {
        2
    } else {
        3
    }
}

fn print_band_header(zstd: bool) {
    if zstd {
        println!(
            "    {:<11}  {:>6}  {:>10}  {:>10}  {:>10}  {:>10}  {:>10}  {:>7}",
            "band", "docs", "tot dec MB", "tot on MB", "mean dec", "median dec", "max dec", "ratio"
        );
    } else {
        println!(
            "    {:<11}  {:>6}  {:>10}  {:>10}  {:>10}  {:>10}",
            "band", "docs", "tot MB", "mean", "median", "max"
        );
    }
}

fn print_band_row(label: &str, entries: &[&EntryMeta], zstd: bool) {
    if entries.is_empty() {
        if zstd {
            println!(
                "    {label:<11}  {:>6}  {:>10}  {:>10}  {:>10}  {:>10}  {:>10}  {:>7}",
                0, "—", "—", "—", "—", "—", "—"
            );
        } else {
            println!(
                "    {label:<11}  {:>6}  {:>10}  {:>10}  {:>10}  {:>10}",
                0, "—", "—", "—", "—"
            );
        }
        return;
    }
    let count = entries.len();
    let total_dec: u64 = entries.iter().map(|e| u64::from(e.decoded_len)).sum();
    let total_on: u64 = entries.iter().map(|e| u64::from(e.payload_len)).sum();
    let mean_dec = total_dec as f64 / count as f64;
    let max_dec = entries.iter().map(|e| e.decoded_len).max().unwrap_or(0);
    let mut sorted: Vec<u32> = entries.iter().map(|e| e.decoded_len).collect();
    sorted.sort_unstable();
    let median_dec = sorted[count / 2];

    let mb = |b: u64| b as f64 / 1_048_576.0;
    let kb = |b: f64| b / 1024.0;

    if zstd {
        let ratio = if total_on > 0 {
            total_dec as f64 / total_on as f64
        } else {
            1.0
        };
        println!(
            "    {label:<11}  {count:>6}  {:>10.2}  {:>10.2}  {:>9.1}K  {:>9.1}K  {:>9.1}K  {ratio:>6.2}×",
            mb(total_dec),
            mb(total_on),
            kb(mean_dec),
            kb(f64::from(median_dec)),
            kb(f64::from(max_dec)),
        );
    } else {
        println!(
            "    {label:<11}  {count:>6}  {:>10.2}  {:>9.1}K  {:>9.1}K  {:>9.1}K",
            mb(total_dec),
            kb(mean_dec),
            kb(f64::from(median_dec)),
            kb(f64::from(max_dec)),
        );
    }
}

/// Plain "year of UNIX epoch second" — works for the 1970-2099 range
/// (the only range Aozora corpus mtimes will ever land in). Avoids
/// pulling chrono just for this readout.
fn year_of(t: SystemTime) -> i32 {
    let secs = t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let days = secs / 86_400;
    // Compute year via the same formula civil_from_days uses;
    // accuracy ±1 day at year boundaries, fine for histogram bucketing.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    (yoe as i64 + era * 400) as i32
}

fn bar(count: usize, total: usize, width: usize) -> String {
    if total == 0 {
        return String::new();
    }
    let filled = (count * width + total / 2) / total;
    "█".repeat(filled.min(width))
}

/// Evict a path (file or directory tree) from the kernel page cache
/// via `posix_fadvise(POSIX_FADV_DONTNEED)`. Safe-API rustix wrapper;
/// works without sudo for files the caller can open.
///
/// Linux-only: `posix_fadvise(POSIX_FADV_DONTNEED)` exists only on Linux
/// (`rustix::fs::fadvise` is configured out on Windows and macOS has no
/// `Advice::DontNeed`), so the non-Linux build carries the stub below,
/// which fails loudly rather than silently no-op'ing.
#[cfg(target_os = "linux")]
fn uncache(path: &Path) -> Result<(), String> {
    let total_start = Instant::now();
    let metadata =
        fs::symlink_metadata(path).map_err(|e| format!("stat {}: {e}", path.display()))?;
    let (count, bytes) = if metadata.is_dir() {
        let mut count = 0usize;
        let mut bytes = 0u64;
        for entry in walkdir::WalkDir::new(path) {
            let entry = entry.map_err(|e| format!("walk: {e}"))?;
            let ft = entry.file_type();
            if ft.is_dir() || ft.is_symlink() {
                continue;
            }
            match uncache_file(entry.path()) {
                Ok(n) => {
                    count += 1;
                    bytes += n;
                }
                Err(e) => {
                    eprintln!(
                        "  warning: {} could not be uncached: {e}",
                        entry.path().display()
                    );
                }
            }
        }
        (count, bytes)
    } else {
        let n = uncache_file(path)?;
        (1, n)
    };
    eprintln!(
        "xtask corpus uncache: evicted {count} file(s), {:.2} MB total in {:.2} s",
        bytes as f64 / 1_048_576.0,
        total_start.elapsed().as_secs_f64()
    );
    Ok(())
}

/// Non-Linux stub for `corpus uncache` (see the Linux definition above):
/// `posix_fadvise(POSIX_FADV_DONTNEED)` has no portable counterpart, so
/// the command is unsupported off Linux and says so rather than lying.
#[cfg(not(target_os = "linux"))]
fn uncache(_path: &Path) -> Result<(), String> {
    Err("xtask corpus uncache evicts the page cache via \
         posix_fadvise(POSIX_FADV_DONTNEED), which is Linux-only; \
         it is unsupported on this platform"
        .to_string())
}

#[cfg(target_os = "linux")]
fn uncache_file(path: &Path) -> Result<u64, String> {
    use rustix::fs::{Advice, fadvise};
    let file = fs::File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let len = file.metadata().map_or(0, |m| m.len());
    // `len = None` means "advise to end of file"; for our case (whole-
    // file eviction) that's exactly what we want, regardless of size.
    fadvise(&file, 0, None, Advice::DontNeed)
        .map_err(|e| format!("fadvise {}: {e}", path.display()))?;
    Ok(len)
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
    // re-encode the payload (normalise to UTF-8 here for utf8
    // archives, auto-detecting Shift_JIS vs already-UTF-8 source) but
    // keep `source_blake3` pinned to the raw source bytes so the next
    // incremental pack can match identity.
    let payload_bytes = if utf8 {
        decode_auto(&bytes)
            .map(|text| text.into_owned().into_bytes())
            .unwrap_or(bytes)
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

// ===========================================================================
// corpus audit — empirical ground truth for spec-conformance work
// ===========================================================================

/// Directive classes in report order.
const ANN_KIND_LABELS: &[&str] = &[
    "nonCanonical",
    "editorial",
    "asIs",
    "textualNote",
    "warichuOpen",
    "warichuClose",
    "empty",
    "editorNote",
    "rubyAttached",
    "rubyRetarget",
    "rubyPairOpen",
    "rubyPairClose",
    "marginNotePairOpen",
    "marginNotePairClose",
];

/// 外字 mencode address-form buckets, in the fixed order used by
/// [`gaiji_bucket`] / [`FileStat::gaiji_forms`].
const GAIJI_FORM_LABELS: &[&str] = &[
    "jisLevel",  // 第N水準… (named JIS level)
    "jisTriple", // men-ku-ten N-N-N
    "unicode",   // U+XXXX
    "pageLine",  //底本ページ-行 N-N
    "named",     // free-form description / other
    "absent",    // no mencode at all
];

/// Per-file audit accumulator. Owned data only — it must cross the
/// rayon worker boundary, so it holds no borrows into the per-file
/// parse output.
#[derive(Default)]
struct FileStat {
    label: String,
    decode_error: bool,
    panicked: bool,
    /// Indexed parallel to [`NodeKind::ALL`].
    node_kinds: [u64; NodeKind::ALL.len()],
    /// Indexed parallel to [`ANN_KIND_LABELS`].
    annotation_kinds: [u64; ANN_KIND_LABELS.len()],
    gaiji_total: u64,
    gaiji_unresolved: u64,
    /// Indexed parallel to [`GAIJI_FORM_LABELS`].
    gaiji_forms: [u64; GAIJI_FORM_LABELS.len()],
    /// One `(raw body, 1-based line)` per Unknown annotation occurrence.
    unknown: Vec<(String, u32)>,
    /// Diagnostic codes emitted for this file.
    diags: Vec<&'static str>,
}

#[derive(Serialize, Deserialize)]
struct Kv {
    key: String,
    count: u64,
}

#[derive(Serialize)]
struct UnknownRow {
    body: String,
    count: u64,
    /// First-seen (lexicographically smallest, for determinism)
    /// `corpus-relative-path:line`.
    example: String,
    /// Normalized family this body belongs to.
    shape: String,
}

#[derive(Serialize)]
struct ShapeRow {
    shape: String,
    count: u64,
    distinct: usize,
}

#[derive(Serialize)]
struct GaijiSummary {
    total: u64,
    unresolved: u64,
    forms: Vec<Kv>,
}

#[derive(Serialize)]
struct AuditReport {
    corpus_root: String,
    files_total: usize,
    files_analyzed: usize,
    decode_errors: usize,
    walk_errors: usize,
    panic_count: usize,
    panics: Vec<String>,
    elapsed_secs: f64,
    node_kinds: Vec<Kv>,
    annotation_kinds: Vec<Kv>,
    gaiji: GaijiSummary,
    diagnostics: Vec<Kv>,
    unknown_total: u64,
    unknown_distinct: usize,
    unknown_shapes: Vec<ShapeRow>,
    unknown_bodies: Vec<UnknownRow>,
}

/// Walk the corpus and build the [`AuditReport`] without emitting any
/// human/JSON output — the shared core behind both `corpus audit` and the
/// strict zero-residue `corpus audit-gate`.
fn run_audit(root: Option<&Path>, limit: Option<usize>) -> Result<AuditReport, String> {
    let corpus = resolve_corpus(root)?;
    let root_display = corpus.root().display().to_string();
    eprintln!("xtask corpus audit: walking {root_display} …");
    let start = Instant::now();

    // Per-document parse panics are recorded as a datum (see
    // `audit_one`); suppress the default backtrace printer so a
    // pathological doc does not flood stderr. Restored after the run.
    let prev_hook = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));

    let results: Vec<Result<FileStat, aozora_corpus::CorpusError>> = limit.map_or_else(
        || par_load_decoded(&corpus, audit_one),
        |n| {
            corpus
                .walk_paths()
                .take(n)
                .map(|pr| pr.and_then(|p| corpus.read_path(&p)).map(audit_one))
                .collect()
        },
    );

    panic::set_hook(prev_hook);

    Ok(merge(results, root_display, start.elapsed().as_secs_f64()))
}

fn audit(
    root: Option<&Path>,
    out: Option<&Path>,
    top: usize,
    limit: Option<usize>,
) -> Result<(), String> {
    let report = run_audit(root, limit)?;
    print_human_summary(&report, top);

    let json = serde_json::to_string_pretty(&report).map_err(|e| format!("serialize: {e}"))?;
    match out {
        Some(path) => {
            fs::write(path, json).map_err(|e| format!("write {}: {e}", path.display()))?;
            eprintln!("xtask corpus audit: JSON report → {}", path.display());
        }
        None => println!("{json}"),
    }
    Ok(())
}

fn audit_gate(root: Option<&Path>) -> Result<(), String> {
    let report = run_audit(root, None)?;
    let mut problems = Vec::new();
    if report.files_analyzed == 0 {
        problems.push("no corpus documents were analyzed".to_owned());
    }
    if report.decode_errors != 0 {
        problems.push(format!(
            "{} document(s) failed decoding",
            report.decode_errors
        ));
    }
    if report.walk_errors != 0 {
        problems.push(format!(
            "{} corpus path(s) failed to read",
            report.walk_errors
        ));
    }
    if report.panic_count != 0 {
        problems.push(format!("{} document(s) panicked", report.panic_count));
    }
    if report.unknown_total != 0 {
        problems.push(format!(
            "{} annotation(s) reached the unknown fallback",
            report.unknown_total
        ));
    }
    let internal: u64 = report
        .diagnostics
        .iter()
        .filter(|row| {
            aozora::InternalCheckCode::ALL
                .iter()
                .any(|code| code.as_code() == row.key)
        })
        .map(|row| row.count)
        .sum();
    if internal != 0 {
        problems.push(format!("{internal} internal diagnostic(s) fired"));
    }
    if problems.is_empty() {
        eprintln!(
            "audit-gate: PASS — {} documents, zero decode/read/panic/internal/unknown failures",
            report.files_analyzed
        );
        Ok(())
    } else {
        Err(format!(
            "strict corpus audit failed:\n  {}",
            problems.join("\n  ")
        ))
    }
}

/// Outcome of checking one document's verbatim-provenance invariant.
enum VerbatimOutcome {
    /// `to_source_verbatim()` equalled the decoded original source.
    Match,
    /// Source decoded as neither UTF-8 nor Shift_JIS.
    DecodeSkipped,
    /// The invariant broke (or the parse panicked); carries the label.
    Mismatch(String),
}

/// Verbatim-provenance gate for every corpus document.
fn verbatim_gate(root: Option<&Path>) -> Result<(), String> {
    let corpus = resolve_corpus(root)?;
    let root_display = corpus.root().display().to_string();
    eprintln!("xtask corpus verbatim: walking {root_display} …");
    let start = Instant::now();

    // Per-document parse panics are folded into `Mismatch` (see
    // `verbatim_one`); silence the default backtrace printer so a
    // pathological doc does not flood stderr. Restored after the run.
    let prev_hook = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));

    let results: Vec<Result<VerbatimOutcome, aozora_corpus::CorpusError>> =
        par_load_decoded(&corpus, verbatim_one);

    panic::set_hook(prev_hook);

    let mut checked = 0usize;
    let mut decode_skipped = 0usize;
    let mut walk_errors = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for r in results {
        match r {
            Ok(VerbatimOutcome::Match) => checked += 1,
            Ok(VerbatimOutcome::DecodeSkipped) => decode_skipped += 1,
            Ok(VerbatimOutcome::Mismatch(label)) => {
                checked += 1;
                failures.push(label);
            }
            Err(_) => walk_errors += 1,
        }
    }
    let elapsed = start.elapsed().as_secs_f64();

    if checked == 0 {
        failures.push("no corpus documents were checked".to_owned());
    }
    if decode_skipped != 0 {
        failures.push(format!("{decode_skipped} document(s) failed decoding"));
    }
    if walk_errors != 0 {
        failures.push(format!("{walk_errors} corpus path(s) failed to read"));
    }
    if !failures.is_empty() {
        failures.sort();
        let list = failures
            .iter()
            .take(10)
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n  ");
        let more = failures.len().saturating_sub(10);
        let tail = if more > 0 {
            format!("\n  … and {more} more")
        } else {
            String::new()
        };
        return Err(format!(
            "strict verbatim gate failed with {} failure(s) across {checked} document(s):\n  \
             {list}{tail}",
            failures.len()
        ));
    }

    eprintln!(
        "xtask corpus verbatim: PASS — {checked} docs, all to_source_verbatim() == original \
         ({elapsed:.1}s)"
    );
    Ok(())
}

/// Check one document's verbatim invariant.
fn verbatim_one(item: CorpusItem) -> VerbatimOutcome {
    let label = item.label;
    let source: Arc<str> = match decode_auto(&item.bytes) {
        Ok(text) => Arc::from(text.into_owned()),
        Err(_) => return VerbatimOutcome::DecodeSkipped,
    };
    let Ok(got) = panic::catch_unwind(AssertUnwindSafe(|| {
        aozora::parse(Arc::clone(&source))
            .expect("source fits parser span limit")
            .snapshot()
            .to_source_verbatim()
    })) else {
        return VerbatimOutcome::Mismatch(label);
    };
    if got == source.as_ref() {
        VerbatimOutcome::Match
    } else {
        VerbatimOutcome::Mismatch(label)
    }
}

/// A notation control marker that leaked into rendered visible text.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LeakCat {
    /// A non-empty ruby delimiter `《` (with `》`) survived — a ruby that
    /// never attached to its base and replayed as literal text.
    Ruby,
    /// A fullwidth ruby-base marker `｜` survived — the renderer should
    /// always consume it.
    Bar,
    /// An annotation open `［＃` survived — a directive (incl. gaiji
    /// `※［＃…］`) that never resolved.
    Directive,
}

/// One leaked marker plus a small visible-text context window.
struct LeakHit {
    cat: LeakCat,
    snippet: String,
}

/// Per-document render-audit outcome.
enum DocRenderOutcome {
    /// Rendered, no leaked markers.
    Clean,
    /// Neither UTF-8 nor Shift_JIS — skipped (mirrors `corpus_sweep`).
    DecodeSkipped,
    /// `to_html()` panicked; carries the label.
    Panicked(String),
    /// Rendered and leaked ≥1 marker.
    Leaked { label: String, hits: Vec<LeakHit> },
}

/// Aggregated leak stats for one marker category.
#[derive(Default)]
struct CatAgg {
    files: usize,
    occurrences: usize,
    samples: Vec<(String, String)>,
}

impl CatAgg {
    /// Fold one document's hits of category `cat` into the aggregate.
    fn record(&mut self, cat: LeakCat, label: &str, hits: &[LeakHit], top: usize) {
        let n = hits.iter().filter(|h| h.cat == cat).count();
        if n == 0 {
            return;
        }
        self.files += 1;
        self.occurrences += n;
        if self.samples.len() < top
            && let Some(hit) = hits.iter().find(|h| h.cat == cat)
        {
            self.samples.push((label.to_owned(), hit.snippet.clone()));
        }
    }
}

/// Render one document and scan its visible text for leaked markers.
fn render_audit_one(item: CorpusItem) -> DocRenderOutcome {
    let label = item.label;
    let decoded = match decode_auto(&item.bytes) {
        Ok(t) => t.into_owned(),
        Err(_) => return DocRenderOutcome::DecodeSkipped,
    };
    // Render only the literary body — the standard header legend
    // documents the notation glyphs verbatim and would swamp the signal.
    let text = aozora_body(&decoded);
    let Ok((html, literal_markup)) = panic::catch_unwind(AssertUnwindSafe(|| {
        let document = aozora::parse(text).expect("source fits parser span limit");
        let snapshot = document.snapshot();
        let literal_markup = snapshot
            .literal_markup()
            .iter()
            .map(|view| view.kind())
            .collect::<Vec<_>>();
        (snapshot.to_html(), literal_markup)
    })) else {
        return DocRenderOutcome::Panicked(label);
    };
    let hits = unaccounted_leak_markers(&html, &literal_markup);
    if hits.is_empty() {
        DocRenderOutcome::Clean
    } else {
        DocRenderOutcome::Leaked { label, hits }
    }
}

// ── Render-correctness invariants (I-A tag balance, I-C class membership) ──

/// A structural render defect found on one document's HTML.
#[derive(Clone)]
struct CorrHit {
    cat: CorrCat,
    snippet: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CorrCat {
    /// I-A: tags do not balance (mismatched close or unclosed at EOF).
    Unbalanced,
    /// I-B: a `<ruby>` has an empty base (reading annotates nothing).
    BadRuby,
    /// I-C: an emitted `aozora-*` class is not in `AOZORA_CLASSES`.
    UndeclaredClass,
}

/// Per-document render-correctness outcome (mirrors [`DocRenderOutcome`]).
enum DocCorrOutcome {
    Clean,
    DecodeSkipped,
    Panicked,
    Defective { label: String, hits: Vec<CorrHit> },
}

/// Collapse a trailing `-<digits>` run to its stem (matches the renderer's
/// `classes::collect_classes`), so `aozora-indent-3` checks as `aozora-indent`.
fn class_stem(c: &str) -> &str {
    match c.rfind('-') {
        Some(i) if i + 1 < c.len() && c[i + 1..].bytes().all(|b| b.is_ascii_digit()) => &c[..i],
        _ => c,
    }
}

/// The tag name of `<name …>` / `</name>` — bytes after `<`(`/`) up to the
/// first delimiter.
fn tag_name(tag: &str) -> &str {
    let s = tag.trim_start_matches('<').trim_start_matches('/');
    let end = s
        .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
        .unwrap_or(s.len());
    &s[..end]
}

/// The `aozora-*` class tokens declared in one opening tag's `class="…"`.
fn classes_of(tag: &str) -> Vec<&str> {
    let Some(p) = tag.find("class=\"") else {
        return Vec::new();
    };
    let rest = &tag[p + "class=\"".len()..];
    let end = rest.find('"').unwrap_or(rest.len());
    rest[..end]
        .split_whitespace()
        .filter(|c| c.starts_with("aozora-"))
        .collect()
}

/// Scan rendered `html` for I-A (tag balance) and I-C (undeclared class) in one
/// linear pass. Raw `<` only ever starts a tag (the renderer entity-escapes all
/// text and attribute values), so tag boundaries are unambiguous. Reports at
/// most one imbalance per document (tracking stops after the first anomaly).
fn scan_correctness(html: &str) -> Vec<CorrHit> {
    let mut hits = Vec::new();
    let mut stack: Vec<&str> = Vec::new();
    let mut broken = false;
    // I-B: `Some(seen_content)` while scanning a `<ruby>` base (before its <rt>).
    let mut ruby_base: Option<bool> = None;
    let bytes = html.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    while i < n {
        if bytes[i] != b'<' {
            if ruby_base == Some(false) && !bytes[i].is_ascii_whitespace() {
                ruby_base = Some(true); // non-whitespace text = non-empty base
            }
            i += 1;
            continue;
        }
        let end = html[i..].find('>').map_or(n - 1, |o| i + o);
        let tag = &html[i..=end];
        i = end + 1;
        if tag.starts_with("</") {
            let name = tag_name(tag);
            if !broken && stack.pop() != Some(name) {
                hits.push(CorrHit {
                    cat: CorrCat::Unbalanced,
                    snippet: format!("stray/mismatched </{name}>"),
                });
                broken = true;
            }
            continue;
        }
        let name = tag_name(tag);
        // I-B: the ruby base spans `<ruby>` up to its first `<rt>`/`<rp>`; an
        // intervening non-whitespace text run or nested element makes it non-empty.
        match name {
            "ruby" => ruby_base = Some(false),
            "rt" | "rp" => {
                if ruby_base == Some(false) {
                    hits.push(CorrHit {
                        cat: CorrCat::BadRuby,
                        snippet: "<ruby> with empty base".to_owned(),
                    });
                }
                ruby_base = None;
            }
            _ if ruby_base == Some(false) => ruby_base = Some(true),
            _ => {}
        }
        let void = tag.ends_with("/>") || name == "img" || name == "br";
        if !void && !broken {
            stack.push(name);
        }
        for tok in classes_of(tag) {
            let stem = class_stem(tok);
            if !AOZORA_CLASSES.contains(&stem) {
                hits.push(CorrHit {
                    cat: CorrCat::UndeclaredClass,
                    snippet: tok.to_owned(),
                });
            }
        }
    }
    if !broken && !stack.is_empty() {
        hits.push(CorrHit {
            cat: CorrCat::Unbalanced,
            snippet: format!("unclosed at EOF: <{}>", stack.join("><")),
        });
    }
    hits
}

/// Render one document's literary body and scan for structural render defects.
fn render_correctness_one(item: CorpusItem) -> DocCorrOutcome {
    let label = item.label;
    let decoded = match decode_auto(&item.bytes) {
        Ok(t) => t.into_owned(),
        Err(_) => return DocCorrOutcome::DecodeSkipped,
    };
    let text = aozora_body(&decoded);
    let Ok(html) = panic::catch_unwind(AssertUnwindSafe(|| {
        aozora::parse(text)
            .expect("source fits parser span limit")
            .snapshot()
            .to_html()
    })) else {
        return DocCorrOutcome::Panicked;
    };
    let hits = scan_correctness(&html);
    if hits.is_empty() {
        DocCorrOutcome::Clean
    } else {
        DocCorrOutcome::Defective { label, hits }
    }
}

/// Fold one document's correctness hits of category `cat` into `agg`.
fn record_corr(agg: &mut CatAgg, cat: CorrCat, label: &str, hits: &[CorrHit], top: usize) {
    let n = hits.iter().filter(|h| h.cat == cat).count();
    if n == 0 {
        return;
    }
    agg.files += 1;
    agg.occurrences += n;
    if agg.samples.len() < top
        && let Some(hit) = hits.iter().find(|h| h.cat == cat)
    {
        agg.samples.push((label.to_owned(), hit.snippet.clone()));
    }
}

#[derive(Default)]
struct RenderCorrectnessCounts {
    unbalanced: MarkerStat,
    bad_ruby: MarkerStat,
    undeclared: MarkerStat,
}

fn tally_render_correctness(
    results: Vec<Result<DocCorrOutcome, aozora_corpus::CorpusError>>,
) -> (RenderCorrectnessCounts, usize, usize, usize, usize) {
    let mut unbalanced = CatAgg::default();
    let mut bad_ruby = CatAgg::default();
    let mut undeclared = CatAgg::default();
    let (mut scanned, mut decode_errors, mut panicked, mut walk_errors) = (0, 0, 0, 0);
    for r in results {
        match r {
            Ok(DocCorrOutcome::Clean) => scanned += 1,
            Ok(DocCorrOutcome::DecodeSkipped) => decode_errors += 1,
            Ok(DocCorrOutcome::Panicked) => {
                scanned += 1;
                panicked += 1;
            }
            Ok(DocCorrOutcome::Defective { label, hits }) => {
                scanned += 1;
                record_corr(&mut unbalanced, CorrCat::Unbalanced, &label, &hits, 0);
                record_corr(&mut bad_ruby, CorrCat::BadRuby, &label, &hits, 0);
                record_corr(&mut undeclared, CorrCat::UndeclaredClass, &label, &hits, 0);
            }
            Err(_) => walk_errors += 1,
        }
    }
    let stat = |a: &CatAgg| MarkerStat {
        files: a.files,
        occurrences: a.occurrences,
    };
    let current = RenderCorrectnessCounts {
        unbalanced: stat(&unbalanced),
        bad_ruby: stat(&bad_ruby),
        undeclared: stat(&undeclared),
    };
    (current, scanned, decode_errors, panicked, walk_errors)
}

fn render_correctness_gate(root: Option<&Path>) -> Result<(), String> {
    let corpus = resolve_corpus(root)?;
    eprintln!(
        "xtask corpus render-correctness-gate: walking {} …",
        corpus.root().display()
    );
    let start = Instant::now();
    let prev_hook = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));
    let results = par_load_decoded(&corpus, render_correctness_one);
    panic::set_hook(prev_hook);
    let (current, scanned, decode_errors, panicked, walk_errors) =
        tally_render_correctness(results);
    let elapsed = start.elapsed().as_secs_f64();
    eprintln!(
        "render-correctness-gate: scanned {scanned} docs in {elapsed:.1}s — \
         I-A {}f, I-B {}f, I-C {}f",
        current.unbalanced.files, current.bad_ruby.files, current.undeclared.files,
    );
    let mut problems = Vec::new();
    if scanned == 0 {
        problems.push("no corpus documents were rendered".to_owned());
    }
    if decode_errors != 0 {
        problems.push(format!("{decode_errors} document(s) failed decoding"));
    }
    if walk_errors != 0 {
        problems.push(format!("{walk_errors} corpus path(s) failed to read"));
    }
    if panicked != 0 {
        problems.push(format!("{panicked} document(s) panicked while rendering"));
    }
    for (name, stat) in [
        ("unbalanced tags", current.unbalanced),
        ("empty ruby bases", current.bad_ruby),
        ("undeclared classes", current.undeclared),
    ] {
        if stat.occurrences != 0 {
            problems.push(format!("{} {name}", stat.occurrences));
        }
    }
    if !problems.is_empty() {
        return Err(format!(
            "strict render correctness failed:\n  {}",
            problems.join("\n  ")
        ));
    }
    eprintln!("render-correctness-gate: PASS");
    Ok(())
}

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
struct MarkerStat {
    files: usize,
    occurrences: usize,
}

#[derive(Default)]
struct RenderLeakCounts {
    ruby: MarkerStat,
    bar: MarkerStat,
    directive: MarkerStat,
}

fn tally_render_leaks(
    results: Vec<Result<DocRenderOutcome, aozora_corpus::CorpusError>>,
) -> (RenderLeakCounts, usize, usize, Vec<String>, usize) {
    let mut ruby = CatAgg::default();
    let mut bar = CatAgg::default();
    let mut directive = CatAgg::default();
    let mut scanned = 0usize;
    let mut decode_errors = 0usize;
    let mut panicked: Vec<String> = Vec::new();
    let mut walk_errors = 0usize;
    for r in results {
        match r {
            Err(_) => walk_errors += 1,
            Ok(DocRenderOutcome::DecodeSkipped) => decode_errors += 1,
            Ok(DocRenderOutcome::Clean) => scanned += 1,
            Ok(DocRenderOutcome::Panicked(label)) => {
                scanned += 1;
                panicked.push(label);
            }
            Ok(DocRenderOutcome::Leaked { label, hits }) => {
                scanned += 1;
                ruby.record(LeakCat::Ruby, &label, &hits, 0);
                bar.record(LeakCat::Bar, &label, &hits, 0);
                directive.record(LeakCat::Directive, &label, &hits, 0);
            }
        }
    }
    let current = RenderLeakCounts {
        ruby: MarkerStat {
            files: ruby.files,
            occurrences: ruby.occurrences,
        },
        bar: MarkerStat {
            files: bar.files,
            occurrences: bar.occurrences,
        },
        directive: MarkerStat {
            files: directive.files,
            occurrences: directive.occurrences,
        },
    };
    (current, scanned, decode_errors, panicked, walk_errors)
}

fn render_leak_gate(root: Option<&Path>) -> Result<(), String> {
    let corpus = resolve_corpus(root)?;
    eprintln!(
        "xtask corpus render-leak-gate: walking {} …",
        corpus.root().display()
    );
    let start = Instant::now();

    let prev_hook = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));
    let results = par_load_decoded(&corpus, render_audit_one);
    panic::set_hook(prev_hook);

    let (current, scanned, decode_errors, mut panicked, walk_errors) = tally_render_leaks(results);
    let elapsed = start.elapsed().as_secs_f64();

    let mut problems = Vec::new();
    if scanned == 0 {
        problems.push("no corpus documents were rendered".to_owned());
    }
    if decode_errors != 0 {
        problems.push(format!("{decode_errors} document(s) failed decoding"));
    }
    if walk_errors != 0 {
        problems.push(format!("{walk_errors} corpus path(s) failed to read"));
    }
    if !panicked.is_empty() {
        panicked.sort();
        problems.push(format!(
            "{} document(s) panicked in to_html(): {}",
            panicked.len(),
            panicked
                .iter()
                .take(5)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for (name, stat) in [
        ("ruby delimiter", current.ruby),
        ("ruby base marker", current.bar),
        ("directive marker", current.directive),
    ] {
        if stat.occurrences != 0 {
            problems.push(format!(
                "{} unexplained {name} occurrence(s) across {} document(s)",
                stat.occurrences, stat.files
            ));
        }
    }
    if !problems.is_empty() {
        return Err(format!(
            "strict render-leak gate failed:\n  {}",
            problems.join("\n  ")
        ));
    }

    eprintln!(
        "render-leak-gate: PASS — {scanned} docs, zero unexplained visible notation markers \
         ({elapsed:.1}s)"
    );
    Ok(())
}

/// Scan rendered HTML for notation control markers surviving into the
/// visible text. Two legitimate sources of `《…》` are excluded
/// structurally: empty ruby `《》` (a `《》：ルビ` legend / `empty_ruby`
/// fixture), and the literal delimiters an `≪…≫` angle-quote emits inside
/// an `aozora-angle-quote` span (stripped, nesting-aware, by
/// [`strip_to_visible_text`]).
fn visible_leak_markers(html: &str) -> Vec<LeakHit> {
    let visible = strip_to_visible_text(html);
    let text = html_unescape_min(&visible);
    let chars: Vec<char> = text.chars().collect();
    let mut hits = Vec::new();
    for idx in 0..chars.len() {
        let cat = match chars[idx] {
            // Non-empty ruby-open: `《` not immediately closed by `》`.
            '《' if chars.get(idx + 1) != Some(&'》') => Some(LeakCat::Ruby),
            '｜' => Some(LeakCat::Bar),
            // Annotation open `［＃` (incl. gaiji `※［＃`).
            '［' if chars.get(idx + 1) == Some(&'＃') => Some(LeakCat::Directive),
            _ => None,
        };
        if let Some(cat) = cat {
            hits.push(LeakHit {
                cat,
                snippet: snippet_around(&chars, idx),
            });
        }
    }
    hits
}

fn unaccounted_leak_markers(
    html: &str,
    literal_markup: &[aozora::LiteralMarkupKind],
) -> Vec<LeakHit> {
    let mut literal_counts = [0usize; 3];
    for kind in literal_markup {
        let index = match kind {
            aozora::LiteralMarkupKind::RubyDelimiters => 0,
            aozora::LiteralMarkupKind::RubyBaseMarker => 1,
            aozora::LiteralMarkupKind::DirectiveMarker => 2,
            _ => continue,
        };
        literal_counts[index] += 1;
    }
    visible_leak_markers(html)
        .into_iter()
        .filter(|hit| {
            let index = match hit.cat {
                LeakCat::Ruby => 0,
                LeakCat::Bar => 1,
                LeakCat::Directive => 2,
            };
            if literal_counts[index] == 0 {
                true
            } else {
                literal_counts[index] -= 1;
                false
            }
        })
        .collect()
}

/// Collect the visible text of `html`: drop everything inside `<…>` tags,
/// and drop the entire content of spans that do not display —
/// `aozora-angle-quote` (whose `《…》` delimiters are legitimate output)
/// and any element carrying the `hidden` attribute (a resolved directive
/// preserves its raw `［＃…］` inside `<span class="aozora-directive"
/// hidden>`, which the reader never sees). Span nesting is tracked so a
/// gaiji/ruby span nested inside a suppressed span is suppressed with it.
fn strip_to_visible_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let n = html.len();
    let bytes = html.as_bytes();
    let mut i = 0;
    let mut span_depth: i32 = 0;
    // `Some(d)`: suppressing text until span depth returns to `d`.
    let mut suppress_target: Option<i32> = None;
    while i < n {
        if bytes[i] == b'<' {
            let end = html[i..].find('>').map_or(n - 1, |o| i + o);
            let tag = &html[i..=end];
            if tag.starts_with("</span") {
                span_depth -= 1;
                if let Some(t) = suppress_target
                    && span_depth <= t
                {
                    suppress_target = None;
                }
            } else if is_span_open(tag) {
                let suppressed = tag.contains("aozora-angle-quote")
                    || tag.contains(" data-codepoint=")
                    || tag.contains(" hidden");
                if suppress_target.is_none() && suppressed {
                    suppress_target = Some(span_depth);
                }
                span_depth += 1;
            }
            i = end + 1;
        } else {
            let ch = html[i..].chars().next().unwrap_or('\u{fffd}');
            if suppress_target.is_none() {
                out.push(ch);
            }
            i += ch.len_utf8();
        }
    }
    out
}

/// Strip the standard Aozora boilerplate so the audit sees only the
/// literary body: the header legend block (everything up to and including
/// the 2nd delimiter line — a run of ≥10 `-`), which literally documents
/// `《》 ｜ ［＃］` and would otherwise register as leaks, and the
/// bibliographic footer from the first `底本：` line onward.
fn aozora_body(src: &str) -> String {
    let lines: Vec<&str> = src.lines().collect();
    let is_delim = |l: &str| {
        let t = l.trim_end();
        t.len() >= 10 && t.chars().all(|c| c == '-')
    };
    let delims: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| is_delim(l))
        .map(|(i, _)| i)
        .collect();
    let start = if delims.len() >= 2 { delims[1] + 1 } else { 0 };
    let end = lines
        .iter()
        .enumerate()
        .skip(start)
        .find(|(_, l)| {
            let t = l.trim_start();
            t.starts_with("底本：") || t.starts_with("底本:")
        })
        .map_or(lines.len(), |(i, _)| i);
    lines[start..end].join("\n")
}

/// `<span>` or `<span …>` opening tag (not `</span>`, not `<sub>`/`<sup>`).
fn is_span_open(tag: &str) -> bool {
    let Some(rest) = tag.strip_prefix("<span") else {
        return false;
    };
    matches!(rest.as_bytes().first(), Some(b' ' | b'>' | b'/'))
}

/// Undo the five entities `aozora-render`'s escaper emits (`&amp;` last so
/// `&amp;lt;` → `&lt;`, not `<`).
fn html_unescape_min(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&amp;", "&")
}

/// A short visible-text window around a leak, for triage.
fn snippet_around(chars: &[char], idx: usize) -> String {
    let start = idx.saturating_sub(6);
    let end = (idx + 8).min(chars.len());
    chars[start..end].iter().collect()
}

fn resolve_corpus(root: Option<&Path>) -> Result<FilesystemCorpus, String> {
    let path: PathBuf = match root {
        Some(p) => p.to_path_buf(),
        None => std::env::var_os("AOZORA_CORPUS_ROOT")
            .map(PathBuf::from)
            .ok_or("pass --root or set $AOZORA_CORPUS_ROOT")?,
    };
    FilesystemCorpus::new(path).map_err(|e| format!("invalid corpus root: {e:?}"))
}

/// Decode and parse one document, accumulating its audit stats. The
/// parse and AST walk are wrapped in `catch_unwind` so a single
/// pathological document is recorded as `panicked: true` rather than
/// aborting the whole sweep.
fn audit_one(item: CorpusItem) -> FileStat {
    let label = item.label;
    let text = match decode_auto(&item.bytes) {
        Ok(t) => t.into_owned(),
        Err(_) => {
            return FileStat {
                label,
                decode_error: true,
                ..Default::default()
            };
        }
    };
    match panic::catch_unwind(AssertUnwindSafe(|| analyze(&text))) {
        Ok(mut stat) => {
            stat.label = label;
            stat
        }
        Err(_) => FileStat {
            label,
            panicked: true,
            ..Default::default()
        },
    }
}

/// Walk the owned AST and tally everything we report. Payload text is
/// resolved through the output's store; the returned `FileStat` owns its
/// strings.
fn analyze(text: &str) -> FileStat {
    let doc = aozora::parse(text).expect("source fits parser span limit");
    let snapshot = doc.snapshot();
    let mut s = FileStat::default();

    for node in snapshot.nodes() {
        if let Some(i) = NodeKind::ALL.iter().position(|kind| *kind == node.kind()) {
            s.node_kinds[i] += 1;
        }
    }

    for directive in snapshot.directives() {
        let bucket = match directive.kind() {
            DirectiveClass::NonCanonical => 0,
            DirectiveClass::Editorial => 1,
            DirectiveClass::Sic => 2,
            DirectiveClass::BaseTextVariant => 3,
            DirectiveClass::WarichuOpen => 4,
            DirectiveClass::WarichuClose => 5,
            DirectiveClass::Empty => 6,
            DirectiveClass::EditorNote => 7,
            DirectiveClass::RubyAttached => 8,
            DirectiveClass::RubyRetarget => 9,
            DirectiveClass::RubyPairOpen => 10,
            DirectiveClass::RubyPairClose => 11,
            DirectiveClass::MarginNotePairOpen => 12,
            DirectiveClass::MarginNotePairClose => 13,
            _ => continue,
        };
        s.annotation_kinds[bucket] += 1;
    }

    for gaiji in snapshot.gaiji_resolutions() {
        s.gaiji_total += 1;
        if gaiji.resolved().is_none() {
            s.gaiji_unresolved += 1;
        }
        s.gaiji_forms[gaiji_bucket(gaiji.mencode())] += 1;
    }

    for d in snapshot.diagnostics() {
        s.diags.push(d.code());
    }
    s
}

/// 1-based source line of a sanitized-source byte offset. Offsets are in
/// sanitized-source coordinates, which equal raw-source coordinates for
/// the typical document (no BOM, LF-only, no `〔…〕` accent spans); the
/// `example` pointer is approximate when sanitization shifted bytes.
#[cfg(test)]
fn line_of(text: &str, byte_off: u32) -> u32 {
    let off = (byte_off as usize).min(text.len());
    let newlines = text.as_bytes()[..off]
        .iter()
        .filter(|&&b| b == b'\n')
        .count();
    (newlines + 1) as u32
}

fn is_digit_char(c: char) -> bool {
    c.is_ascii_digit() || ('０'..='９').contains(&c)
}

/// Classify a 外字 mencode reference into one of [`GAIJI_FORM_LABELS`].
fn gaiji_bucket(mencode: Option<&str>) -> usize {
    let Some(s) = mencode else { return 5 }; // absent
    if s.contains("U+") {
        return 2; // unicode
    }
    if s.contains("水準") {
        return 0; // jisLevel
    }
    // Take the leading token before any separator, then inspect its
    // dash structure: N-N-N is a men-ku-ten triple, N-N is a page-line
    // reference, anything else is a free-form / named description.
    let head = s.split(['、', ',', ' ']).next().unwrap_or(s).trim();
    let parts: Vec<&str> = head.split('-').collect();
    let numeric = parts
        .iter()
        .all(|p| !p.is_empty() && p.chars().all(is_digit_char));
    match (parts.len(), numeric) {
        (3, true) => 1, // jisTriple
        (2, true) => 3, // pageLine
        _ => 4,         // named
    }
}

/// Collapse a directive body to its family shape: quoted operands
/// (`「…」`) become `「」` and digit runs become `N`, so
/// `「猫」は太字` and `「犬」は太字` both fold to `「」は太字`.
fn normalize_shape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut in_quote = false;
    let mut prev_digit = false;
    for c in raw.chars() {
        match c {
            '「' => {
                out.push('「');
                in_quote = true;
                prev_digit = false;
            }
            '」' => {
                out.push('」');
                in_quote = false;
                prev_digit = false;
            }
            _ if in_quote => {} // drop the quoted operand's content
            c if is_digit_char(c) => {
                if !prev_digit {
                    out.push('N');
                }
                prev_digit = true;
            }
            c => {
                out.push(c);
                prev_digit = false;
            }
        }
    }
    out
}

fn kv_sorted(rows: impl IntoIterator<Item = (String, u64)>) -> Vec<Kv> {
    let mut v: Vec<Kv> = rows
        .into_iter()
        .filter(|(_, c)| *c > 0)
        .map(|(key, count)| Kv { key, count })
        .collect();
    v.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.key.cmp(&b.key)));
    v
}

fn merge(
    results: Vec<Result<FileStat, aozora_corpus::CorpusError>>,
    corpus_root: String,
    elapsed_secs: f64,
) -> AuditReport {
    let mut node_kinds = [0u64; NodeKind::ALL.len()];
    let mut ann = [0u64; ANN_KIND_LABELS.len()];
    let mut gforms = [0u64; GAIJI_FORM_LABELS.len()];
    let mut gaiji_total = 0u64;
    let mut gaiji_unresolved = 0u64;
    let mut diag_map: HashMap<&'static str, u64> = HashMap::new();
    // body → (count, smallest-example).
    let mut unknown_map: HashMap<String, (u64, String)> = HashMap::new();

    let mut files_total = 0usize;
    let mut files_analyzed = 0usize;
    let mut decode_errors = 0usize;
    let mut walk_errors = 0usize;
    let mut panics: Vec<String> = Vec::new();

    for r in results {
        let Ok(s) = r else {
            walk_errors += 1;
            continue;
        };
        files_total += 1;
        if s.decode_error {
            decode_errors += 1;
            continue;
        }
        if s.panicked {
            panics.push(s.label);
            continue;
        }
        files_analyzed += 1;

        let FileStat {
            label,
            node_kinds: nk,
            annotation_kinds: ak,
            gaiji_total: gt,
            gaiji_unresolved: gu,
            gaiji_forms: gf,
            unknown,
            diags,
            ..
        } = s;

        for (acc, v) in node_kinds.iter_mut().zip(nk) {
            *acc += v;
        }
        for (acc, v) in ann.iter_mut().zip(ak) {
            *acc += v;
        }
        for (acc, v) in gforms.iter_mut().zip(gf) {
            *acc += v;
        }
        gaiji_total += gt;
        gaiji_unresolved += gu;
        for code in diags {
            *diag_map.entry(code).or_insert(0) += 1;
        }
        for (body, line) in unknown {
            let example = format!("{label}:{line}");
            let entry = unknown_map
                .entry(body)
                .or_insert_with(|| (0, example.clone()));
            entry.0 += 1;
            if example < entry.1 {
                entry.1 = example;
            }
        }
    }

    let unknown_total: u64 = unknown_map.values().map(|(c, _)| *c).sum();
    let unknown_distinct = unknown_map.len();

    // Fold the distinct bodies into family shapes.
    let mut shape_map: HashMap<String, (u64, usize)> = HashMap::new();
    for (body, (count, _)) in &unknown_map {
        let entry = shape_map.entry(normalize_shape(body)).or_insert((0, 0));
        entry.0 += *count;
        entry.1 += 1;
    }
    let mut unknown_shapes: Vec<ShapeRow> = shape_map
        .into_iter()
        .map(|(shape, (count, distinct))| ShapeRow {
            shape,
            count,
            distinct,
        })
        .collect();
    unknown_shapes.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.shape.cmp(&b.shape)));

    let mut unknown_bodies: Vec<UnknownRow> = unknown_map
        .into_iter()
        .map(|(body, (count, example))| {
            let shape = normalize_shape(&body);
            UnknownRow {
                body,
                count,
                example,
                shape,
            }
        })
        .collect();
    unknown_bodies.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.body.cmp(&b.body)));

    panics.sort();

    AuditReport {
        corpus_root,
        files_total,
        files_analyzed,
        decode_errors,
        walk_errors,
        panic_count: panics.len(),
        panics,
        elapsed_secs,
        node_kinds: kv_sorted(
            NodeKind::ALL
                .iter()
                .zip(node_kinds)
                .map(|(k, c)| (k.as_json_tag().to_owned(), c)),
        ),
        annotation_kinds: kv_sorted(
            ANN_KIND_LABELS
                .iter()
                .zip(ann)
                .map(|(l, c)| ((*l).to_owned(), c)),
        ),
        gaiji: GaijiSummary {
            total: gaiji_total,
            unresolved: gaiji_unresolved,
            forms: kv_sorted(
                GAIJI_FORM_LABELS
                    .iter()
                    .zip(gforms)
                    .map(|(l, c)| ((*l).to_owned(), c)),
            ),
        },
        diagnostics: kv_sorted(diag_map.into_iter().map(|(k, c)| (k.to_owned(), c))),
        unknown_total,
        unknown_distinct,
        unknown_shapes,
        unknown_bodies,
    }
}

fn print_human_summary(r: &AuditReport, top: usize) {
    eprintln!();
    eprintln!("=== corpus audit ===");
    eprintln!("corpus root      : {}", r.corpus_root);
    eprintln!(
        "files            : {} analyzed / {} read ({} decode-errors, {} walk-errors)",
        r.files_analyzed, r.files_total, r.decode_errors, r.walk_errors
    );
    eprintln!("parse panics     : {}", r.panic_count);
    eprintln!("elapsed          : {:.2}s", r.elapsed_secs);
    eprintln!();

    eprintln!("NodeKind frequency:");
    for kv in &r.node_kinds {
        eprintln!("  {:<16} {:>10}", kv.key, kv.count);
    }
    eprintln!();

    eprintln!("DirectiveKind breakdown:");
    for kv in &r.annotation_kinds {
        eprintln!("  {:<16} {:>10}", kv.key, kv.count);
    }
    eprintln!();

    eprintln!(
        "外字 (gaiji)      : {} total, {} unresolved",
        r.gaiji.total, r.gaiji.unresolved
    );
    for kv in &r.gaiji.forms {
        eprintln!("  {:<16} {:>10}", kv.key, kv.count);
    }
    eprintln!();

    if !r.diagnostics.is_empty() {
        eprintln!("diagnostics by code:");
        for kv in &r.diagnostics {
            eprintln!("  {:<28} {:>10}", kv.key, kv.count);
        }
        eprintln!();
    }

    eprintln!(
        "Unknown annotations: {} occurrences, {} distinct bodies, {} shape families",
        r.unknown_total,
        r.unknown_distinct,
        r.unknown_shapes.len()
    );
    eprintln!();
    eprintln!("Top {top} Unknown shape families (count / distinct):");
    for row in r.unknown_shapes.iter().take(top) {
        eprintln!(
            "  {:>8} {:>6}d  {}",
            row.count,
            row.distinct,
            truncate_for_display(&row.shape, 60)
        );
    }
    eprintln!();
    eprintln!("Top {top} Unknown bodies (count — first seen):");
    for row in r.unknown_bodies.iter().take(top) {
        eprintln!(
            "  {:>8}  {}  [{}]",
            row.count,
            truncate_for_display(&row.body, 56),
            row.example
        );
    }
    eprintln!();
    eprintln!("(full untruncated lists are in the JSON report)");
}

/// Truncate by `char` (never mid-codepoint) for terminal display.
fn truncate_for_display(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

// ================= WS-1: stratified golden-works selection (#414) =================
//
// The `fixtures/works/` golden set byte-compares the parser's own `to_html()`
// over whole real works, catching drift on the notation *combinations* real
// documents exhibit that single-construct fixtures miss. `select-works` grows
// that set reproducibly: it fingerprints every corpus work by the notation
// families it exercises, then runs a deterministic greedy weighted set-cover —
// rare families (e.g. `lineGothic`) weighted highest — over the works not already
// vendored, excluding Unknown-dominated, unclean, and >500 KiB works. The value
// of a new golden is the family *combination* it forces through `to_html()`, so
// coverage (not proportional sampling) is the objective.

/// Family universe derived from the three wire authorities.
const FAM_NODE: usize = NodeKind::ALL.len();
const FAM_ANN: usize = ANN_KIND_LABELS.len();
const FAM_GAIJI: usize = GAIJI_FORM_LABELS.len();
const FAM_TOTAL: usize = FAM_NODE + FAM_ANN + FAM_GAIJI;

fn family_name(id: usize) -> &'static str {
    if id < FAM_NODE {
        NodeKind::ALL[id].as_json_tag()
    } else if id < FAM_NODE + FAM_ANN {
        ANN_KIND_LABELS[id - FAM_NODE]
    } else {
        GAIJI_FORM_LABELS[id - FAM_NODE - FAM_ANN]
    }
}

/// Reverse of [`family_name`]: the family id a name denotes, or `None`.
fn family_id_by_name(name: &str) -> Option<usize> {
    (0..FAM_TOTAL).find(|&id| family_name(id) == name)
}

/// The family ids a document exercises (count > 0), ascending.
fn family_ids(stat: &FileStat) -> Vec<usize> {
    let mut ids = Vec::new();
    for (i, &c) in stat.node_kinds.iter().enumerate() {
        if c > 0 {
            ids.push(i);
        }
    }
    for i in 0..ANN_KIND_LABELS.len() {
        if stat.annotation_kinds[i] > 0 {
            ids.push(FAM_NODE + i);
        }
    }
    for (i, &c) in stat.gaiji_forms.iter().enumerate() {
        if c > 0 {
            ids.push(FAM_NODE + FAM_ANN + i);
        }
    }
    ids
}

/// The notation-hygiene catalogue bucket a raw `［＃…］` Unknown body falls into.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Bucket {
    /// Tier1 catalogue normalization resolves it — a zero-FP near-miss.
    Tier1,
    /// Tier1 declines but Tier2 catalogue normalization reduces it (render-only).
    Tier2,
    /// Neither catalogue matches — the discovery residue.
    Residue,
}

/// Strip the `［＃…］` frame exactly as the serializer's `emit_annotation` does
/// (the single authority), then classify the trimmed body.
fn classify_body(raw: &str) -> Bucket {
    let body = raw
        .strip_prefix("［＃")
        .and_then(|s| s.strip_suffix('］'))
        .unwrap_or(raw)
        .trim();
    match Catalogue::normalization(body) {
        Some(CatalogueMatch::Canonical) => Bucket::Tier1,
        // Degraded — and any future recognized tier, since CatalogueMatch is
        // #[non_exhaustive] — is a known but non-Core directive.
        Some(_) => Bucket::Tier2,
        None => Bucket::Residue,
    }
}

#[derive(Serialize)]
struct ResidueRow {
    shape: String,
    count: u64,
    /// The raw body of this shape's highest-count member — the concrete text a
    /// human vets (the shape elides `「…」` / digit operands).
    sample: String,
    /// First-seen `corpus-relative-path:line` of the sample.
    example: String,
}

#[derive(Serialize)]
struct ClassifyReport {
    files_analyzed: usize,
    unknown_total: u64,
    tier1_occurrences: u64,
    tier2_occurrences: u64,
    residue_occurrences: u64,
    /// `(tier1 + tier2) / unknown_total` — the share the catalogues already cover.
    resolved_ratio: f64,
    residue_distinct_shapes: usize,
    residue_shapes: Vec<ResidueRow>,
}

/// Bucket a set of raw Unknown bodies against the catalogues into a
/// [`ClassifyReport`] — the pure, corpus-free core of `classify-unknown`.
/// `bodies` is expected sorted by count descending, so each residue shape's
/// first-seen member is its highest-count sample.
fn classify_report(
    bodies: &[UnknownRow],
    files_analyzed: usize,
    unknown_total: u64,
) -> ClassifyReport {
    let (mut t1, mut t2, mut residue) = (0_u64, 0_u64, 0_u64);
    let mut residue_map: BTreeMap<String, (u64, String, String)> = BTreeMap::new();
    for row in bodies {
        match classify_body(&row.body) {
            Bucket::Tier1 => t1 += row.count,
            Bucket::Tier2 => t2 += row.count,
            Bucket::Residue => {
                residue += row.count;
                let e = residue_map
                    .entry(row.shape.clone())
                    .or_insert_with(|| (0, row.body.clone(), row.example.clone()));
                e.0 += row.count;
            }
        }
    }

    let mut residue_shapes: Vec<ResidueRow> = residue_map
        .into_iter()
        .map(|(shape, (count, sample, example))| ResidueRow {
            shape,
            count,
            sample,
            example,
        })
        .collect();
    residue_shapes.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.shape.cmp(&b.shape)));

    ClassifyReport {
        files_analyzed,
        unknown_total,
        tier1_occurrences: t1,
        tier2_occurrences: t2,
        residue_occurrences: residue,
        resolved_ratio: (t1 + t2) as f64 / unknown_total.max(1) as f64,
        residue_distinct_shapes: residue_shapes.len(),
        residue_shapes,
    }
}

/// `corpus classify-unknown`: bucket the Unknown residue against the catalogues.
fn classify_unknown(root: Option<&Path>, out: Option<&Path>, top: usize) -> Result<(), String> {
    let audit = run_audit(root, None)?;
    let report = classify_report(
        &audit.unknown_bodies,
        audit.files_analyzed,
        audit.unknown_total,
    );

    eprintln!(
        "xtask corpus classify-unknown: {} files, {} Unknown occurrences\n  \
         Tier1-covered: {}\n  Tier2-covered: {}\n  residue: {} \
         ({} distinct shapes)\n  resolved: {:.1}%",
        report.files_analyzed,
        report.unknown_total,
        report.tier1_occurrences,
        report.tier2_occurrences,
        report.residue_occurrences,
        report.residue_distinct_shapes,
        report.resolved_ratio * 100.0,
    );
    for r in report.residue_shapes.iter().take(top) {
        eprintln!(
            "  {:>6}  {}  (e.g. {} @ {})",
            r.count, r.shape, r.sample, r.example
        );
    }

    let json = serde_json::to_string_pretty(&report).map_err(|e| format!("serialize: {e}"))?;
    match out {
        Some(path) => {
            fs::write(path, json).map_err(|e| format!("write {}: {e}", path.display()))?;
            eprintln!(
                "xtask corpus classify-unknown: JSON report → {}",
                path.display()
            );
        }
        None => println!("{json}"),
    }
    Ok(())
}

/// The family ids in `[0, FAM_TOTAL)` not present in `covered`, as names — the
/// pure set-difference behind `family-coverage`.
fn missing_families(covered: &std::collections::HashSet<usize>) -> Vec<&'static str> {
    (0..FAM_TOTAL)
        .filter(|id| !covered.contains(id))
        .map(family_name)
        .collect()
}

/// Assert every notation family is exercised by committed fixtures.
fn family_coverage(vendored: &Path, render: &Path) -> Result<(), String> {
    if !vendored.is_dir() {
        return Err(format!("not a directory: {}", vendored.display()));
    }
    // Union coverage from the real golden works AND the crafted render fixtures.
    let (mut covered, _hw) = load_vendored(vendored);
    let (render_covered, _hr) = load_vendored(render);
    covered.extend(render_covered);
    let missing = missing_families(&covered);

    eprintln!(
        "xtask corpus family-coverage: {}/{FAM_TOTAL} families exercised (works ∪ render fixtures)",
        covered.len(),
    );

    if missing.is_empty() {
        eprintln!(
            "family-coverage: PASS — {} covered = {FAM_TOTAL}.",
            covered.len(),
        );
        Ok(())
    } else {
        Err(format!(
            "family-coverage regression: families no longer covered: {}. \
             Add a golden work (`select-works --require-family`) or a `fixtures/render/` \
             fixture.",
            missing.join(", ")
        ))
    }
}

/// `corpus family-probe`: the notation families a single source exercises,
/// via the exact [`analyze`] → [`family_ids`] walk `family-coverage` counts.
fn family_probe(text: Option<&str>, file: Option<&Path>) -> Result<(), String> {
    let src = match (text, file) {
        (Some(t), _) => t.to_owned(),
        (None, Some(f)) => {
            fs::read_to_string(f).map_err(|e| format!("read {}: {e}", f.display()))?
        }
        (None, None) => return Err("pass --text <SOURCE> or --file <PATH>".to_owned()),
    };
    let stat = panic::catch_unwind(AssertUnwindSafe(|| analyze(&src)))
        .map_err(|_| "parse panicked".to_owned())?;
    let names: Vec<&'static str> = family_ids(&stat)
        .iter()
        .map(|&id| family_name(id))
        .collect();
    eprintln!(
        "xtask corpus family-probe: {} families — {}",
        names.len(),
        if names.is_empty() {
            "(none)".to_owned()
        } else {
            names.join(", ")
        }
    );
    let report = serde_json::json!({ "families": names });
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|e| format!("serialize: {e}"))?
    );
    Ok(())
}

/// A deterministic ASCII slug from the aozora card id (the first digit run in
/// the filename), e.g. `作品/種田山頭火/其中日記（49258…）.txt` → `w49258`. Card
/// ids are unique per work, so slugs do not collide; a human may rename to a
/// mnemonic before vendoring, but this keeps the bulk selection reproducible
/// without romanising 80 kanji titles by hand.
fn slug_from_label(label: &str) -> String {
    let stem = label.rsplit('/').next().unwrap_or(label);
    let digits: String = stem
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    if digits.is_empty() {
        format!("w{}", &blake3::hash(label.as_bytes()).to_hex()[..10])
    } else {
        format!("w{digits}")
    }
}

/// The corpus filename's orthography tag, informational. (`mtime` is a vacuous
/// era proxy on a flat checkout, so we surface the 新字新仮名 / 旧字旧仮名 axis
/// the filename carries instead.)
fn orthography_of(label: &str) -> &'static str {
    for tag in ["新字新仮名", "旧字旧仮名", "新字旧仮名", "旧字新仮名"] {
        if label.contains(tag) {
            return tag;
        }
    }
    ""
}

/// A candidate work's selection fingerprint.
struct WorkProfile {
    label: String,
    len: usize,
    band: usize,
    orthography: &'static str,
    families: Vec<usize>,
    unknown_ratio: f64,
    content_hash: blake3::Hash,
    clean: bool,
    decodable: bool,
}

/// Decode, lex, and render one work into a fingerprint. Both the AST walk
/// (families) and the render (cleanliness) are `catch_unwind`-guarded.
fn profile_one(item: CorpusItem) -> WorkProfile {
    let label = item.label;
    let orthography = orthography_of(&label);
    let text = match decode_auto(&item.bytes) {
        Ok(t) => t.into_owned(),
        Err(_) => {
            return WorkProfile {
                label,
                len: 0,
                band: 0,
                orthography,
                families: Vec::new(),
                unknown_ratio: 1.0,
                content_hash: blake3::hash(b""),
                clean: false,
                decodable: false,
            };
        }
    };
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let content_hash = blake3::hash(normalized.as_bytes());
    let len = normalized.len();
    let band = band_slot(u32::try_from(len).unwrap_or(u32::MAX));

    let stat = panic::catch_unwind(AssertUnwindSafe(|| analyze(&text))).ok();
    let (families, unknown_ratio) = stat.as_ref().map_or_else(
        || (Vec::new(), 1.0),
        |s| {
            let ann_total: u64 = s.annotation_kinds.iter().sum();
            let ratio = if ann_total == 0 {
                0.0
            } else {
                ((s.annotation_kinds[0] + s.annotation_kinds[1]) as f64) / (ann_total as f64)
            };
            (family_ids(s), ratio)
        },
    );
    let clean = stat.is_some()
        && panic::catch_unwind(AssertUnwindSafe(|| {
            let html = aozora::parse(text.clone())
                .expect("source fits parser span limit")
                .snapshot()
                .to_html();
            scan_correctness(&html).is_empty() && visible_leak_markers(&html).is_empty()
        }))
        .unwrap_or(false);

    WorkProfile {
        label,
        len,
        band,
        orthography,
        families,
        unknown_ratio,
        content_hash,
        clean,
        decodable: stat.is_some(),
    }
}

/// Seed family coverage and the dedup hash-set from the already-vendored works.
fn load_vendored(
    dir: &Path,
) -> (
    std::collections::HashSet<usize>,
    std::collections::HashSet<[u8; 32]>,
) {
    let mut covered = std::collections::HashSet::new();
    let mut hashes = std::collections::HashSet::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return (covered, hashes);
    };
    for entry in entries.flatten() {
        let src = entry.path().join("source.txt");
        let Ok(text) = fs::read_to_string(&src) else {
            continue;
        };
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        hashes.insert(*blake3::hash(normalized.as_bytes()).as_bytes());
        if let Ok(stat) = panic::catch_unwind(AssertUnwindSafe(|| analyze(&text))) {
            for f in family_ids(&stat) {
                covered.insert(f);
            }
        }
    }
    (covered, hashes)
}

#[derive(Serialize)]
struct WorksManifest {
    note: String,
    work: Vec<WorkRow>,
}

#[derive(Serialize)]
struct WorkRow {
    path: String,
    /// Blank on generation — a human fills a mnemonic romanisation (kanji
    /// author/title is not deterministically romanisable).
    slug: String,
    size: usize,
    band: &'static str,
    orthography: &'static str,
    families: Vec<&'static str>,
    /// The families this work was the first (in selection order) to contribute.
    new_families: Vec<&'static str>,
}

/// Deterministic greedy weighted set-cover selection of golden-work candidates.
fn select_works(
    root: Option<&Path>,
    target: usize,
    out: &Path,
    vendored: &Path,
    max_unknown_ratio: f64,
    max_total_source_bytes: usize,
    require_family: &[String],
) -> Result<(), String> {
    // Resolve --require-family names up front — a typo fails before the walk.
    let required: Vec<(String, usize)> = require_family
        .iter()
        .map(|name| {
            family_id_by_name(name)
                .map(|id| (name.clone(), id))
                .ok_or_else(|| {
                    format!("unknown family '{name}' (run `family-coverage` for the 43 names)")
                })
        })
        .collect::<Result<_, _>>()?;

    let corpus = resolve_corpus(root)?;
    eprintln!(
        "xtask corpus select-works: walking {} …",
        corpus.root().display()
    );
    let start = Instant::now();

    let prev_hook = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));
    let mut profiles: Vec<WorkProfile> = par_load_decoded(&corpus, profile_one)
        .into_iter()
        .filter_map(Result::ok)
        .collect();
    panic::set_hook(prev_hook);

    // Determinism: `par_load_decoded` is unordered — sort by label first.
    profiles.sort_by(|a, b| a.label.cmp(&b.label));

    let (seed_covered, vendored_hashes) = load_vendored(vendored);

    // Rarity weights from global family frequency over decodable works.
    let mut global = [0u64; FAM_TOTAL];
    for p in &profiles {
        if p.decodable {
            for &f in &p.families {
                global[f] += 1;
            }
        }
    }
    let weight = |f: usize| -> f64 { 1.0 / ((global[f] as f64) + std::f64::consts::E).ln() };

    let candidates: Vec<&WorkProfile> = profiles
        .iter()
        .filter(|p| {
            p.decodable
                && p.clean
                && p.band < 2 // exclude Large / Pathological (byte budget)
                && p.unknown_ratio < max_unknown_ratio
                && !p.families.is_empty()
                && !vendored_hashes.contains(p.content_hash.as_bytes())
        })
        .collect();

    // Greedy under a hard byte budget. Coverage phase (gain > 0): rank by
    // weighted coverage PER BYTE, so a small work covering a rare family beats a
    // large one covering the same — this front-loads cheap rare-family coverage
    // and keeps the golden set lean. Diversity phase (gain == 0, families
    // saturated): keep adding combination-diverse works (family-set not a subset
    // of one already chosen), smallest first. Integer keys → deterministic ties;
    // candidates are label-sorted so equal keys break label-ascending.
    let mut covered = seed_covered;
    let mut selected_sets: Vec<Vec<usize>> = Vec::new();
    let mut chosen: Vec<(&WorkProfile, Vec<usize>)> = Vec::new();
    let mut used = vec![false; candidates.len()];
    let mut spent: usize = 0;

    // Force-include the smallest clean work per --require-family, ahead of the
    // greedy pass. Skips a family already covered (by seed or a prior force-pick)
    // so overlapping requirements coalesce; a family no candidate exercises is
    // reported unsatisfiable (it needs a crafted render fixture, not a work).
    let mut unsatisfiable: Vec<&'static str> = Vec::new();
    for (name, id) in &required {
        if covered.contains(id) {
            continue;
        }
        let pick = candidates
            .iter()
            .enumerate()
            .filter(|(i, c)| !used[*i] && c.families.contains(id))
            .min_by(|(_, a), (_, b)| a.len.cmp(&b.len).then_with(|| a.label.cmp(&b.label)))
            .map(|(i, _)| i);
        match pick {
            Some(i) => {
                let c = candidates[i];
                used[i] = true;
                spent += c.len;
                let new_families: Vec<usize> = c
                    .families
                    .iter()
                    .copied()
                    .filter(|f| !covered.contains(f))
                    .collect();
                for &f in &c.families {
                    covered.insert(f);
                }
                selected_sets.push(c.families.clone());
                chosen.push((c, new_families));
                eprintln!(
                    "select-works: require-family {name} → {} ({} B)",
                    c.label, c.len
                );
            }
            None => unsatisfiable.push(family_name(*id)),
        }
    }
    if !unsatisfiable.is_empty() {
        eprintln!(
            "select-works: WARNING — no clean corpus work exercises {}; craft a \
             fixtures/render/ fixture for each instead",
            unsatisfiable.join(", ")
        );
    }

    while chosen.len() < target {
        let mut best: Option<usize> = None;
        let mut best_key = (i64::MIN, i64::MIN, i64::MIN);
        for (i, c) in candidates.iter().enumerate() {
            if used[i] || spent + c.len > max_total_source_bytes {
                continue;
            }
            let gain: f64 = c
                .families
                .iter()
                .filter(|f| !covered.contains(*f))
                .map(|&f| weight(f))
                .sum();
            let key = if gain > 0.0 {
                // (phase=1, weighted gain per byte, weighted gain)
                (
                    1i64,
                    (gain / c.len as f64 * 1e12) as i64,
                    (gain * 1_000_000.0) as i64,
                )
            } else {
                // Redundancy guard: skip a work whose family-set ⊆ one already chosen.
                if selected_sets
                    .iter()
                    .any(|s| c.families.iter().all(|f| s.contains(f)))
                {
                    continue;
                }
                // (phase=0, distinct families PER BYTE → small diverse works win)
                (
                    0i64,
                    (c.families.len() as f64 / c.len as f64 * 1e9) as i64,
                    0i64,
                )
            };
            if best.is_none() || key > best_key {
                best = Some(i);
                best_key = key;
            }
        }
        let Some(i) = best else { break };
        used[i] = true;
        spent += candidates[i].len;
        let new_families: Vec<usize> = candidates[i]
            .families
            .iter()
            .copied()
            .filter(|f| !covered.contains(f))
            .collect();
        for &f in &candidates[i].families {
            covered.insert(f);
        }
        selected_sets.push(candidates[i].families.clone());
        chosen.push((candidates[i], new_families));
    }
    let covered_total = covered.len();

    let rows: Vec<WorkRow> = chosen
        .iter()
        .map(|(c, new)| WorkRow {
            path: c.label.clone(),
            slug: slug_from_label(&c.label),
            size: c.len,
            band: if c.band == 0 { "small" } else { "medium" },
            orthography: c.orthography,
            families: c.families.iter().map(|&f| family_name(f)).collect(),
            new_families: new.iter().map(|&f| family_name(f)).collect(),
        })
        .collect();

    let manifest = WorksManifest {
        note: format!(
            "Generated by `xtask corpus select-works` (deterministic). {} works \
             to extend fixtures/works/. Slugs are the aozora card id (w<id>); \
             rename any to a mnemonic before `xtask corpus vendor-works` if you \
             like. Rows are a greedy weighted family set-cover under a source-byte \
             budget; `new_families` is what each first contributed.",
            rows.len()
        ),
        work: rows,
    };
    let toml = toml::to_string_pretty(&manifest).map_err(|e| format!("serialise manifest: {e}"))?;
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    fs::write(out, &toml).map_err(|e| format!("write {}: {e}", out.display()))?;

    eprintln!(
        "select-works: {} candidates → {} selected ({} KiB source, {}/{} families covered incl. seed) \
         in {:.1}s → {} (review, then vendor-works)",
        candidates.len(),
        manifest.work.len(),
        spent / 1024,
        covered_total,
        FAM_TOTAL,
        start.elapsed().as_secs_f64(),
        out.display()
    );
    Ok(())
}

#[derive(Deserialize)]
struct WorksManifestIn {
    #[serde(default)]
    work: Vec<WorkRowIn>,
}

#[derive(Deserialize)]
struct WorkRowIn {
    path: String,
    #[serde(default)]
    slug: String,
}

/// Vendor the slugged works from a selection manifest into `dest/<slug>/source.txt`
/// (decoded, CRLF→LF). Skips rows with a blank slug; never writes `expected.html`.
fn vendor_works(root: Option<&Path>, manifest: &Path, dest: &Path) -> Result<(), String> {
    let corpus = resolve_corpus(root)?;
    let raw = fs::read_to_string(manifest)
        .map_err(|e| format!("read manifest {}: {e}", manifest.display()))?;
    let parsed: WorksManifestIn =
        toml::from_str(&raw).map_err(|e| format!("parse manifest: {e}"))?;

    let (mut added, mut updated, mut unchanged, mut skipped) = (0u32, 0u32, 0u32, 0u32);
    for row in &parsed.work {
        if row.slug.trim().is_empty() {
            skipped += 1;
            continue;
        }
        let path = corpus.root().join(&row.path);
        let bytes = fs::read(&path).map_err(|e| format!("read corpus {}: {e}", path.display()))?;
        let text = decode_auto(&bytes).map_err(|e| format!("decode {}: {e:?}", row.path))?;
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let dir = dest.join(&row.slug);
        fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        let src = dir.join("source.txt");
        match fs::read_to_string(&src) {
            Ok(old) if old == normalized => unchanged += 1,
            Ok(_) => {
                fs::write(&src, &normalized)
                    .map_err(|e| format!("write {}: {e}", src.display()))?;
                updated += 1;
            }
            Err(_) => {
                fs::write(&src, &normalized)
                    .map_err(|e| format!("write {}: {e}", src.display()))?;
                added += 1;
            }
        }
    }
    eprintln!(
        "vendor-works: {added} added, {updated} updated, {unchanged} unchanged, {skipped} skipped \
         (blank slug). Seed goldens: UPDATE_GOLDEN=1 cargo test -p aozora-conformance --test works_gate"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- scan_correctness (I-A / I-B / I-C) ---------------------------

    #[test]
    fn scan_correctness_flags_each_defect_class() {
        let count = |html: &str, cat: CorrCat| {
            scan_correctness(html)
                .iter()
                .filter(|h| h.cat == cat)
                .count()
        };
        // I-B: an empty ruby base fires; a real base (text or nested element)
        // does not — proves the check is not vacuous (corpus reports 0).
        assert_eq!(
            count(
                "<ruby><rp>(</rp><rt>あ</rt><rp>)</rp></ruby>",
                CorrCat::BadRuby
            ),
            1
        );
        assert_eq!(
            count(
                "<ruby>猫<rp>(</rp><rt>ねこ</rt><rp>)</rp></ruby>",
                CorrCat::BadRuby
            ),
            0
        );
        assert_eq!(
            count(
                r#"<ruby><span class="aozora-gaiji">x</span><rp>(</rp><rt>y</rt><rp>)</rp></ruby>"#,
                CorrCat::BadRuby,
            ),
            0,
        );
        // I-A: unclosed div at EOF, and a stray close; a balanced doc is clean.
        assert_eq!(count("<div>x", CorrCat::Unbalanced), 1);
        assert_eq!(count("<p><span>x</p>", CorrCat::Unbalanced), 1);
        assert_eq!(count("<p>x</p>", CorrCat::Unbalanced), 0);
        // I-C: an undeclared class fires; a real class (numeric stem collapsed)
        // does not.
        assert_eq!(
            count(
                r#"<span class="aozora-zzz-bogus"></span>"#,
                CorrCat::UndeclaredClass
            ),
            1,
        );
        assert_eq!(
            count(
                r#"<span class="aozora-gaiji"></span>"#,
                CorrCat::UndeclaredClass
            ),
            0
        );
    }

    // ---- band_slot ----------------------------------------------------

    #[test]
    fn band_slot_buckets_by_decoded_len() {
        assert_eq!(band_slot(0), 0, "empty → <50KB");
        assert_eq!(band_slot(SMALL_MAX - 1), 0, "just under 50KB → band 0");
        assert_eq!(band_slot(SMALL_MAX), 1, "50KB boundary → band 1");
        assert_eq!(band_slot(MEDIUM_MAX - 1), 1, "just under 500KB → band 1");
        assert_eq!(band_slot(MEDIUM_MAX), 2, "500KB boundary → band 2");
        assert_eq!(band_slot(LARGE_MAX - 1), 2, "just under 2MB → band 2");
        assert_eq!(band_slot(LARGE_MAX), 3, "2MB boundary → band 3");
        assert_eq!(band_slot(u32::MAX), 3, "huge → band 3");
    }

    // ---- render-leak scanner ------------------------------------------

    fn cats(html: &str) -> (usize, usize, usize) {
        let hits = visible_leak_markers(html);
        let count = |c: LeakCat| hits.iter().filter(|h| h.cat == c).count();
        (
            count(LeakCat::Ruby),
            count(LeakCat::Bar),
            count(LeakCat::Directive),
        )
    }

    #[test]
    fn leak_detects_quote_interior_ruby() {
        // The reported bug: a ruby that leaked as literal `《…》`.
        assert_eq!(cats("<p>「駄目《だめ》」</p>"), (1, 0, 0));
    }

    #[test]
    fn leak_detects_barred_ruby_and_directive_in_quote() {
        assert_eq!(cats("<p>「一｜時《じ》」</p>"), (1, 1, 0));
        assert_eq!(
            cats("<p>「娘［＃１段階小さな文字］」</p>"),
            (0, 0, 1),
            "leaked font-size directive inside a quote",
        );
    }

    #[test]
    fn leak_ignores_resolved_ruby() {
        // A correctly-formed <ruby> emits no bare 《》.
        let html = "<p><ruby>瞳<rp>(</rp><rt>ひとみ</rt><rp>)</rp></ruby></p>";
        assert_eq!(cats(html), (0, 0, 0));
    }

    #[test]
    fn leak_ignores_angle_quote_delimiters() {
        // `≪…≫` legitimately renders literal 《…》 inside an angle-quote span.
        let html = "<p><span class=\"aozora-angle-quote\">《重要》</span>な記述。</p>";
        assert_eq!(cats(html), (0, 0, 0));
    }

    #[test]
    fn leak_ignores_resolved_gaiji_notation_glyphs() {
        assert!(visible_leak_markers(
            r#"<p><span class="aozora-gaiji" data-codepoint="U+300A">《</span>x<span class="aozora-gaiji" data-codepoint="U+300B">》</span></p>"#
        )
        .is_empty());
        assert_eq!(
            visible_leak_markers(
                r#"<p><span class="aozora-gaiji" data-description="raw">［＃raw］</span></p>"#
            )
            .len(),
            1
        );
    }

    #[test]
    fn leak_ignores_nested_span_inside_angle_quote() {
        // A gaiji span nested inside an angle-quote must be suppressed with
        // it (span-depth tracking), and the outer 《…》 delimiters too.
        let html = "<p><span class=\"aozora-angle-quote\">《<span class=\"aozora-gaiji\" \
                    data-codepoint=\"U+775C\">睜</span>》</span></p>";
        assert_eq!(cats(html), (0, 0, 0));
    }

    #[test]
    fn leak_ignores_hidden_directive() {
        // A resolved directive keeps its raw ［＃…］ inside a hidden span.
        let html = "<p><span class=\"aozora-directive\" hidden>［＃］</span>：入力者注</p>";
        assert_eq!(cats(html), (0, 0, 0));
    }

    #[test]
    fn leak_ignores_empty_ruby() {
        // `《》` with no reading renders literally by design (empty_ruby).
        assert_eq!(cats("<p>青梅《》</p>"), (0, 0, 0));
    }

    #[test]
    fn leak_gate_accounts_for_explicit_literal_markup() {
        use aozora::LiteralMarkupKind;

        let literal = [
            LiteralMarkupKind::RubyDelimiters,
            LiteralMarkupKind::RubyBaseMarker,
            LiteralMarkupKind::DirectiveMarker,
        ];
        assert!(unaccounted_leak_markers("<p>六ヶ《むつか》 ｜ ［＃注</p>", &literal).is_empty());
        let hits = unaccounted_leak_markers("<p>六ヶ《むつか》 別《もれ》</p>", &literal[..1]);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].cat, LeakCat::Ruby);
    }

    #[test]
    fn aozora_body_strips_header_and_footer() {
        let src = "表題\n著者\n\n\
                   -------------------------------------------------------\n\
                   【テキスト中に現れる記号について】\n\
                   《》：ルビ\n\
                   ｜：ルビの付く文字列の始まりを特定する記号\n\
                   -------------------------------------------------------\n\
                   \n本文の「駄目《だめ》」です。\n\n\
                   底本：「作品集」出版社\n入力：誰か\n";
        let body = aozora_body(src);
        assert!(body.contains("本文の"), "body kept: {body:?}");
        assert!(!body.contains("《》：ルビ"), "header legend stripped");
        assert!(!body.contains("ルビの付く文字列"), "bar legend stripped");
        assert!(!body.contains("底本"), "footer stripped");
    }

    #[test]
    fn is_span_open_discriminates() {
        assert!(is_span_open("<span>"));
        assert!(is_span_open("<span class=\"aozora-gaiji\">"));
        assert!(!is_span_open("</span>"));
        assert!(!is_span_open("<sub>"));
        assert!(!is_span_open("<sup>"));
    }

    // ---- year_of ------------------------------------------------------

    fn at_epoch_secs(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn year_of_mid_year_timestamps() {
        // `year_of` carries ±1 day slop at year boundaries (documented), so
        // pin it to mid-year instants where the bucketing is unambiguous.
        // 2000-07-01 12:00:01 UTC = 962_452_801.
        assert_eq!(year_of(at_epoch_secs(962_452_801)), 2000, "mid-2000");
        // 2026-06-15 00:00:01 UTC = 1_781_481_601.
        assert_eq!(year_of(at_epoch_secs(1_781_481_601)), 2026, "mid-2026");
        // 1995-06-01 00:00:01 UTC = 802_044_001 (a typical corpus mtime era).
        assert_eq!(year_of(at_epoch_secs(802_044_001)), 1995, "mid-1995");
    }

    #[test]
    fn year_of_pre_epoch_does_not_panic() {
        // `duration_since(UNIX_EPOCH)` fails for pre-epoch times → maps to 0s,
        // landing on the 1969/1970 boundary; the contract is "doesn't panic".
        let before = UNIX_EPOCH - Duration::from_secs(100);
        let y = year_of(before);
        assert!((1969..=1970).contains(&y), "pre-epoch lands near 1970: {y}");
    }

    // ---- bar ----------------------------------------------------------

    #[test]
    fn bar_empty_total_is_blank() {
        assert_eq!(bar(5, 0, 40), "", "zero total → empty bar");
    }

    #[test]
    fn bar_full_and_partial() {
        assert_eq!(bar(10, 10, 8).chars().count(), 8, "full ratio fills width");
        assert_eq!(bar(0, 10, 8), "", "zero count → empty");
        // Half of width 8, rounded.
        assert_eq!(bar(5, 10, 8).chars().count(), 4, "half ratio → half width");
    }

    #[test]
    fn bar_never_exceeds_width() {
        assert_eq!(
            bar(100, 10, 5).chars().count(),
            5,
            "over-ratio is clamped to width"
        );
    }

    // ---- line_of ------------------------------------------------------

    #[test]
    fn line_of_counts_newlines_before_offset() {
        let text = "a\nb\nc";
        assert_eq!(line_of(text, 0), 1, "offset 0 → line 1");
        assert_eq!(line_of(text, 2), 2, "after first newline → line 2");
        assert_eq!(line_of(text, 4), 3, "after second newline → line 3");
    }

    #[test]
    fn line_of_clamps_offset_past_end() {
        let text = "x\ny";
        assert_eq!(
            line_of(text, 9999),
            2,
            "offset past end clamps to last line"
        );
    }

    // ---- is_digit_char ------------------------------------------------

    #[test]
    fn is_digit_char_ascii_and_fullwidth() {
        assert!(is_digit_char('0'), "ascii zero");
        assert!(is_digit_char('9'), "ascii nine");
        assert!(is_digit_char('０'), "fullwidth zero");
        assert!(is_digit_char('９'), "fullwidth nine");
        assert!(!is_digit_char('a'), "letter is not a digit");
        assert!(!is_digit_char('「'), "kanji bracket is not a digit");
    }

    // ---- gaiji_bucket -------------------------------------------------

    #[test]
    fn gaiji_bucket_absent() {
        assert_eq!(gaiji_bucket(None), 5, "no mencode → absent");
    }

    #[test]
    fn gaiji_bucket_unicode() {
        assert_eq!(gaiji_bucket(Some("U+9DD7")), 2, "U+ form → unicode");
    }

    #[test]
    fn gaiji_bucket_jis_level() {
        assert_eq!(gaiji_bucket(Some("第3水準1-15-94")), 0, "水準 → jisLevel");
    }

    #[test]
    fn gaiji_bucket_jis_triple() {
        assert_eq!(gaiji_bucket(Some("1-15-94")), 1, "N-N-N → jisTriple");
    }

    #[test]
    fn gaiji_bucket_page_line() {
        assert_eq!(gaiji_bucket(Some("123-4")), 3, "N-N → pageLine");
    }

    #[test]
    fn gaiji_bucket_named_fallback() {
        assert_eq!(gaiji_bucket(Some("猫の絵")), 4, "free-form → named");
        assert_eq!(gaiji_bucket(Some("1-2-3-4")), 4, "four-part dash → named");
    }

    #[test]
    fn gaiji_bucket_takes_leading_token() {
        // Only the head token (before a separator) drives the dash analysis;
        // trailing prose after a separator is ignored.
        assert_eq!(
            gaiji_bucket(Some("1-15-94、参照")),
            1,
            "leading triple wins; trailing prose dropped"
        );
        assert_eq!(
            gaiji_bucket(Some("123-4 注")),
            3,
            "leading page-line pair via space separator"
        );
    }

    #[test]
    fn gaiji_bucket_jis_level_short_circuits_dash_analysis() {
        // The 水準 substring check precedes the dash analysis, so a triple
        // that also carries 水準 classifies as jisLevel, not jisTriple.
        assert_eq!(
            gaiji_bucket(Some("1-15-94、第3水準")),
            0,
            "水準 substring wins over the leading triple"
        );
    }

    // ---- normalize_shape ---------------------------------------------

    #[test]
    fn normalize_shape_folds_quoted_operands() {
        assert_eq!(
            normalize_shape("「猫」は太字"),
            "「」は太字",
            "quoted content dropped"
        );
        // Two distinct operands fold to the same shape.
        assert_eq!(
            normalize_shape("「猫」は太字"),
            normalize_shape("「犬」は太字")
        );
    }

    #[test]
    fn normalize_shape_collapses_digit_runs() {
        assert_eq!(normalize_shape("第123水準"), "第N水準", "digit run → N");
        assert_eq!(
            normalize_shape("１２３行"),
            "N行",
            "fullwidth digit run → single N"
        );
    }

    #[test]
    fn normalize_shape_plain_text_unchanged() {
        assert_eq!(
            normalize_shape("改ページ"),
            "改ページ",
            "no operands/digits"
        );
    }

    // ---- kv_sorted ----------------------------------------------------

    #[test]
    fn kv_sorted_drops_zero_counts() {
        let kvs = kv_sorted(vec![("a".to_owned(), 0), ("b".to_owned(), 3)]);
        assert_eq!(kvs.len(), 1, "zero-count rows are dropped");
        assert_eq!(kvs[0].key, "b", "non-zero row kept");
    }

    #[test]
    fn kv_sorted_orders_by_count_desc_then_key() {
        let kvs = kv_sorted(vec![
            ("zebra".to_owned(), 2),
            ("apple".to_owned(), 5),
            ("mango".to_owned(), 2),
        ]);
        let order: Vec<&str> = kvs.iter().map(|k| k.key.as_str()).collect();
        // Highest count first; ties broken by ascending key.
        assert_eq!(
            order,
            ["apple", "mango", "zebra"],
            "count-desc then key-asc"
        );
    }

    // ---- truncate_for_display -----------------------------------------

    #[test]
    fn truncate_for_display_keeps_short_strings() {
        assert_eq!(
            truncate_for_display("猫", 10),
            "猫",
            "under limit is untouched"
        );
    }

    #[test]
    fn truncate_for_display_truncates_by_char_with_ellipsis() {
        let out = truncate_for_display("あいうえお", 3);
        assert_eq!(out, "あい…", "truncate to (max-1) chars + ellipsis");
        assert_eq!(out.chars().count(), 3, "char count respects the limit");
    }

    // ---- analyze (real parser, pure str → FileStat) -------------------

    #[test]
    fn analyze_counts_ruby_node_kind() {
        // `｜青空《あおぞら》` is a base-ruby span → one Ruby node.
        let stat = analyze("｜青空《あおぞら》文庫");
        let ruby_idx = NodeKind::ALL
            .iter()
            .position(|k| *k == NodeKind::Ruby)
            .expect("Ruby is in ALL");
        assert!(stat.node_kinds[ruby_idx] >= 1, "ruby node tallied");
    }

    #[test]
    fn analyze_plain_text_has_no_annotations_or_gaiji() {
        let stat = analyze("ただのテキストです");
        assert_eq!(stat.gaiji_total, 0, "no gaiji in plain text");
        assert_eq!(
            stat.annotation_kinds.iter().sum::<u64>(),
            0,
            "no annotations in plain text"
        );
    }

    #[test]
    fn analyze_editorial_annotation_has_an_explicit_bucket() {
        let stat = analyze("前置き\n［＃まったく未知の指示］");
        assert_eq!(stat.annotation_kinds[1], 1);
        assert!(stat.unknown.is_empty());
    }

    // ---- audit_one (CorpusItem → FileStat) ----------------------------

    #[test]
    fn audit_one_carries_label_through() {
        let item = CorpusItem::new("a/b.txt", "ただのテキスト".as_bytes().to_vec());
        let stat = audit_one(item);
        assert_eq!(stat.label, "a/b.txt", "label preserved");
        assert!(!stat.decode_error, "valid UTF-8 decodes");
        assert!(!stat.panicked, "well-formed doc does not panic");
    }

    #[test]
    fn audit_one_flags_undecodable_bytes() {
        // Lone continuation byte: not valid UTF-8 and not valid Shift_JIS text.
        let item = CorpusItem::new("bad.txt", vec![0xFF, 0xFE, 0xFD, 0xFC]);
        let stat = audit_one(item);
        // Either it decodes (lenient) with no error, or it flags decode_error;
        // in both cases the label is intact and it must not panic.
        assert_eq!(stat.label, "bad.txt", "label preserved even on decode path");
        assert!(!stat.panicked, "decode failure is recorded, not a panic");
    }

    // ---- merge --------------------------------------------------------

    fn stat_labeled(label: &str) -> FileStat {
        FileStat {
            label: label.to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn merge_tallies_files_and_node_kinds() {
        let mut s = FileStat {
            label: "doc1".to_owned(),
            ..Default::default()
        };
        s.node_kinds[0] = 3; // Ruby
        s.gaiji_total = 2;
        s.gaiji_unresolved = 1;
        s.gaiji_forms[2] = 2; // unicode
        let results = vec![Ok(s), Ok(stat_labeled("doc2"))];
        let report = merge(results, "root".to_owned(), 1.5);

        assert_eq!(report.files_total, 2, "two readable files");
        assert_eq!(report.files_analyzed, 2, "both analyzed");
        assert_eq!(report.corpus_root, "root", "root recorded");
        assert!(
            (report.elapsed_secs - 1.5).abs() < f64::EPSILON,
            "elapsed kept"
        );
        assert_eq!(report.gaiji.total, 2, "gaiji summed");
        assert_eq!(report.gaiji.unresolved, 1, "unresolved summed");
        // Ruby is index 0 in NodeKind::ALL with camelCase "ruby".
        let ruby = report
            .node_kinds
            .iter()
            .find(|kv| kv.key == "ruby")
            .expect("ruby row present");
        assert_eq!(ruby.count, 3, "ruby count aggregated");
    }

    #[test]
    fn merge_separates_decode_errors_panics_and_walk_errors() {
        let decode = FileStat {
            label: "decode".to_owned(),
            decode_error: true,
            ..Default::default()
        };
        let panicked = FileStat {
            label: "boom".to_owned(),
            panicked: true,
            ..Default::default()
        };
        let walk_err = Err(aozora_corpus::CorpusError::RootNotDirectory {
            path: PathBuf::from("/nope"),
        });
        let results = vec![Ok(decode), Ok(panicked), walk_err, Ok(stat_labeled("good"))];
        let report = merge(results, "r".to_owned(), 0.0);

        assert_eq!(report.walk_errors, 1, "one walk error");
        assert_eq!(report.decode_errors, 1, "one decode error");
        assert_eq!(report.panic_count, 1, "one panic");
        assert_eq!(
            report.panics,
            vec!["boom".to_owned()],
            "panic label captured"
        );
        // files_total counts everything that read OK (decode + panic + good),
        // not the walk error; files_analyzed only the clean one.
        assert_eq!(report.files_total, 3, "three files read OK");
        assert_eq!(report.files_analyzed, 1, "only one fully analyzed");
    }

    #[test]
    fn merge_folds_unknown_bodies_into_shapes_with_smallest_example() {
        let mk = |label: &str, body: &str, line: u32| FileStat {
            label: label.to_owned(),
            annotation_kinds: {
                let mut a = [0u64; ANN_KIND_LABELS.len()];
                a[0] = 1;
                a
            },
            unknown: vec![(body.to_owned(), line)],
            ..Default::default()
        };
        // Two bodies fold to the same shape "「」は太字".
        let results = vec![
            Ok(mk("zfile", "「猫」は太字", 5)),
            Ok(mk("afile", "「犬」は太字", 9)),
        ];
        let report = merge(results, "r".to_owned(), 0.0);

        assert_eq!(report.unknown_total, 2, "two unknown occurrences");
        assert_eq!(report.unknown_distinct, 2, "two distinct bodies");
        assert_eq!(
            report.unknown_shapes.len(),
            1,
            "folded into one shape family"
        );
        let shape = &report.unknown_shapes[0];
        assert_eq!(shape.shape, "「」は太字", "operands folded out");
        assert_eq!(shape.count, 2, "shape covers both occurrences");
        assert_eq!(shape.distinct, 2, "shape spans two distinct bodies");

        // Smallest-example selection is lexicographic: "afile:9" < "zfile:5".
        let cat = report
            .unknown_bodies
            .iter()
            .find(|r| r.body == "「犬」は太字")
            .expect("dog body present");
        assert_eq!(cat.example, "afile:9", "lexicographically smallest example");
    }

    #[test]
    fn merge_empty_input_yields_zeroed_report() {
        let report = merge(Vec::new(), "r".to_owned(), 0.0);
        assert_eq!(report.files_total, 0, "no files");
        assert_eq!(report.unknown_total, 0, "no unknowns");
        assert!(report.node_kinds.is_empty(), "no node-kind rows");
        assert!(report.unknown_shapes.is_empty(), "no shapes");
    }

    // ---- print_band_row / header (smoke: must not panic) --------------

    #[test]
    fn print_band_row_handles_empty_and_populated_bands() {
        // These print to stdout; we only assert they run without panicking
        // for both the empty and populated paths, raw and zstd.
        let empty: Vec<&EntryMeta> = Vec::new();
        print_band_header(false);
        print_band_header(true);
        print_band_row("<50KB", &empty, false);
        print_band_row("<50KB", &empty, true);

        let meta = EntryMeta {
            payload_offset: 0,
            payload_len: 100,
            decoded_len: 400,
            source_mtime_ns: 0,
            source_blake3: [0u8; 32],
            label: "x".to_owned(),
        };
        let entries = vec![&meta];
        print_band_row("<50KB", &entries, false);
        print_band_row("<50KB", &entries, true);
    }

    fn stat(files: usize, occurrences: usize) -> MarkerStat {
        MarkerStat { files, occurrences }
    }

    #[test]
    fn tally_render_leaks_counts_files_and_occurrences() {
        let leak = |label: &str, cats: &[LeakCat]| {
            Ok(DocRenderOutcome::Leaked {
                label: label.to_owned(),
                hits: cats
                    .iter()
                    .map(|&cat| LeakHit {
                        cat,
                        snippet: String::new(),
                    })
                    .collect(),
            })
        };
        let results = vec![
            leak("a", &[LeakCat::Ruby, LeakCat::Ruby]), // 1 file, 2 ruby occ
            leak("b", &[LeakCat::Ruby, LeakCat::Bar]),  // ruby +1/+1, bar +1/+1
            Ok(DocRenderOutcome::Clean),
            Ok(DocRenderOutcome::DecodeSkipped),
        ];
        let (cur, scanned, decode_errors, panicked, walk_errors) = tally_render_leaks(results);
        assert_eq!(cur.ruby, stat(2, 3));
        assert_eq!(cur.bar, stat(1, 1));
        assert_eq!(cur.directive, stat(0, 0));
        assert_eq!(scanned, 3, "2 leaked + 1 clean; decode-skip not scanned");
        assert_eq!(decode_errors, 1);
        assert!(panicked.is_empty());
        assert_eq!(walk_errors, 0);
    }

    #[test]
    fn slug_from_label_uses_card_id_then_hashes() {
        assert_eq!(
            slug_from_label("作品/種田山頭火/其中日記（49258_ruby_36099）.txt"),
            "w49258"
        );
        assert_eq!(
            slug_from_label("作品/岩野泡鳴/神秘的半獣主義（733_txt）.txt"),
            "w733"
        );
        // No digits in the filename → deterministic content-hash fallback.
        let a = slug_from_label("作品/中原中也/コキューの憶ひ出.txt");
        assert!(a.starts_with('w') && a.len() == 11, "hash slug: {a}");
        assert_eq!(
            a,
            slug_from_label("作品/中原中也/コキューの憶ひ出.txt"),
            "slug is deterministic"
        );
    }

    #[test]
    fn orthography_of_reads_the_filename_tag() {
        assert_eq!(orthography_of("x（新字新仮名）.txt"), "新字新仮名");
        assert_eq!(orthography_of("x（旧字旧仮名）.txt"), "旧字旧仮名");
        assert_eq!(orthography_of("plain.txt"), "");
    }

    #[test]
    fn family_ids_and_names_span_the_universe() {
        let mut node_kinds = [0u64; NodeKind::ALL.len()];
        node_kinds[0] = 3;
        let mut annotation_kinds = [0u64; ANN_KIND_LABELS.len()];
        annotation_kinds[0] = 5;
        annotation_kinds[1] = 2;
        let mut gaiji_forms = [0u64; GAIJI_FORM_LABELS.len()];
        gaiji_forms[0] = 1;
        let s = FileStat {
            node_kinds,
            annotation_kinds,
            gaiji_forms,
            ..FileStat::default()
        };
        assert_eq!(
            family_ids(&s),
            vec![0, FAM_NODE, FAM_NODE + 1, FAM_NODE + FAM_ANN]
        );
        assert_eq!(family_name(0), NodeKind::ALL[0].as_json_tag());
        assert_eq!(family_name(FAM_NODE), ANN_KIND_LABELS[0]);
        assert_eq!(family_name(FAM_NODE + FAM_ANN), GAIJI_FORM_LABELS[0]);
        assert_eq!(FAM_TOTAL, FAM_NODE + FAM_ANN + FAM_GAIJI);
    }

    #[test]
    fn classify_body_buckets_against_catalogues() {
        // A Tier1 near-miss, framed as it appears in the corpus.
        assert_eq!(classify_body("［＃字下げ終わり］"), Bucket::Tier1);
        // A Tier2 degraded (lossy) form.
        assert_eq!(
            classify_body("［＃ここから最後まで3字下げ］"),
            Bucket::Tier2
        );
        // Genuine editorial prose — the discovery residue, matched by neither.
        assert_eq!(
            classify_body("［＃「甲」は「乙」の誤記か］"),
            Bucket::Residue
        );
        // The frame strip + trim mirrors the serializer: a bare body classifies too.
        assert_eq!(classify_body("字下げ終わり"), Bucket::Tier1);
    }

    #[test]
    fn classify_report_tallies_buckets_and_ranks_residue() {
        fn row(body: &str, count: u64, example: &str, shape: &str) -> UnknownRow {
            UnknownRow {
                body: body.to_owned(),
                count,
                example: example.to_owned(),
                shape: shape.to_owned(),
            }
        }
        // Bodies sorted by count desc, as run_audit emits them.
        let bodies = vec![
            row(
                "［＃「甲」は「乙」の誤記か］",
                7,
                "a:1",
                "［＃「」は「」の誤記か］",
            ), // residue
            row("［＃字下げ終わり］", 5, "b:2", "［＃字下げ終わり］"), // Tier1
            row(
                "［＃ここから最後まで3字下げ］",
                3,
                "c:3",
                "［＃ここから最後までN字下げ］",
            ), // Tier2
            row("［＃初出時「甲」］", 2, "d:4", "［＃初出時「」］"),   // residue
        ];
        let r = classify_report(&bodies, 100, 17);

        assert_eq!(r.tier1_occurrences, 5);
        assert_eq!(r.tier2_occurrences, 3);
        assert_eq!(r.residue_occurrences, 9); // 7 + 2
        assert_eq!(r.residue_distinct_shapes, 2);
        assert!((r.resolved_ratio - 8.0 / 17.0).abs() < 1e-9);
        // Residue ranked by count descending, with the highest-count sample kept.
        assert_eq!(r.residue_shapes[0].count, 7);
        assert_eq!(r.residue_shapes[0].sample, "［＃「甲」は「乙」の誤記か］");
        assert_eq!(r.residue_shapes[1].count, 2);
    }

    #[test]
    fn missing_families_is_the_uncovered_complement() {
        let mut covered: std::collections::HashSet<usize> = (0..FAM_TOTAL).collect();
        assert!(
            missing_families(&covered).is_empty(),
            "full set → none missing"
        );

        covered.remove(&0);
        covered.remove(&FAM_NODE); // first annotation family
        let missing = missing_families(&covered);
        assert_eq!(missing.len(), 2);
        assert!(missing.contains(&family_name(0)));
        assert!(missing.contains(&family_name(FAM_NODE)));
    }
}
