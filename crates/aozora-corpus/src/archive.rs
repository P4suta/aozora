//! Single-file binary corpus archive (L-5, ADR-0020).
//!
//! Replaces "directory of 17 435 small files + walkdir + per-file
//! `read(2)` + `decode_sjis` per call" with **one** binary file that
//! contains the entire corpus pre-packed and (optionally)
//! pre-decoded and/or zstd-compressed.
//!
//! # Why a custom binary format
//!
//! The reference data structures are **Git pack files**, **SQLite
//! page files**, and **`Apache Arrow IPC`**: a single sequentially
//! laid-out file with an offset index for O(1) random access. For
//! Aozora's read-only corpus-sweep workload these are overkill; we
//! need only a minimal subset. The format is:
//!
//! ```text
//! [Header — 16 bytes]
//!   4   magic       = b"AOZC"
//!   4   version     = u32 LE (currently 1)
//!   4   flags       = u32 LE
//!         bit 0:  payload is zstd-compressed per-entry
//!         bit 1:  payload is pre-decoded UTF-8 (otherwise raw SJIS)
//!   4   entry_count = u32 LE
//!
//! [Index — entry_count records, packed back-to-back, variable size]
//!   8   payload_offset   : u64 LE  (from start of file)
//!   4   payload_len      : u32 LE  (bytes in file, possibly compressed)
//!   4   decoded_len      : u32 LE  (bytes after decompress; for raw
//!                                   payloads equals payload_len)
//!   8   source_mtime_ns  : i64 LE  (for incremental-pack diff)
//!   32  source_blake3    : [u8;32] (for incremental-pack diff)
//!   4   label_len        : u32 LE
//!   ?   label_bytes      : variable UTF-8
//!
//! [Payload]
//!   Concatenated entry bytes, each at its declared offset.
//! ```
//!
//! Four shipping variants (selected via flags at build time):
//!
//! | flags  | name              | trade-off |
//! |---|---|---|
//! | `0b00` | raw SJIS          | smallest build cost, current decode work |
//! | `0b01` | zstd SJIS         | smaller disk, current decode work |
//! | `0b10` | pre-decoded UTF-8 | larger disk, decode skipped at runtime |
//! | `0b11` | zstd UTF-8        | smallest runtime wall (read+decompress only) |
//!
//! # Why mtime + blake3 instead of mtime alone
//!
//! mtime can be stale (filesystem races, `touch`-without-content-change,
//! cache restored from backup). blake3 confirms identity when mtime
//! says "maybe changed". Per-entry hash adds ~32 bytes to the index
//! (negligible for a 17 k-file corpus = ~544 KB index).
//!
//! # Lifetime model
//!
//! [`Archive::open`] mmaps would be ideal for zero-copy reads, but
//! `unsafe` is non-negotiable here (ADR-0020 § L-4). Instead, the
//! whole file is read into a single `Vec<u8>` (one `fs::read`
//! syscall, kernel does one page-cache → vec memcpy). [`ArchiveEntry`]
//! borrows from the archive's payload, so iterators yield zero-copy
//! `&[u8]` slices for raw payloads. zstd-decompressed entries
//! materialise into a fresh `Vec<u8>` per call (decompression
//! intrinsically allocates).

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::missing_panics_doc,
    clippy::too_many_arguments,
    clippy::doc_markdown,
    clippy::too_many_lines,
    reason = "binary-format parser/builder module; on-disk type widths and magic-number boundaries are documented inline in the format spec at the top of this file. The casts are intentional and bounded by the spec; panics are unreachable per the spec's `try_from` checks at the API boundary."
)]

use core::error;
use core::fmt;
use core::str;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::CorpusError;

/// Magic bytes at the start of every archive — `b"AOZC"` (Aozora Corpus).
pub const MAGIC: [u8; 4] = *b"AOZC";

/// Current archive format version.
pub const FORMAT_VERSION: u32 = 1;

/// Flag bit indicating per-entry zstd compression.
pub const FLAG_ZSTD: u32 = 1 << 0;

/// Flag bit indicating the payload is pre-decoded UTF-8 text. When
/// unset, the payload is raw Shift-JIS bytes — same shape as
/// [`crate::CorpusItem::bytes`] in the directory-walker path.
pub const FLAG_UTF8: u32 = 1 << 1;

/// Length of a per-entry index record's fixed header. The label
/// (variable-length) follows.
const INDEX_FIXED_LEN: usize = 8 + 4 + 4 + 8 + 32 + 4;

/// Length of the file header.
const HEADER_LEN: usize = 4 + 4 + 4 + 4;

/// Errors specific to archive format parsing / IO. Wraps generic
/// [`CorpusError::Io`] for filesystem failures and adds format-level
/// distinctions (bad magic, unsupported version, truncated index).
#[derive(Debug)]
#[non_exhaustive]
pub enum ArchiveError {
    /// I/O failed (read / write / open).
    Io(io::Error),
    /// File does not start with the archive magic bytes.
    BadMagic,
    /// File version is newer than this binary supports.
    UnsupportedVersion(u32),
    /// File is shorter than the declared header / index / payload
    /// would require.
    Truncated {
        /// What we tried to read (e.g. `"index entry 14233"`).
        what: &'static str,
    },
    /// Per-entry zstd decompression failed.
    Decompress(io::Error),
    /// Label bytes are not valid UTF-8.
    BadLabel,
}

impl fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "archive IO: {e}"),
            Self::BadMagic => f.write_str("file does not start with the AOZC magic bytes"),
            Self::UnsupportedVersion(v) => write!(
                f,
                "archive format version {v} is not supported by this binary"
            ),
            Self::Truncated { what } => write!(f, "archive truncated while reading {what}"),
            Self::Decompress(e) => write!(f, "zstd decompression failed: {e}"),
            Self::BadLabel => f.write_str("entry label is not valid UTF-8"),
        }
    }
}

impl error::Error for ArchiveError {}

impl From<io::Error> for ArchiveError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<ArchiveError> for CorpusError {
    fn from(value: ArchiveError) -> Self {
        match value {
            ArchiveError::Io(source) => Self::Io {
                path: PathBuf::new(),
                source,
            },
            other => Self::Io {
                path: PathBuf::new(),
                source: io::Error::other(other.to_string()),
            },
        }
    }
}

/// Per-entry metadata held in the archive's index. Mirrors the
/// on-disk record byte-for-byte; doc-comments document the on-disk
/// semantics.
#[derive(Debug, Clone)]
pub struct EntryMeta {
    /// Offset in bytes from the start of the archive file at which
    /// this entry's payload begins.
    pub payload_offset: u64,
    /// Number of payload bytes in the file. For zstd-compressed
    /// archives this is the compressed length; for raw archives it
    /// equals `decoded_len`.
    pub payload_len: u32,
    /// Number of bytes after decompression. For raw archives this
    /// equals `payload_len`. Useful for callers that want to
    /// pre-allocate a destination buffer before decoding.
    pub decoded_len: u32,
    /// Source-file `mtime` in nanoseconds since UNIX epoch. Used by
    /// the incremental pack diff to detect "may have changed" files
    /// before falling back to full-content `blake3` confirmation.
    pub source_mtime_ns: i64,
    /// blake3 hash of the source file bytes (raw SJIS bytes for
    /// archives built from a directory of `.txt` files). Acts as the
    /// content-addressed identity used by the incremental pack diff.
    pub source_blake3: [u8; 32],
    /// Human-readable label (typically the path relative to the
    /// corpus root).
    pub label: String,
}

/// A loaded corpus archive — header + parsed index + payload bytes
/// held in memory.
#[derive(Debug)]
pub struct Archive {
    flags: u32,
    entries: Vec<EntryMeta>,
    /// Raw archive bytes from offset 0 to end. Per-entry payloads
    /// are slices into this buffer at `entry.payload_offset ..
    /// entry.payload_offset + entry.payload_len`.
    bytes: Vec<u8>,
}

impl Archive {
    /// Open and fully load an archive from disk. Reads the file in
    /// one [`fs::read`] call; subsequent [`Self::iter`] / [`Self::get`]
    /// calls slice into the in-memory buffer.
    ///
    /// # Errors
    ///
    /// Returns [`ArchiveError`] if the file is missing, malformed, or
    /// uses a version this binary does not support.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ArchiveError> {
        let bytes = fs::read(path)?;
        Self::from_bytes(bytes)
    }

    /// Parse an archive from an already-loaded byte buffer.
    ///
    /// # Errors
    ///
    /// Same shape as [`Self::open`] (minus the I/O case).
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, ArchiveError> {
        if bytes.len() < HEADER_LEN {
            return Err(ArchiveError::Truncated { what: "header" });
        }
        if bytes[0..4] != MAGIC {
            return Err(ArchiveError::BadMagic);
        }
        let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        if version != FORMAT_VERSION {
            return Err(ArchiveError::UnsupportedVersion(version));
        }
        let flags = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        let count = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;

        let mut entries = Vec::with_capacity(count);
        let mut cursor = HEADER_LEN;
        for i in 0..count {
            if cursor + INDEX_FIXED_LEN > bytes.len() {
                return Err(ArchiveError::Truncated {
                    what: "index entry header",
                });
            }
            let payload_offset =
                u64::from_le_bytes(bytes[cursor..cursor + 8].try_into().expect("8-byte slice"));
            let payload_len = u32::from_le_bytes(
                bytes[cursor + 8..cursor + 12]
                    .try_into()
                    .expect("4-byte slice"),
            );
            let decoded_len = u32::from_le_bytes(
                bytes[cursor + 12..cursor + 16]
                    .try_into()
                    .expect("4-byte slice"),
            );
            let source_mtime_ns = i64::from_le_bytes(
                bytes[cursor + 16..cursor + 24]
                    .try_into()
                    .expect("8-byte slice"),
            );
            let mut source_blake3 = [0u8; 32];
            source_blake3.copy_from_slice(&bytes[cursor + 24..cursor + 56]);
            let label_len = u32::from_le_bytes(
                bytes[cursor + 56..cursor + 60]
                    .try_into()
                    .expect("4-byte slice"),
            ) as usize;
            cursor += INDEX_FIXED_LEN;
            if cursor + label_len > bytes.len() {
                return Err(ArchiveError::Truncated {
                    what: "index entry label",
                });
            }
            let label = str::from_utf8(&bytes[cursor..cursor + label_len])
                .map_err(|_| ArchiveError::BadLabel)?
                .to_owned();
            cursor += label_len;

            // Validate that the declared payload range fits in the
            // file before we hand it out to readers.
            let payload_end = payload_offset.checked_add(u64::from(payload_len)).ok_or(
                ArchiveError::Truncated {
                    what: "index entry payload range overflow",
                },
            )?;
            if payload_end > bytes.len() as u64 {
                return Err(ArchiveError::Truncated {
                    what: "index entry payload past end of file",
                });
            }
            let _ = i; // bound-check uses i implicitly via cursor advance
            entries.push(EntryMeta {
                payload_offset,
                payload_len,
                decoded_len,
                source_mtime_ns,
                source_blake3,
                label,
            });
        }

        Ok(Self {
            flags,
            entries,
            bytes,
        })
    }

    /// Whether the archive's payloads are zstd-compressed per entry.
    #[must_use]
    pub fn is_zstd(&self) -> bool {
        self.flags & FLAG_ZSTD != 0
    }

    /// Whether the archive's payloads are pre-decoded UTF-8 (rather
    /// than raw Shift-JIS bytes).
    #[must_use]
    pub fn is_utf8(&self) -> bool {
        self.flags & FLAG_UTF8 != 0
    }

    /// Number of entries in the archive.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the archive is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Borrow the underlying flags word — useful for callers that
    /// want to surface "this archive is zstd-compressed UTF-8" etc.
    #[must_use]
    pub fn flags(&self) -> u32 {
        self.flags
    }

    /// Borrow every entry's metadata. Iterating this is much cheaper
    /// than [`Self::iter`] when the caller only needs labels /
    /// lengths.
    #[must_use]
    pub fn entries(&self) -> &[EntryMeta] {
        &self.entries
    }

    /// Random-access counterpart to [`Self::iter_borrowed`] — yield
    /// the payload at `index` directly, no iteration cost (L-6c).
    ///
    /// Used by parallel callers that fan out via `(0..len).into_par_iter()`
    /// — calling `iter_borrowed().nth(i)` inside the parallel body
    /// would be O(n) per item and turn the whole loop O(n²);
    /// `payload_at(i)` is O(1).
    ///
    /// # Errors
    ///
    /// Returns [`ArchiveError::Decompress`] if the entry is
    /// zstd-compressed and decompression fails.
    ///
    /// # Panics
    ///
    /// Panics if `index >= self.len()`.
    pub fn payload_at(&self, index: usize) -> Result<ArchivePayload<'_>, ArchiveError> {
        let entry = &self.entries[index];
        let raw = self.raw_payload(index);
        if self.is_zstd() {
            let bytes = decompress_payload(raw, entry.decoded_len as usize)
                .map_err(ArchiveError::Decompress)?;
            Ok(ArchivePayload::Decompressed(bytes))
        } else {
            Ok(ArchivePayload::Borrowed(raw))
        }
    }

    /// Borrow the raw payload slice for a given entry — without
    /// decompression. Returns the on-disk bytes (potentially
    /// zstd-compressed). For the high-level reader use [`Self::iter`].
    ///
    /// # Panics
    ///
    /// Panics if `index >= self.len()`.
    #[must_use]
    pub fn raw_payload(&self, index: usize) -> &[u8] {
        let entry = &self.entries[index];
        let start = entry.payload_offset as usize;
        let end = start + entry.payload_len as usize;
        &self.bytes[start..end]
    }

    /// Iterate entries, decompressing each payload on the fly if
    /// needed. Yields owned `(label, bytes)` pairs — for raw
    /// archives the bytes are a `to_vec` of the in-memory slice;
    /// for zstd archives they are the freshly decompressed bytes.
    ///
    /// Use [`Self::iter_borrowed`] (L-6c) when the caller does not
    /// need to take ownership — zero-copy on raw archives.
    ///
    /// # Errors
    ///
    /// Returns [`ArchiveError::Decompress`] if a per-entry zstd
    /// decompression fails. Failures do not abort iteration; each
    /// item is yielded as `Result`.
    pub fn iter(&self) -> impl Iterator<Item = Result<(String, Vec<u8>), ArchiveError>> + '_ {
        self.iter_borrowed().map(|item| {
            let (label, payload) = item?;
            Ok((label.to_owned(), payload.into_owned()))
        })
    }

    /// Zero-copy iterator (L-6c, ADR-0020).
    ///
    /// Yields `(label_borrow, payload_borrow_or_decompress)` pairs
    /// where:
    ///
    /// - `label_borrow: &'a str` borrows directly from the archive's
    ///   in-memory index — no allocation per entry.
    /// - `payload` is an [`ArchivePayload<'a>`]: for raw archives it
    ///   borrows the in-memory payload slice (zero-copy); for zstd
    ///   archives it materialises the decompressed bytes into a fresh
    ///   `Vec<u8>` (decompression intrinsically allocates, so there
    ///   is no zero-copy path for compressed payloads — the
    ///   `ArchivePayload` enum exposes both shapes uniformly).
    ///
    /// Saves one `Vec<u8>` allocation + memcpy per entry on raw
    /// archives compared to [`Self::iter`]. On a 17 k-entry corpus
    /// this is a measurable allocator-pressure improvement.
    ///
    /// # Errors
    ///
    /// Same as [`Self::iter`] — per-entry zstd decompression failures
    /// surface as `Err` items.
    pub fn iter_borrowed(
        &self,
    ) -> impl Iterator<Item = Result<(&str, ArchivePayload<'_>), ArchiveError>> + '_ {
        self.entries.iter().enumerate().map(move |(i, entry)| {
            let payload = self.payload_at(i)?;
            Ok((entry.label.as_str(), payload))
        })
    }
}

/// Per-entry payload yielded by [`Archive::iter_borrowed`] (L-6c).
///
/// Unifies the two shapes the archive can produce — a borrowed slice
/// into the in-memory archive (raw variants) or a freshly
/// decompressed owned `Vec<u8>` (zstd variants) — behind one type so
/// callers don't have to switch on `archive.is_zstd()` themselves.
///
/// Both variants implement [`Self::as_bytes`] for borrow-only use,
/// and [`Self::into_owned`] for the cases where ownership is required
/// (e.g. building a `String` via `String::from_utf8`).
#[derive(Debug)]
pub enum ArchivePayload<'a> {
    /// Raw payload — slice into the archive's in-memory bytes. No
    /// per-entry allocation; lifetime tied to the [`Archive`].
    Borrowed(&'a [u8]),
    /// Freshly decompressed payload. Owned because zstd
    /// decompression intrinsically allocates a destination buffer.
    Decompressed(Vec<u8>),
}

impl ArchivePayload<'_> {
    /// Borrow the payload bytes irrespective of variant.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Borrowed(s) => s,
            Self::Decompressed(v) => v,
        }
    }

    /// Take ownership — copies the bytes if this is a `Borrowed`
    /// variant, returns the `Vec` directly if `Decompressed`.
    #[must_use]
    pub fn into_owned(self) -> Vec<u8> {
        match self {
            Self::Borrowed(s) => s.to_vec(),
            Self::Decompressed(v) => v,
        }
    }

    /// Length of the payload in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.as_bytes().len()
    }

    /// Whether the payload is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.as_bytes().is_empty()
    }
}

fn decompress_payload(src: &[u8], expected_len: usize) -> io::Result<Vec<u8>> {
    let mut out = Vec::with_capacity(expected_len);
    let mut decoder = zstd::stream::read::Decoder::new(src)?;
    decoder.read_to_end(&mut out)?;
    Ok(out)
}

/// Convert a [`SystemTime`] into a UNIX-epoch nanosecond count
/// suitable for [`EntryMeta::source_mtime_ns`]. Pre-epoch times
/// (which should not exist on real corpora) are truncated to 0.
#[must_use]
pub fn system_time_to_ns(t: SystemTime) -> i64 {
    t.duration_since(SystemTime::UNIX_EPOCH).map_or_else(
        |_| 0,
        |d| {
            // u128 → i64 saturating cast; corpus timestamps are well
            // within i64::MAX nanoseconds for any plausible decade.
            let ns = d.as_nanos();
            i64::try_from(ns.min(i64::MAX as u128)).unwrap_or(i64::MAX)
        },
    )
}

/// Inverse of [`system_time_to_ns`] — useful for debug printing /
/// equality reasoning. Saturating below epoch.
#[must_use]
pub fn ns_to_system_time(ns: i64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_nanos(ns.max(0) as u64)
}

// ---------------------------------------------------------------------
// Builder — used by `xtask corpus pack` (L-5b) at build time only.
// ---------------------------------------------------------------------

/// Streaming archive builder.
///
/// Append entries via [`push_entry`] (or [`push_already_encoded`] for
/// pre-encoded payloads, or [`push_prebuilt`] for verbatim copies
/// from a previous archive); then finalise via [`finish`] which
/// writes header + index + payload to the supplied path atomically
/// (write-to-tmpfile-then-rename).
///
/// [`push_entry`]: ArchiveBuilder::push_entry
/// [`push_already_encoded`]: ArchiveBuilder::push_already_encoded
/// [`push_prebuilt`]: ArchiveBuilder::push_prebuilt
/// [`finish`]: ArchiveBuilder::finish
#[derive(Debug)]
pub struct ArchiveBuilder {
    flags: u32,
    /// Per-entry metadata accumulated so far. `payload_offset` is set
    /// when the entry is pushed (offsets are computed against the
    /// growing `payload` Vec).
    entries: Vec<EntryMeta>,
    /// Concatenated payload bytes (already encoded — zstd-compressed
    /// if `flags & FLAG_ZSTD`, raw otherwise).
    payload: Vec<u8>,
    /// zstd compression level used when `flags & FLAG_ZSTD` is set.
    /// 19 is the long-mode default that gives the best ratio on
    /// Aozora's text profile (~5:1 on UTF-8 input).
    zstd_level: i32,
}

impl ArchiveBuilder {
    /// New empty builder. `flags` is the bitwise OR of [`FLAG_ZSTD`]
    /// and/or [`FLAG_UTF8`]; both default off.
    #[must_use]
    pub fn new(flags: u32) -> Self {
        Self {
            flags,
            entries: Vec::new(),
            payload: Vec::new(),
            zstd_level: 19,
        }
    }

    /// Override the zstd compression level (1..=22). The default
    /// (19) targets the maximum compression that still fits in
    /// reasonable build wall (a few seconds for the full corpus on a
    /// modern CPU). Has no effect when [`FLAG_ZSTD`] is unset.
    pub fn zstd_level(&mut self, level: i32) -> &mut Self {
        self.zstd_level = level;
        self
    }

    /// Append one entry to the archive in construction. `bytes` is
    /// the *unencoded* source bytes (raw SJIS for `!FLAG_UTF8`
    /// archives, UTF-8 for `FLAG_UTF8` archives); the builder
    /// compresses if needed and computes the blake3 hash.
    ///
    /// `source_mtime_ns` and the computed hash flow into the on-disk
    /// index for incremental-pack diff.
    ///
    /// # Errors
    ///
    /// Returns [`ArchiveError::Decompress`] if zstd encoding fails
    /// (it shouldn't on valid input but the error path is preserved
    /// for completeness).
    pub fn push_entry(
        &mut self,
        label: impl Into<String>,
        bytes: &[u8],
        source_mtime_ns: i64,
    ) -> Result<(), ArchiveError> {
        let source_blake3: [u8; 32] = blake3::hash(bytes).into();
        let decoded_len = u32::try_from(bytes.len()).expect("entry larger than u32 unsupported");
        let payload_offset_in_section = u64::try_from(self.payload.len())
            .expect("archive payload section larger than u64 unsupported");

        if self.flags & FLAG_ZSTD != 0 {
            use zstd::stream::copy_encode;
            let mut compressed = Vec::with_capacity(bytes.len() / 4);
            copy_encode(bytes, &mut compressed, self.zstd_level)
                .map_err(ArchiveError::Decompress)?;
            let payload_len =
                u32::try_from(compressed.len()).expect("compressed entry larger than u32");
            self.payload.extend_from_slice(&compressed);
            self.entries.push(EntryMeta {
                // payload_offset is fixed up to absolute file offset
                // in `finish`; for now it stores the offset within
                // the payload section.
                payload_offset: payload_offset_in_section,
                payload_len,
                decoded_len,
                source_mtime_ns,
                source_blake3,
                label: label.into(),
            });
        } else {
            self.payload.extend_from_slice(bytes);
            self.entries.push(EntryMeta {
                payload_offset: payload_offset_in_section,
                payload_len: decoded_len,
                decoded_len,
                source_mtime_ns,
                source_blake3,
                label: label.into(),
            });
        }
        Ok(())
    }

    /// Append an entry whose payload has already been encoded
    /// (compressed if the archive is `FLAG_ZSTD`, raw otherwise) by
    /// the caller — typically because the encode happened in a
    /// parallel work loop and only the assembly step is sequential.
    /// The caller has already computed `decoded_len` and the source
    /// hash; the builder simply records them as-is.
    pub fn push_already_encoded(
        &mut self,
        label: &str,
        payload: &[u8],
        decoded_len: u32,
        source_mtime_ns: i64,
        source_blake3: [u8; 32],
    ) {
        let payload_offset_in_section = u64::try_from(self.payload.len())
            .expect("archive payload section larger than u64 unsupported");
        let payload_len =
            u32::try_from(payload.len()).expect("payload longer than u32 unsupported");
        self.payload.extend_from_slice(payload);
        self.entries.push(EntryMeta {
            payload_offset: payload_offset_in_section,
            payload_len,
            decoded_len,
            source_mtime_ns,
            source_blake3,
            label: label.to_owned(),
        });
    }

    /// Append an entry whose compressed/raw payload is *already*
    /// encoded. Used by the incremental-pack path to copy unchanged
    /// entries verbatim from a previous archive without
    /// recompressing.
    pub fn push_prebuilt(&mut self, meta: EntryMeta, payload: &[u8]) {
        let payload_offset_in_section = u64::try_from(self.payload.len())
            .expect("archive payload section larger than u64 unsupported");
        self.payload.extend_from_slice(payload);
        self.entries.push(EntryMeta {
            payload_offset: payload_offset_in_section,
            ..meta
        });
    }

    /// Finalise the archive: assemble header + index + payload and
    /// write atomically to `path`. The write goes to `path.tmp`
    /// first, then `rename(2)` to `path` — partially-written archives
    /// never leak.
    ///
    /// Returns the total file size in bytes (useful for reporting
    /// compression ratios).
    ///
    /// # Errors
    ///
    /// Returns [`ArchiveError::Io`] on filesystem failures.
    pub fn finish(self, path: impl AsRef<Path>) -> Result<u64, ArchiveError> {
        let path = path.as_ref();

        // Compute index section size to know the absolute payload
        // offset (so each entry's `payload_offset` can be promoted
        // from "offset within payload section" to "offset within
        // file").
        let index_section_len: usize = self
            .entries
            .iter()
            .map(|e| INDEX_FIXED_LEN + e.label.len())
            .sum();
        let payload_section_offset = HEADER_LEN + index_section_len;

        // Pre-size the output vector so the final `fs::write` is one
        // sequential memcpy with no growth realloc.
        let total_len = payload_section_offset + self.payload.len();
        let mut out = Vec::with_capacity(total_len);

        // Header
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        out.extend_from_slice(&self.flags.to_le_bytes());
        out.extend_from_slice(
            &u32::try_from(self.entries.len())
                .expect("entry count larger than u32 unsupported")
                .to_le_bytes(),
        );

        // Index
        for entry in &self.entries {
            let absolute_offset = u64::try_from(payload_section_offset)
                .expect("file offsets fit in u64")
                + entry.payload_offset;
            out.extend_from_slice(&absolute_offset.to_le_bytes());
            out.extend_from_slice(&entry.payload_len.to_le_bytes());
            out.extend_from_slice(&entry.decoded_len.to_le_bytes());
            out.extend_from_slice(&entry.source_mtime_ns.to_le_bytes());
            out.extend_from_slice(&entry.source_blake3);
            let label_len_u32 =
                u32::try_from(entry.label.len()).expect("label longer than u32 unsupported");
            out.extend_from_slice(&label_len_u32.to_le_bytes());
            out.extend_from_slice(entry.label.as_bytes());
        }

        // Payload
        out.extend_from_slice(&self.payload);
        debug_assert_eq!(out.len(), total_len, "archive layout invariant");

        let tmp = path.with_extension(format!(
            "{}.tmp",
            path.extension().and_then(|e| e.to_str()).unwrap_or("aozc")
        ));
        fs::write(&tmp, &out)?;
        fs::rename(&tmp, path)?;
        Ok(total_len as u64)
    }
}

#[cfg(test)]
mod tests {
    use core::ptr;

    use super::*;

    fn roundtrip(flags: u32, entries: &[(&str, &[u8])]) -> Archive {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("corpus.aozc");
        let mut builder = ArchiveBuilder::new(flags);
        for (label, bytes) in entries {
            builder
                .push_entry(*label, bytes, 0)
                .expect("push_entry must succeed");
        }
        let _ = builder.finish(&path).expect("finish must succeed");
        Archive::open(&path).expect("open must succeed")
    }

    #[test]
    fn roundtrip_raw_sjis_one_entry() {
        let arc = roundtrip(0, &[("a.txt", b"hello world")]);
        assert!(!arc.is_zstd());
        assert!(!arc.is_utf8());
        assert_eq!(arc.len(), 1);
        let items: Vec<_> = arc.iter().filter_map(Result::ok).collect();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].0, "a.txt");
        assert_eq!(items[0].1, b"hello world");
    }

    #[test]
    fn roundtrip_raw_utf8_three_entries() {
        let arc = roundtrip(
            FLAG_UTF8,
            &[
                ("a.txt", "あおい".as_bytes()),
                ("b/c.txt", "そら".as_bytes()),
                ("d.txt", "ぶん こ".as_bytes()),
            ],
        );
        assert!(arc.is_utf8());
        assert_eq!(arc.len(), 3);
        let items: Vec<_> = arc.iter().filter_map(Result::ok).collect();
        assert_eq!(items[0].0, "a.txt");
        assert_eq!(items[0].1, "あおい".as_bytes());
        assert_eq!(items[1].0, "b/c.txt");
        assert_eq!(items[1].1, "そら".as_bytes());
        assert_eq!(items[2].0, "d.txt");
        assert_eq!(items[2].1, "ぶん こ".as_bytes());
    }

    #[test]
    fn roundtrip_zstd_compressed_payload_decompresses() {
        let big = "あ".repeat(10_000); // ~30 KB highly compressible
        let arc = roundtrip(FLAG_ZSTD | FLAG_UTF8, &[("big.txt", big.as_bytes())]);
        assert!(arc.is_zstd());
        assert!(arc.is_utf8());
        // The on-disk payload is much smaller than the decompressed
        // length — verify the compression flag is doing real work.
        let raw_payload = arc.raw_payload(0);
        let entry = &arc.entries()[0];
        assert!(
            (raw_payload.len() as u32) < entry.decoded_len,
            "zstd payload should be smaller than decoded length"
        );
        let items: Vec<_> = arc.iter().filter_map(Result::ok).collect();
        assert_eq!(items[0].1, big.as_bytes());
    }

    #[test]
    fn iter_borrowed_returns_zero_copy_slice_for_raw_archive() {
        // For a raw archive (no zstd), iter_borrowed must yield the
        // payload as `Borrowed(&[u8])` whose pointer is *inside* the
        // archive's in-memory buffer — confirming zero-copy.
        let arc = roundtrip(0, &[("a.txt", b"hello world")]);
        let items: Vec<_> = arc.iter_borrowed().filter_map(Result::ok).collect();
        assert_eq!(items.len(), 1);
        let (label, payload) = &items[0];
        assert_eq!(*label, "a.txt");
        match payload {
            ArchivePayload::Borrowed(slice) => {
                assert_eq!(*slice, b"hello world");
                // The slice's pointer must lie inside `arc.bytes`'s
                // buffer (i.e. the archive's in-memory mmap-equivalent).
                let archive_payload_ptr = arc.raw_payload(0).as_ptr();
                assert!(
                    ptr::eq(archive_payload_ptr, slice.as_ptr()),
                    "iter_borrowed must yield the archive's own buffer slice (zero-copy)"
                );
            }
            ArchivePayload::Decompressed(_) => panic!("raw archive must yield Borrowed"),
        }
    }

    #[test]
    fn iter_borrowed_decompresses_zstd_payload() {
        let big = "あ".repeat(2_000);
        let arc = roundtrip(FLAG_ZSTD | FLAG_UTF8, &[("big.txt", big.as_bytes())]);
        let items: Vec<_> = arc.iter_borrowed().filter_map(Result::ok).collect();
        let (label, payload) = &items[0];
        assert_eq!(*label, "big.txt");
        match payload {
            ArchivePayload::Decompressed(v) => assert_eq!(v, big.as_bytes()),
            ArchivePayload::Borrowed(_) => panic!("zstd archive must yield Decompressed"),
        }
    }

    #[test]
    fn archive_payload_as_bytes_is_uniform_across_variants() {
        let body = b"some bytes".to_vec();
        let raw_payload = ArchivePayload::Borrowed(&body);
        let owned_payload = ArchivePayload::Decompressed(body.clone());
        assert_eq!(raw_payload.as_bytes(), owned_payload.as_bytes());
        assert_eq!(raw_payload.len(), 10);
        assert!(!raw_payload.is_empty());
    }

    #[test]
    fn archive_payload_into_owned_returns_vec_for_both_variants() {
        let raw_payload = ArchivePayload::Borrowed(b"abc");
        let owned_payload = ArchivePayload::Decompressed(b"def".to_vec());
        assert_eq!(raw_payload.into_owned(), b"abc");
        assert_eq!(owned_payload.into_owned(), b"def");
    }

    #[test]
    fn iter_and_iter_borrowed_yield_byte_equal_outputs() {
        // Owned `iter` is now built on top of `iter_borrowed`; a
        // round-trip equality check pins that the convenience wrapper
        // produces byte-for-byte the same payload.
        let arc = roundtrip(
            FLAG_UTF8,
            &[("a.txt", "あおい".as_bytes()), ("b.txt", "そら".as_bytes())],
        );
        let owned: Vec<_> = arc.iter().filter_map(Result::ok).collect();
        let borrowed: Vec<_> = arc
            .iter_borrowed()
            .filter_map(Result::ok)
            .map(|(label, payload)| (label.to_owned(), payload.into_owned()))
            .collect();
        assert_eq!(owned, borrowed);
    }

    #[test]
    fn empty_archive_roundtrips() {
        let arc = roundtrip(0, &[]);
        assert_eq!(arc.len(), 0);
        assert!(arc.is_empty());
        assert!(arc.iter().next().is_none());
    }

    #[test]
    fn bad_magic_rejected() {
        let buf = vec![b'X'; 32];
        match Archive::from_bytes(buf) {
            Err(ArchiveError::BadMagic) => {}
            other => panic!("expected BadMagic, got {other:?}"),
        }
    }

    #[test]
    fn unsupported_version_rejected() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&MAGIC);
        buf.extend_from_slice(&999u32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        match Archive::from_bytes(buf) {
            Err(ArchiveError::UnsupportedVersion(999)) => {}
            other => panic!("expected UnsupportedVersion(999), got {other:?}"),
        }
    }

    #[test]
    fn truncated_header_rejected() {
        let buf = vec![b'A', b'O']; // less than 16 bytes
        match Archive::from_bytes(buf) {
            Err(ArchiveError::Truncated { what: "header" }) => {}
            other => panic!("expected Truncated header, got {other:?}"),
        }
    }

    #[test]
    fn entry_metadata_records_blake3() {
        let arc = roundtrip(0, &[("a.txt", b"hello world")]);
        let expected: [u8; 32] = blake3::hash(b"hello world").into();
        assert_eq!(arc.entries()[0].source_blake3, expected);
    }

    #[test]
    fn entry_metadata_records_mtime() {
        // Build manually so we control mtime.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("corpus.aozc");
        let mut b = ArchiveBuilder::new(0);
        b.push_entry("a.txt", b"hi", 12_345_678_900).unwrap();
        b.finish(&path).unwrap();
        let arc = Archive::open(&path).unwrap();
        assert_eq!(arc.entries()[0].source_mtime_ns, 12_345_678_900);
    }

    #[test]
    fn push_prebuilt_carries_through_to_iter() {
        // First archive: real entry with zstd.
        let dir = tempfile::tempdir().expect("tempdir");
        let path1 = dir.path().join("a.aozc");
        let path2 = dir.path().join("b.aozc");
        let body = "あ".repeat(2000);

        let mut b1 = ArchiveBuilder::new(FLAG_ZSTD | FLAG_UTF8);
        b1.push_entry("doc.txt", body.as_bytes(), 1).unwrap();
        b1.finish(&path1).unwrap();
        let arc1 = Archive::open(&path1).unwrap();

        // Second archive: copy the prebuilt entry verbatim into a
        // fresh builder (the incremental-pack code path).
        let mut b2 = ArchiveBuilder::new(FLAG_ZSTD | FLAG_UTF8);
        let meta = arc1.entries()[0].clone();
        let raw = arc1.raw_payload(0).to_vec();
        b2.push_prebuilt(meta, &raw);
        b2.finish(&path2).unwrap();

        let arc2 = Archive::open(&path2).unwrap();
        assert_eq!(
            arc2.entries()[0].source_blake3,
            arc1.entries()[0].source_blake3
        );
        let items: Vec<_> = arc2.iter().filter_map(Result::ok).collect();
        assert_eq!(items[0].0, "doc.txt");
        assert_eq!(items[0].1, body.as_bytes());
    }

    #[test]
    fn system_time_ns_roundtrip() {
        use core::convert::Infallible;
        let now = SystemTime::now();
        let ns = system_time_to_ns(now);
        let back = ns_to_system_time(ns);
        let drift = back
            .duration_since(now)
            .or_else(|e| Ok::<_, Infallible>(e.duration()))
            .unwrap();
        assert!(drift.as_nanos() < 100, "round-trip drift > 100 ns");
    }
}
