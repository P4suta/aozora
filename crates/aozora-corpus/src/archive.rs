#![expect(
    clippy::expect_used,
    reason = "archive construction validates sizes, offsets, and fixture metadata before access"
)]

//! Loads indexed corpus archives through [`Archive`] and writes them through [`ArchiveBuilder`].

use core::error;
use core::fmt;
use core::str;
use std::fs;
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::CorpusError;

/// Magic bytes at the start of every archive.
pub const MAGIC: [u8; 4] = *b"AOZC";

/// Current archive format version.
pub const FORMAT_VERSION: u32 = 1;

/// Flag bit indicating per-entry zstd compression.
pub const FLAG_ZSTD: u32 = 1 << 0;

/// Flag bit indicating the payload is pre-decoded UTF-8 text.
/// Without this flag, payloads contain raw `Shift_JIS` bytes.
pub const FLAG_UTF8: u32 = 1 << 1;

const SUPPORTED_FLAGS: u32 = FLAG_ZSTD | FLAG_UTF8;

const INDEX_FIXED_LEN: usize = 8 + 4 + 4 + 8 + 32 + 4;

const HEADER_LEN: usize = 4 + 4 + 4 + 4;
const VERSION_OFFSET: usize = 4;
const FLAGS_OFFSET: usize = 8;
const COUNT_OFFSET: usize = 12;
const PAYLOAD_LEN_OFFSET: usize = 8;
const DECODED_LEN_OFFSET: usize = 12;
const MTIME_OFFSET: usize = 16;
const HASH_OFFSET: usize = 24;
const LABEL_LEN_OFFSET: usize = 56;
const DECODE_CAPACITY_MULTIPLIER: usize = 4;
const DEFAULT_ZSTD_LEVEL: i32 = 19;
const COMPRESSED_CAPACITY_DIVISOR: usize = 4;
#[cfg(unix)]
const ARCHIVE_FILE_MODE: u32 = 0o666;
const MAX_SYMLINK_HOPS: usize = 40;

/// Per-entry decompressed size limit in bytes.
pub const MAX_DECODED_LEN_PER_ENTRY: u32 = 256 * 1024 * 1024;

/// Archive format and filesystem errors.
#[derive(Debug)]
#[non_exhaustive]
pub enum ArchiveError {
    /// A filesystem operation failed.
    Io(io::Error),
    /// The file has invalid magic bytes.
    BadMagic,
    /// The file uses an unsupported format version.
    UnsupportedVersion(u32),
    /// The header contains undefined flag bits.
    UnsupportedFlags(u32),
    /// File is shorter than the declared header / index / payload would require.
    Truncated {
        /// The truncated field or section.
        what: &'static str,
    },
    /// Per-entry zstd decompression failed.
    Decompress(io::Error),
    /// A label contains invalid UTF-8.
    BadLabel,
    /// A declared size or computed layout exceeds an archive bound.
    InvalidSize {
        /// The field exceeding its bound.
        what: &'static str,
        /// The unreasonable value, surfaced for diagnostics.
        value: u64,
    },
    /// A payload's declared length disagrees with its encoded data.
    LengthMismatch {
        /// Field whose declaration was inconsistent.
        what: &'static str,
        /// Length recorded in the archive index.
        declared: u64,
        /// Length derived from the payload bytes.
        actual: u64,
    },
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
            Self::UnsupportedFlags(flags) => {
                write!(f, "archive contains unsupported flags 0x{flags:08x}")
            }
            Self::Truncated { what } => write!(f, "archive truncated while reading {what}"),
            Self::Decompress(e) => write!(f, "zstd decompression failed: {e}"),
            Self::BadLabel => f.write_str("entry label is not valid UTF-8"),
            Self::InvalidSize { what, value } => {
                write!(f, "archive {what} is invalid ({value})")
            }
            Self::LengthMismatch {
                what,
                declared,
                actual,
            } => write!(
                f,
                "archive {what} length mismatch: declared {declared}, actual {actual}"
            ),
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

/// Metadata for an indexed archive entry.
#[derive(Debug, Clone)]
pub struct EntryMeta {
    /// Offset in bytes from the start of the archive file at which this entry's payload begins.
    pub payload_offset: u64,
    /// Number of payload bytes in the file.
    /// Equals `decoded_len` for uncompressed archives.
    pub payload_len: u32,
    /// Number of bytes after decompression.
    /// For raw archives this equals `payload_len`.
    pub decoded_len: u32,
    /// Source modification time in nanoseconds since the Unix epoch.
    pub source_mtime_ns: i64,
    /// Source content hash computed by `blake3`.
    pub source_blake3: [u8; 32],
    /// Entry label for diagnostics.
    pub label: String,
}

/// A corpus archive with its index and payload bytes in memory.
#[derive(Debug)]
pub struct Archive {
    flags: u32,
    entries: Vec<EntryMeta>,
    bytes: Vec<u8>,
}

impl Archive {
    /// Loads an archive and validates its header and index.
    ///
    /// # Errors
    ///
    /// Returns an error for filesystem failures, malformed metadata, or unsupported format versions or flags.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ArchiveError> {
        let bytes = fs::read(path)?;
        Self::from_bytes(bytes)
    }

    /// Parses an archive from an owned byte buffer.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed metadata or unsupported format versions or flags.
    #[expect(
        clippy::missing_panics_doc,
        clippy::too_many_lines,
        reason = "the format reader validates header and record bounds before fixed-width reads"
    )]
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, ArchiveError> {
        if bytes.len() < HEADER_LEN {
            return Err(ArchiveError::Truncated { what: "header" });
        }
        if bytes[..VERSION_OFFSET] != MAGIC {
            return Err(ArchiveError::BadMagic);
        }
        let version = u32::from_le_bytes(
            bytes[VERSION_OFFSET..FLAGS_OFFSET]
                .try_into()
                .expect("version bytes"),
        );
        if version != FORMAT_VERSION {
            return Err(ArchiveError::UnsupportedVersion(version));
        }
        let flags = u32::from_le_bytes(
            bytes[FLAGS_OFFSET..COUNT_OFFSET]
                .try_into()
                .expect("flags bytes"),
        );
        let unsupported_flags = flags & !SUPPORTED_FLAGS;
        if unsupported_flags != 0 {
            return Err(ArchiveError::UnsupportedFlags(unsupported_flags));
        }
        let count = u32::from_le_bytes(
            bytes[COUNT_OFFSET..HEADER_LEN]
                .try_into()
                .expect("count bytes"),
        ) as usize;

        let count_ceiling = bytes.len().saturating_sub(HEADER_LEN) / INDEX_FIXED_LEN;
        if count > count_ceiling {
            return Err(ArchiveError::InvalidSize {
                what: "entry count",
                value: count as u64,
            });
        }

        let mut entries = Vec::with_capacity(count);
        let mut cursor = HEADER_LEN;
        for _ in 0..count {
            let fixed_end =
                cursor
                    .checked_add(INDEX_FIXED_LEN)
                    .ok_or(ArchiveError::InvalidSize {
                        what: "index cursor",
                        value: cursor as u64,
                    })?;
            if fixed_end > bytes.len() {
                return Err(ArchiveError::Truncated {
                    what: "index entry header",
                });
            }
            let payload_offset = u64::from_le_bytes(
                bytes[cursor..cursor + PAYLOAD_LEN_OFFSET]
                    .try_into()
                    .expect("8-byte slice"),
            );
            let payload_len = u32::from_le_bytes(
                bytes[cursor + PAYLOAD_LEN_OFFSET..cursor + DECODED_LEN_OFFSET]
                    .try_into()
                    .expect("4-byte slice"),
            );
            let decoded_len = u32::from_le_bytes(
                bytes[cursor + DECODED_LEN_OFFSET..cursor + MTIME_OFFSET]
                    .try_into()
                    .expect("4-byte slice"),
            );
            if decoded_len > MAX_DECODED_LEN_PER_ENTRY {
                return Err(ArchiveError::InvalidSize {
                    what: "decoded_len",
                    value: u64::from(decoded_len),
                });
            }
            let source_mtime_ns = i64::from_le_bytes(
                bytes[cursor + MTIME_OFFSET..cursor + HASH_OFFSET]
                    .try_into()
                    .expect("8-byte slice"),
            );
            let mut source_blake3 = [0u8; 32];
            source_blake3.copy_from_slice(&bytes[cursor + HASH_OFFSET..cursor + LABEL_LEN_OFFSET]);
            let label_len = u32::from_le_bytes(
                bytes[cursor + LABEL_LEN_OFFSET..cursor + INDEX_FIXED_LEN]
                    .try_into()
                    .expect("4-byte slice"),
            ) as usize;
            cursor = fixed_end;
            let label_end = cursor
                .checked_add(label_len)
                .ok_or(ArchiveError::InvalidSize {
                    what: "label length",
                    value: label_len as u64,
                })?;
            if label_end > bytes.len() {
                return Err(ArchiveError::Truncated {
                    what: "index entry label",
                });
            }
            let label = str::from_utf8(&bytes[cursor..label_end])
                .map_err(|_| ArchiveError::BadLabel)?
                .to_owned();
            cursor = label_end;

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
            entries.push(EntryMeta {
                payload_offset,
                payload_len,
                decoded_len,
                source_mtime_ns,
                source_blake3,
                label,
            });
        }

        let payload_section_start =
            u64::try_from(cursor).map_err(|_| ArchiveError::InvalidSize {
                what: "payload section offset",
                value: u64::MAX,
            })?;
        let mut ranges = Vec::with_capacity(entries.len());
        for entry in &entries {
            if entry.payload_offset < payload_section_start {
                return Err(ArchiveError::InvalidSize {
                    what: "payload offset before payload section",
                    value: entry.payload_offset,
                });
            }
            if flags & FLAG_ZSTD == 0 && entry.payload_len != entry.decoded_len {
                return Err(ArchiveError::LengthMismatch {
                    what: "raw payload",
                    declared: u64::from(entry.decoded_len),
                    actual: u64::from(entry.payload_len),
                });
            }
            let end = entry
                .payload_offset
                .checked_add(u64::from(entry.payload_len))
                .ok_or(ArchiveError::InvalidSize {
                    what: "payload range",
                    value: entry.payload_offset,
                })?;
            ranges.push((entry.payload_offset, end));
        }
        ranges.sort_unstable();
        for pair in ranges.windows(2) {
            if pair[1].0 < pair[0].1 {
                return Err(ArchiveError::InvalidSize {
                    what: "overlapping payload range",
                    value: pair[1].0,
                });
            }
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

    /// Whether payloads contain UTF-8 instead of raw `Shift_JIS` bytes.
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

    /// Returns the archive flags.
    #[must_use]
    pub fn flags(&self) -> u32 {
        self.flags
    }

    /// Returns entry metadata without decompressing payloads.
    #[must_use]
    pub fn entries(&self) -> &[EntryMeta] {
        &self.entries
    }

    /// Returns the payload at `index` as borrowed raw bytes or owned decompressed bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if decompression fails or the decoded length differs from the index.
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
            if bytes.len() != entry.decoded_len as usize {
                return Err(ArchiveError::LengthMismatch {
                    what: "decoded payload",
                    declared: u64::from(entry.decoded_len),
                    actual: bytes.len() as u64,
                });
            }
            Ok(ArchivePayload::Decompressed(bytes))
        } else {
            Ok(ArchivePayload::Borrowed(raw))
        }
    }

    /// Borrows an entry's stored bytes without decompression.
    ///
    /// # Panics
    ///
    /// Panics if `index >= self.len()`.
    #[must_use]
    pub fn raw_payload(&self, index: usize) -> &[u8] {
        let entry = &self.entries[index];
        let start = usize::try_from(entry.payload_offset).expect("validated payload offset");
        let end = start + entry.payload_len as usize;
        &self.bytes[start..end]
    }

    /// Iterates entries as owned labels and decoded bytes.
    ///
    /// # Errors
    ///
    /// Yields an error for entries with invalid compressed data or decoded lengths and continues iteration.
    pub fn iter(&self) -> impl Iterator<Item = Result<(String, Vec<u8>), ArchiveError>> + '_ {
        self.iter_borrowed().map(|item| {
            let (label, payload) = item?;
            Ok((label.to_owned(), payload.into_owned()))
        })
    }

    /// Iterates borrowed labels and payloads, allocating only to decompress entries.
    ///
    /// # Errors
    ///
    /// Yields the same per-entry errors as [`Self::iter`].
    pub fn iter_borrowed(
        &self,
    ) -> impl Iterator<Item = Result<(&str, ArchivePayload<'_>), ArchiveError>> + '_ {
        self.entries.iter().enumerate().map(move |(i, entry)| {
            let payload = self.payload_at(i)?;
            Ok((entry.label.as_str(), payload))
        })
    }
}

/// Decoded payload returned by [`Archive::iter_borrowed`].
#[derive(Debug)]
pub enum ArchivePayload<'a> {
    /// Raw bytes borrowed from the archive.
    Borrowed(&'a [u8]),
    /// Freshly decompressed payload.
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

    /// Returns owned bytes, copying borrowed payloads.
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
    let initial_capacity = expected_len
        .min(src.len().saturating_mul(DECODE_CAPACITY_MULTIPLIER))
        .min(1024 * 1024);
    let mut out = Vec::new();
    out.try_reserve(initial_capacity)
        .map_err(|error| io::Error::other(error.to_string()))?;
    let decoder = zstd::stream::read::Decoder::new(src)?;
    let limit = u64::try_from(expected_len.min(MAX_DECODED_LEN_PER_ENTRY as usize))
        .unwrap_or_else(|_| u64::from(MAX_DECODED_LEN_PER_ENTRY))
        .saturating_add(1);
    let mut limited = decoder.take(limit);
    limited.read_to_end(&mut out)?;
    if out.len() as u64 > u64::from(MAX_DECODED_LEN_PER_ENTRY) {
        return Err(io::Error::other(
            "zstd payload expanded past per-entry decode budget",
        ));
    }
    Ok(out)
}

/// Converts a time to nanoseconds since the Unix epoch, saturating to `0..=i64::MAX`.
#[must_use]
pub fn system_time_to_ns(t: SystemTime) -> i64 {
    t.duration_since(SystemTime::UNIX_EPOCH).map_or_else(
        |_| 0,
        |d| {
            let ns = d.as_nanos();
            i64::try_from(ns.min(i64::MAX as u128)).unwrap_or(i64::MAX)
        },
    )
}

/// Converts nanoseconds since the Unix epoch to a time, treating negative values as zero.
#[must_use]
pub fn ns_to_system_time(ns: i64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_nanos(u64::try_from(ns).unwrap_or(0))
}

/// Collects entries for an archive written by [`Self::finish`].
#[derive(Debug)]
pub struct ArchiveBuilder {
    flags: u32,
    entries: Vec<EntryMeta>,
    payload: Vec<u8>,
    zstd_level: i32,
}

impl ArchiveBuilder {
    /// Creates an empty builder using any combination of [`FLAG_ZSTD`] and [`FLAG_UTF8`].
    #[must_use]
    pub fn new(flags: u32) -> Self {
        Self {
            flags,
            entries: Vec::new(),
            payload: Vec::new(),
            zstd_level: DEFAULT_ZSTD_LEVEL,
        }
    }

    /// Sets the compression level for [`FLAG_ZSTD`] archives.
    pub fn zstd_level(&mut self, level: i32) -> &mut Self {
        self.zstd_level = level;
        self
    }

    /// Adds uncompressed source bytes with their content hash and optional compression.
    /// `bytes` must contain UTF-8 for [`FLAG_UTF8`] archives and `Shift_JIS` otherwise.
    ///
    /// # Errors
    ///
    /// Returns an [`ArchiveError`] if the builder configuration or entry size is invalid, or if zstd encoding fails.
    pub fn push_entry(
        &mut self,
        label: impl Into<String>,
        bytes: &[u8],
        source_mtime_ns: i64,
    ) -> Result<(), ArchiveError> {
        validate_flags(self.flags)?;
        let label = label.into();
        validate_label_len(&label)?;
        let source_blake3: [u8; 32] = blake3::hash(bytes).into();
        let decoded_len = decoded_len(bytes.len())?;
        let payload_offset_in_section = payload_offset(self.payload.len())?;

        if self.flags & FLAG_ZSTD != 0 {
            use zstd::stream::copy_encode;
            let mut compressed = Vec::with_capacity(bytes.len() / COMPRESSED_CAPACITY_DIVISOR);
            copy_encode(bytes, &mut compressed, self.zstd_level)
                .map_err(ArchiveError::Decompress)?;
            let payload_len = payload_len(compressed.len())?;
            checked_payload_growth(self.payload.len(), compressed.len())?;
            self.payload.extend_from_slice(&compressed);
            self.entries.push(EntryMeta {
                payload_offset: payload_offset_in_section,
                payload_len,
                decoded_len,
                source_mtime_ns,
                source_blake3,
                label,
            });
        } else {
            checked_payload_growth(self.payload.len(), bytes.len())?;
            self.payload.extend_from_slice(bytes);
            self.entries.push(EntryMeta {
                payload_offset: payload_offset_in_section,
                payload_len: decoded_len,
                decoded_len,
                source_mtime_ns,
                source_blake3,
                label,
            });
        }
        Ok(())
    }

    /// Adds an encoded payload with caller-supplied source metadata.
    /// The payload's compression and encoding must match the builder flags.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid flags, labels, or lengths.
    #[expect(
        clippy::too_many_arguments,
        reason = "the existing encoded-entry API accepts source metadata without changing its public signature"
    )]
    pub fn push_already_encoded(
        &mut self,
        label: &str,
        payload: &[u8],
        decoded_len: u32,
        source_mtime_ns: i64,
        source_blake3: [u8; 32],
    ) -> Result<(), ArchiveError> {
        validate_flags(self.flags)?;
        validate_label_len(label)?;
        validate_decoded_len(decoded_len)?;
        let payload_offset_in_section = payload_offset(self.payload.len())?;
        let payload_len = payload_len(payload.len())?;
        validate_raw_lengths(self.flags, payload_len, decoded_len)?;
        checked_payload_growth(self.payload.len(), payload.len())?;
        self.payload.extend_from_slice(payload);
        self.entries.push(EntryMeta {
            payload_offset: payload_offset_in_section,
            payload_len,
            decoded_len,
            source_mtime_ns,
            source_blake3,
            label: label.to_owned(),
        });
        Ok(())
    }

    /// Adds an encoded payload and existing metadata without recompression.
    /// The payload's compression and encoding must match the builder flags.
    ///
    /// # Errors
    ///
    /// Returns an [`ArchiveError`] if the builder configuration or metadata is invalid, or the payload length disagrees with the metadata.
    pub fn push_prebuilt(&mut self, meta: EntryMeta, payload: &[u8]) -> Result<(), ArchiveError> {
        validate_flags(self.flags)?;
        validate_label_len(&meta.label)?;
        validate_decoded_len(meta.decoded_len)?;
        let actual_payload_len = payload_len(payload.len())?;
        if actual_payload_len != meta.payload_len {
            return Err(ArchiveError::LengthMismatch {
                what: "prebuilt payload",
                declared: u64::from(meta.payload_len),
                actual: u64::from(actual_payload_len),
            });
        }
        validate_raw_lengths(self.flags, meta.payload_len, meta.decoded_len)?;
        let payload_offset_in_section = payload_offset(self.payload.len())?;
        checked_payload_growth(self.payload.len(), payload.len())?;
        self.payload.extend_from_slice(payload);
        self.entries.push(EntryMeta {
            payload_offset: payload_offset_in_section,
            ..meta
        });
        Ok(())
    }

    /// Writes the archive by atomic replacement and returns its size in bytes.
    ///
    /// # Errors
    ///
    /// Returns an [`ArchiveError`] if the archive is invalid or a filesystem operation fails.
    #[expect(
        clippy::too_many_lines,
        reason = "archive layout validation and atomic replacement remain one finalization operation"
    )]
    pub fn finish(self, path: impl AsRef<Path>) -> Result<u64, ArchiveError> {
        let path = archive_destination(path.as_ref())?;
        validate_flags(self.flags)?;
        let entry_count =
            u32::try_from(self.entries.len()).map_err(|_| ArchiveError::InvalidSize {
                what: "entry count",
                value: self.entries.len() as u64,
            })?;

        let mut index_section_len = 0usize;
        for entry in &self.entries {
            validate_label_len(&entry.label)?;
            validate_decoded_len(entry.decoded_len)?;
            validate_raw_lengths(self.flags, entry.payload_len, entry.decoded_len)?;
            let payload_end = entry
                .payload_offset
                .checked_add(u64::from(entry.payload_len))
                .ok_or(ArchiveError::InvalidSize {
                    what: "payload range",
                    value: entry.payload_offset,
                })?;
            if payload_end > self.payload.len() as u64 {
                return Err(ArchiveError::InvalidSize {
                    what: "payload range",
                    value: payload_end,
                });
            }
            let record_len = INDEX_FIXED_LEN.checked_add(entry.label.len()).ok_or(
                ArchiveError::InvalidSize {
                    what: "index entry length",
                    value: entry.label.len() as u64,
                },
            )?;
            index_section_len =
                index_section_len
                    .checked_add(record_len)
                    .ok_or(ArchiveError::InvalidSize {
                        what: "index section length",
                        value: self.entries.len() as u64,
                    })?;
        }
        let payload_section_offset =
            HEADER_LEN
                .checked_add(index_section_len)
                .ok_or(ArchiveError::InvalidSize {
                    what: "payload section offset",
                    value: index_section_len as u64,
                })?;

        let total_len = payload_section_offset
            .checked_add(self.payload.len())
            .ok_or(ArchiveError::InvalidSize {
                what: "archive length",
                value: self.payload.len() as u64,
            })?;
        let total_len_u64 = u64::try_from(total_len).map_err(|_| ArchiveError::InvalidSize {
            what: "archive length",
            value: u64::MAX,
        })?;
        let mut out = Vec::new();
        out.try_reserve_exact(total_len)
            .map_err(|_| ArchiveError::InvalidSize {
                what: "archive allocation",
                value: total_len_u64,
            })?;

        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        out.extend_from_slice(&self.flags.to_le_bytes());
        out.extend_from_slice(&entry_count.to_le_bytes());

        for entry in &self.entries {
            let section_offset =
                u64::try_from(payload_section_offset).map_err(|_| ArchiveError::InvalidSize {
                    what: "payload section offset",
                    value: u64::MAX,
                })?;
            let absolute_offset = section_offset.checked_add(entry.payload_offset).ok_or(
                ArchiveError::InvalidSize {
                    what: "payload offset",
                    value: entry.payload_offset,
                },
            )?;
            out.extend_from_slice(&absolute_offset.to_le_bytes());
            out.extend_from_slice(&entry.payload_len.to_le_bytes());
            out.extend_from_slice(&entry.decoded_len.to_le_bytes());
            out.extend_from_slice(&entry.source_mtime_ns.to_le_bytes());
            out.extend_from_slice(&entry.source_blake3);
            let label_len_u32 =
                u32::try_from(entry.label.len()).map_err(|_| ArchiveError::InvalidSize {
                    what: "label length",
                    value: entry.label.len() as u64,
                })?;
            out.extend_from_slice(&label_len_u32.to_le_bytes());
            out.extend_from_slice(entry.label.as_bytes());
        }

        out.extend_from_slice(&self.payload);
        debug_assert_eq!(out.len(), total_len, "archive layout invariant");

        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let permissions = match fs::metadata(&path) {
            Ok(metadata) => Some(metadata.permissions()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(ArchiveError::Io(error)),
        };
        let mut builder = tempfile::Builder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            builder.permissions(fs::Permissions::from_mode(ARCHIVE_FILE_MODE))
        };
        let mut temporary = builder.tempfile_in(parent)?;
        if let Some(permissions) = permissions {
            temporary.as_file().set_permissions(permissions)?;
        }
        temporary.write_all(&out)?;
        temporary.as_file().sync_all()?;
        temporary
            .persist(&path)
            .map_err(|error| ArchiveError::Io(error.error))?;
        sync_directory(parent)?;
        Ok(total_len_u64)
    }
}

fn archive_destination(path: &Path) -> io::Result<PathBuf> {
    let mut current = path.to_path_buf();
    for _ in 0..MAX_SYMLINK_HOPS {
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let target = fs::read_link(&current)?;
                current = if target.is_absolute() {
                    target
                } else {
                    current
                        .parent()
                        .filter(|parent| !parent.as_os_str().is_empty())
                        .unwrap_or_else(|| Path::new("."))
                        .join(target)
                };
            }
            Ok(_) => return Ok(current),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(current),
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "symbolic link chain is too deep",
    ))
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    fs::File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

fn validate_flags(flags: u32) -> Result<(), ArchiveError> {
    let unsupported = flags & !SUPPORTED_FLAGS;
    if unsupported == 0 {
        Ok(())
    } else {
        Err(ArchiveError::UnsupportedFlags(unsupported))
    }
}

fn validate_label_len(label: &str) -> Result<(), ArchiveError> {
    u32::try_from(label.len())
        .map(|_| ())
        .map_err(|_| ArchiveError::InvalidSize {
            what: "label length",
            value: label.len() as u64,
        })
}

fn validate_decoded_len(len: u32) -> Result<(), ArchiveError> {
    if len <= MAX_DECODED_LEN_PER_ENTRY {
        Ok(())
    } else {
        Err(ArchiveError::InvalidSize {
            what: "decoded_len",
            value: u64::from(len),
        })
    }
}

fn decoded_len(len: usize) -> Result<u32, ArchiveError> {
    let len = u32::try_from(len).map_err(|_| ArchiveError::InvalidSize {
        what: "decoded_len",
        value: len as u64,
    })?;
    validate_decoded_len(len)?;
    Ok(len)
}

fn payload_len(len: usize) -> Result<u32, ArchiveError> {
    u32::try_from(len).map_err(|_| ArchiveError::InvalidSize {
        what: "payload_len",
        value: len as u64,
    })
}

fn payload_offset(len: usize) -> Result<u64, ArchiveError> {
    u64::try_from(len).map_err(|_| ArchiveError::InvalidSize {
        what: "payload offset",
        value: u64::MAX,
    })
}

fn checked_payload_growth(current: usize, additional: usize) -> Result<(), ArchiveError> {
    current
        .checked_add(additional)
        .map(|_| ())
        .ok_or(ArchiveError::InvalidSize {
            what: "payload section length",
            value: additional as u64,
        })
}

fn validate_raw_lengths(
    flags: u32,
    payload_len: u32,
    decoded_len: u32,
) -> Result<(), ArchiveError> {
    if flags & FLAG_ZSTD != 0 || payload_len == decoded_len {
        Ok(())
    } else {
        Err(ArchiveError::LengthMismatch {
            what: "raw payload",
            declared: u64::from(decoded_len),
            actual: u64::from(payload_len),
        })
    }
}

#[cfg(test)]
mod tests {
    use core::ptr;
    use std::sync::{Arc, Barrier};
    use std::thread;

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
        let big = "あ".repeat(10_000);
        let arc = roundtrip(FLAG_ZSTD | FLAG_UTF8, &[("big.txt", big.as_bytes())]);
        assert!(arc.is_zstd());
        assert!(arc.is_utf8());
        let raw_payload = arc.raw_payload(0);
        let entry = &arc.entries()[0];
        assert!(
            raw_payload.len() < entry.decoded_len as usize,
            "zstd payload should be smaller than decoded length"
        );
        let items: Vec<_> = arc.iter().filter_map(Result::ok).collect();
        assert_eq!(items[0].1, big.as_bytes());
    }

    #[test]
    fn iter_borrowed_returns_zero_copy_slice_for_raw_archive() {
        let arc = roundtrip(0, &[("a.txt", b"hello world")]);
        let items: Vec<_> = arc.iter_borrowed().filter_map(Result::ok).collect();
        assert_eq!(items.len(), 1);
        let (label, payload) = &items[0];
        assert_eq!(*label, "a.txt");
        match payload {
            ArchivePayload::Borrowed(slice) => {
                assert_eq!(*slice, b"hello world");
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
    fn unsupported_flags_are_rejected() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&MAGIC);
        buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        buf.extend_from_slice(&(1u32 << 31).to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        assert!(matches!(
            Archive::from_bytes(buf),
            Err(ArchiveError::UnsupportedFlags(flags)) if flags == 1 << 31
        ));
    }

    #[test]
    fn raw_payload_length_must_match_decoded_length() {
        let arc = roundtrip(0, &[("a.txt", b"hello")]);
        let mut bytes = arc.bytes;
        bytes[HEADER_LEN + 12..HEADER_LEN + 16].copy_from_slice(&6u32.to_le_bytes());
        assert!(matches!(
            Archive::from_bytes(bytes),
            Err(ArchiveError::LengthMismatch {
                what: "raw payload",
                ..
            })
        ));
    }

    #[test]
    fn payload_must_begin_after_the_complete_index() {
        let arc = roundtrip(0, &[("a.txt", b"hello")]);
        let mut bytes = arc.bytes;
        bytes[HEADER_LEN..HEADER_LEN + 8].copy_from_slice(&0u64.to_le_bytes());
        assert!(matches!(
            Archive::from_bytes(bytes),
            Err(ArchiveError::InvalidSize {
                what: "payload offset before payload section",
                ..
            })
        ));
    }

    #[test]
    fn payload_ranges_must_not_overlap() {
        let arc = roundtrip(0, &[("a.txt", b"hello"), ("b.txt", b"world")]);
        let mut bytes = arc.bytes;
        let first_offset = bytes[HEADER_LEN..HEADER_LEN + 8].to_vec();
        let second = HEADER_LEN + INDEX_FIXED_LEN + "a.txt".len();
        bytes[second..second + 8].copy_from_slice(&first_offset);
        assert!(matches!(
            Archive::from_bytes(bytes),
            Err(ArchiveError::InvalidSize {
                what: "overlapping payload range",
                ..
            })
        ));
    }

    #[test]
    fn zstd_payload_must_match_declared_decoded_length() {
        let arc = roundtrip(FLAG_ZSTD, &[("a.txt", b"hello")]);
        let mut bytes = arc.bytes;
        bytes[HEADER_LEN + 12..HEADER_LEN + 16].copy_from_slice(&6u32.to_le_bytes());
        let malformed = Archive::from_bytes(bytes).expect("index shape remains valid");
        assert!(matches!(
            malformed.payload_at(0),
            Err(ArchiveError::LengthMismatch {
                what: "decoded payload",
                declared: 6,
                actual: 5
            })
        ));
    }

    #[test]
    fn truncated_header_rejected() {
        let buf = vec![b'A', b'O'];
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
        let dir = tempfile::tempdir().expect("tempdir");
        let path1 = dir.path().join("a.aozc");
        let path2 = dir.path().join("b.aozc");
        let body = "あ".repeat(2000);

        let mut b1 = ArchiveBuilder::new(FLAG_ZSTD | FLAG_UTF8);
        b1.push_entry("doc.txt", body.as_bytes(), 1).unwrap();
        b1.finish(&path1).unwrap();
        let arc1 = Archive::open(&path1).unwrap();

        let mut b2 = ArchiveBuilder::new(FLAG_ZSTD | FLAG_UTF8);
        let meta = arc1.entries()[0].clone();
        let raw = arc1.raw_payload(0).to_vec();
        b2.push_prebuilt(meta, &raw).unwrap();
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
    fn prebuilt_payload_length_mismatch_is_an_error() {
        let mut builder = ArchiveBuilder::new(FLAG_ZSTD);
        let meta = EntryMeta {
            payload_offset: 0,
            payload_len: 2,
            decoded_len: 1,
            source_mtime_ns: 0,
            source_blake3: [0; 32],
            label: "a.txt".to_owned(),
        };
        assert!(matches!(
            builder.push_prebuilt(meta, b"x"),
            Err(ArchiveError::LengthMismatch {
                what: "prebuilt payload",
                ..
            })
        ));
    }

    #[test]
    fn builder_rejects_unsupported_flags() {
        let builder = ArchiveBuilder::new(1 << 31);
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(matches!(
            builder.finish(dir.path().join("bad.aozc")),
            Err(ArchiveError::UnsupportedFlags(flags)) if flags == 1 << 31
        ));
    }

    #[test]
    fn rejects_unreasonable_entry_count() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&MAGIC);
        buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&u32::MAX.to_le_bytes());
        match Archive::from_bytes(buf) {
            Err(ArchiveError::InvalidSize { what, value }) => {
                assert_eq!(what, "entry count");
                assert_eq!(value, u64::from(u32::MAX));
            }
            other => panic!("expected InvalidSize entry count, got {other:?}"),
        }
    }

    #[test]
    fn accepts_count_exactly_at_ceiling_then_fails_on_record() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&MAGIC);
        buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&1u32.to_le_bytes());
        let payload_offset = (HEADER_LEN + INDEX_FIXED_LEN) as u64;
        buf.extend_from_slice(&payload_offset.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&0i64.to_le_bytes());
        buf.extend_from_slice(&[0u8; 32]);
        buf.extend_from_slice(&0u32.to_le_bytes());
        let arc = Archive::from_bytes(buf).expect("valid count-at-ceiling archive");
        assert_eq!(arc.len(), 1);
        assert_eq!(arc.entries()[0].label, "");
        assert_eq!(arc.entries()[0].payload_len, 0);
    }

    #[test]
    fn rejects_unreasonable_decoded_len_at_parse_time() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&MAGIC);
        buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        buf.extend_from_slice(&FLAG_ZSTD.to_le_bytes());
        buf.extend_from_slice(&1u32.to_le_bytes());
        buf.extend_from_slice(&0u64.to_le_bytes());
        buf.extend_from_slice(&10u32.to_le_bytes());
        buf.extend_from_slice(&(1u32 << 30).to_le_bytes());
        buf.extend_from_slice(&0i64.to_le_bytes());
        buf.extend_from_slice(&[0u8; 32]);
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.resize(buf.len() + 100, 0);
        match Archive::from_bytes(buf) {
            Err(ArchiveError::InvalidSize { what, value }) => {
                assert_eq!(what, "decoded_len");
                assert_eq!(value, u64::from(1u32 << 30));
            }
            other => panic!("expected InvalidSize decoded_len, got {other:?}"),
        }
    }

    #[test]
    fn decoded_len_at_max_passes_one_over_fails() {
        for (value, should_pass) in [
            (MAX_DECODED_LEN_PER_ENTRY, true),
            (MAX_DECODED_LEN_PER_ENTRY + 1, false),
        ] {
            let mut buf = Vec::new();
            buf.extend_from_slice(&MAGIC);
            buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
            buf.extend_from_slice(&FLAG_ZSTD.to_le_bytes());
            buf.extend_from_slice(&1u32.to_le_bytes());
            let payload_offset = (HEADER_LEN + INDEX_FIXED_LEN) as u64;
            buf.extend_from_slice(&payload_offset.to_le_bytes());
            buf.extend_from_slice(&0u32.to_le_bytes());
            buf.extend_from_slice(&value.to_le_bytes());
            buf.extend_from_slice(&0i64.to_le_bytes());
            buf.extend_from_slice(&[0u8; 32]);
            buf.extend_from_slice(&0u32.to_le_bytes());
            let result = Archive::from_bytes(buf);
            assert_eq!(
                result.is_ok(),
                should_pass,
                "decoded_len={value} should_pass={should_pass}, got {result:?}",
            );
        }
    }

    #[test]
    fn decompress_payload_clamps_giant_expected_len() {
        let empty_input: &[u8] = &[];
        let empty: Vec<u8> =
            zstd::encode_all(io::Cursor::new(empty_input), 1).expect("zstd encode");
        let out = decompress_payload(&empty, usize::MAX).expect("decompress empty");
        assert!(out.is_empty(), "empty stream must decompress to empty");
    }

    #[test]
    fn decompress_payload_stops_after_declared_length_mismatch() {
        let compressed =
            zstd::encode_all(io::Cursor::new(vec![b'x'; 1024 * 1024]), 1).expect("zstd encode");

        let out = decompress_payload(&compressed, 0).expect("decompress prefix");

        assert_eq!(out, b"x");
    }

    #[test]
    fn finish_produces_a_readable_archive() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("corpus.aozc");
        let mut b = ArchiveBuilder::new(FLAG_UTF8);
        b.push_entry("a.txt", b"hello", 0).unwrap();
        b.finish(&path).unwrap();
        let arc = Archive::open(&path).unwrap();
        assert_eq!(arc.len(), 1);
        let payload = arc.payload_at(0).unwrap();
        assert_eq!(payload.as_bytes(), b"hello");
    }

    #[test]
    fn finish_does_not_leave_temporary_file_behind() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("corpus.aozc");
        let mut b = ArchiveBuilder::new(0);
        b.push_entry("a.txt", b"hi", 0).unwrap();
        b.finish(&path).unwrap();
        let count = fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(
            count,
            1,
            "expected only the archive in {:?}, got {} entries",
            dir.path(),
            count,
        );
    }

    #[cfg(unix)]
    #[test]
    fn finish_preserves_existing_archive_permissions() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("corpus.aozc");
        fs::write(&path, b"old").expect("seed archive");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).expect("set permissions");
        let mut builder = ArchiveBuilder::new(0);
        builder.push_entry("a.txt", b"new", 0).unwrap();

        builder.finish(&path).expect("finish");

        let mode = fs::metadata(path).expect("metadata").permissions().mode();
        assert_eq!(mode & 0o777, 0o640);
    }

    #[cfg(unix)]
    #[test]
    fn finish_replaces_the_target_of_an_existing_symlink() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("target.aozc");
        let link = dir.path().join("corpus.aozc");
        fs::write(&target, b"old").expect("seed target");
        symlink("target.aozc", &link).expect("create link");
        let mut builder = ArchiveBuilder::new(FLAG_UTF8);
        builder.push_entry("a.txt", b"new", 0).unwrap();

        builder.finish(&link).expect("finish");

        assert!(
            fs::symlink_metadata(&link)
                .expect("link metadata")
                .file_type()
                .is_symlink()
        );
        let archive = Archive::open(&target).expect("target archive");
        assert_eq!(archive.payload_at(0).unwrap().as_bytes(), b"new");
    }

    #[cfg(unix)]
    #[test]
    fn new_archive_uses_the_process_default_creation_mode() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().expect("tempdir");
        let reference = dir.path().join("reference");
        let path = dir.path().join("corpus.aozc");
        fs::File::create(&reference).expect("create reference");
        let builder = ArchiveBuilder::new(0);

        builder.finish(&path).expect("finish");

        let reference_mode = fs::metadata(reference)
            .expect("reference metadata")
            .permissions()
            .mode();
        let archive_mode = fs::metadata(path)
            .expect("archive metadata")
            .permissions()
            .mode();
        assert_eq!(archive_mode & 0o777, reference_mode & 0o777);
    }

    #[test]
    fn concurrent_finish_to_the_same_path_publishes_one_complete_archive() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("corpus.aozc");
        let barrier = Arc::new(Barrier::new(2));

        let threads: Vec<_> = b"ab"
            .iter()
            .copied()
            .map(|byte| {
                let path = path.clone();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    let payload = vec![byte; 4 * 1024 * 1024];
                    let mut builder = ArchiveBuilder::new(FLAG_UTF8);
                    builder.push_entry("entry.txt", &payload, 0)?;
                    barrier.wait();
                    builder.finish(path)
                })
            })
            .collect();

        for thread in threads {
            thread
                .join()
                .expect("finish thread panicked")
                .expect("finish failed");
        }

        let archive = Archive::open(&path).expect("final archive must be readable");
        let payload = archive.payload_at(0).expect("payload must be readable");
        assert_eq!(payload.len(), 4 * 1024 * 1024);
        assert!(
            payload.as_bytes().iter().all(|byte| *byte == b'a')
                || payload.as_bytes().iter().all(|byte| *byte == b'b')
        );
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn gatekeeper_archive_magic_is_aozc() {
        assert_eq!(
            MAGIC, *b"AOZC",
            "MAGIC bytes are part of the on-disk format; \
             changing them is a hard-incompatible bump",
        );
    }

    #[test]
    fn gatekeeper_format_version_is_one() {
        assert_eq!(
            FORMAT_VERSION, 1,
            "bumping FORMAT_VERSION must come with a CHANGELOG entry \
             AND a backward-compat parser arm",
        );
    }

    #[test]
    fn gatekeeper_header_layout_matches_the_documented_spec() {
        assert_eq!(HEADER_LEN, 16);
        assert_eq!(INDEX_FIXED_LEN, 60);
    }

    #[test]
    fn gatekeeper_flag_bits_are_pinned() {
        assert_eq!(FLAG_ZSTD, 0b01);
        assert_eq!(FLAG_UTF8, 0b10);
    }

    #[test]
    fn gatekeeper_max_decoded_len_is_256_mib() {
        assert_eq!(MAX_DECODED_LEN_PER_ENTRY, 256 * 1024 * 1024);
    }

    #[test]
    fn gatekeeper_archive_error_variant_inventory() {
        for err in [
            ArchiveError::Io(io::Error::other("x")),
            ArchiveError::BadMagic,
            ArchiveError::UnsupportedVersion(0),
            ArchiveError::Truncated { what: "x" },
            ArchiveError::Decompress(io::Error::other("x")),
            ArchiveError::BadLabel,
            ArchiveError::InvalidSize {
                what: "x",
                value: 0,
            },
        ] {
            let display = format!("{err}");
            assert!(!display.is_empty(), "empty Display for {err:?}");
        }
    }

    #[test]
    fn gatekeeper_format_constants_compose_to_expected_minimum_file_size() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&MAGIC);
        buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(buf.len(), HEADER_LEN);
        let arc = Archive::from_bytes(buf).expect("minimal archive parses");
        assert_eq!(arc.len(), 0);
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
