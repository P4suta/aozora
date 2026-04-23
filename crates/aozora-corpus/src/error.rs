//! Error types for corpus source operations.
//!
//! Kept in its own module so that future impls in sibling files (M2-S2)
//! can extend [`CorpusError`] without touching the public-facing `lib.rs`.

use std::io;
use std::path::PathBuf;

/// Errors that may arise while iterating a corpus source.
///
/// Marked `#[non_exhaustive]` so additional variants (missing root,
/// malformed file layout, etc.) can be added in follow-up milestones
/// without breaking downstream `match` sites.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CorpusError {
    /// An I/O call failed on a specific path. The underlying
    /// [`io::Error`] is preserved as `source` so callers can inspect
    /// [`io::ErrorKind`] when they care (e.g. to distinguish `NotFound`
    /// from `PermissionDenied`).
    #[error("failed to read corpus item at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;
    use std::io::{Error, ErrorKind};

    use super::*;

    #[test]
    fn io_error_display_includes_path_and_cause() {
        let err = CorpusError::Io {
            path: PathBuf::from("/tmp/afm-corpus-missing"),
            source: Error::from(ErrorKind::NotFound),
        };
        let display = format!("{err}");
        assert!(
            display.contains("/tmp/afm-corpus-missing"),
            "display should mention path: {display}"
        );
    }

    #[test]
    fn io_error_exposes_source_chain() {
        let err = CorpusError::Io {
            path: PathBuf::from("x"),
            source: Error::from(ErrorKind::PermissionDenied),
        };
        let source = err.source().expect("Io variant should expose source");
        assert!(
            source.to_string().to_lowercase().contains("permission"),
            "source should be the io::Error: {source}"
        );
    }
}
