//! Corpus source abstraction for aozora parser sweep tests.

#![forbid(unsafe_code)]

use std::env;
use std::path::PathBuf;

pub mod archive;
mod error;
mod filesystem;
mod in_memory;
mod parallel;
mod vendored;

pub use archive::{Archive, ArchiveBuilder, ArchiveError, ArchivePayload, EntryMeta};
pub use error::CorpusError;
pub use filesystem::FilesystemCorpus;
pub use in_memory::InMemoryCorpus;
pub use parallel::{par_load_decoded, with_load_pool};
pub use vendored::VendoredCorpus;

/// Environment variable name consulted by [`from_env`].
pub const ENV_CORPUS_ROOT: &str = "AOZORA_CORPUS_ROOT";

/// A single candidate text for sweep invariants to check.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CorpusItem {
    /// Human-readable diagnostic label, not a stable key.
    pub label: String,
    /// Raw file content in its original encoding (typically Shift_JIS).
    /// Decoding is the caller's responsibility.
    pub bytes: Vec<u8>,
}

impl CorpusItem {
    /// Construct an item.
    #[must_use]
    pub fn new(label: impl Into<String>, bytes: Vec<u8>) -> Self {
        Self {
            label: label.into(),
            bytes,
        }
    }
}

/// Source of candidate texts for parser sweep tests.
pub trait CorpusSource: Send + Sync {
    /// Iterate candidate texts in implementation-defined order.
    /// Per-item errors (for example an unreadable file in a filesystem walk) are yielded inline so the caller can choose to skip, log, or fail fast.
    fn iter(&self) -> Box<dyn Iterator<Item = Result<CorpusItem, CorpusError>> + '_>;

    /// Human-readable provenance label for diagnostics.
    fn provenance(&self) -> &str;
}

/// Construct a [`FilesystemCorpus`] from [`ENV_CORPUS_ROOT`].
///
/// Returns [`None`] if the variable is unset, the path is not a directory, or construction fails.
#[must_use]
pub fn from_env() -> Option<Box<dyn CorpusSource>> {
    from_path(env::var_os(ENV_CORPUS_ROOT)?)
}

fn from_path(raw: impl Into<PathBuf>) -> Option<Box<dyn CorpusSource>> {
    let path = raw.into();
    if !path.is_dir() {
        return None;
    }
    let concrete = FilesystemCorpus::new(path).ok()?;
    Some(Box::new(concrete))
}

#[cfg(test)]
mod tests {
    use core::fmt;
    use std::fs;

    use super::*;

    #[test]
    fn corpus_item_preserves_label_and_bytes() {
        let item = CorpusItem::new("case-1", vec![0xEF, 0xBB, 0xBF, b'X']);
        assert_eq!(item.label, "case-1");
        assert_eq!(item.bytes, vec![0xEF, 0xBB, 0xBF, b'X']);
    }

    #[test]
    fn corpus_item_accepts_string_owned_label() {
        let owned: String = "owned".to_owned();
        let item = CorpusItem::new(owned, Vec::new());
        assert_eq!(item.label, "owned");
        assert_eq!(item.bytes, Vec::<u8>::new());
    }

    #[test]
    fn corpus_item_is_debug_and_clone() {
        fn assert_debug_clone<T: fmt::Debug + Clone>() {}
        assert_debug_clone::<CorpusItem>();
    }

    #[test]
    fn env_constant_matches_documented_name() {
        assert_eq!(ENV_CORPUS_ROOT, "AOZORA_CORPUS_ROOT");
    }

    #[test]
    fn from_path_rejects_nonexistent_path() {
        assert!(from_path("/absolutely/not/a/real/path").is_none());
    }

    #[test]
    fn from_path_rejects_file_path() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let file = dir.path().join("not-a-dir.txt");
        fs::write(&file, b"").expect("write file");
        assert!(from_path(&file).is_none());
    }

    #[test]
    fn from_path_accepts_valid_directory() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let source = from_path(dir.path()).expect("valid dir yields source");
        assert!(source.provenance().starts_with("filesystem:"));
    }
}
