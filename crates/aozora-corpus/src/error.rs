//! Error types for corpus source operations.
//!
//! Kept in its own module so each `CorpusSource` impl (see
//! `in_memory.rs`, `filesystem.rs`, `vendored.rs`) can extend
//! [`CorpusError`] without touching the public-facing `lib.rs`.

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

    /// Constructor was given a path that does not exist or is not a
    /// directory. Construction is rejected eagerly rather than deferring
    /// to iteration time so the error surfaces where the mistake was
    /// made (the caller's configuration code, not deep inside a sweep
    /// loop).
    #[error("corpus root is not a directory: {path}")]
    RootNotDirectory { path: PathBuf },
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;
    use std::io::{Error, ErrorKind};

    use super::*;

    #[test]
    fn io_error_display_includes_path_and_cause() {
        let err = CorpusError::Io {
            path: PathBuf::from("/tmp/aozora-corpus-missing"),
            source: Error::from(ErrorKind::NotFound),
        };
        let display = format!("{err}");
        assert!(
            display.contains("/tmp/aozora-corpus-missing"),
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

    #[test]
    fn root_not_directory_display_includes_path() {
        let err = CorpusError::RootNotDirectory {
            path: PathBuf::from("/does/not/exist"),
        };
        let display = format!("{err}");
        assert!(
            display.contains("/does/not/exist"),
            "display should mention the offending path: {display}"
        );
    }

    #[test]
    fn root_not_directory_has_no_source_chain() {
        let err = CorpusError::RootNotDirectory {
            path: PathBuf::from("/irrelevant"),
        };
        // Structural variant — the offending path is self-contained, there's
        // no underlying cause to forward.
        assert!(err.source().is_none());
    }
}
