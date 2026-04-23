//! Corpus source abstraction for afm parser sweep tests.
//!
//! A [`CorpusSource`] yields a stream of candidate input texts for
//! property-based sweep tests: each item carries raw bytes plus a
//! human-readable label used only for diagnostics. Implementations choose
//! where the bytes come from — an in-memory literal, a vendored fixture
//! directory, or a filesystem root supplied by the developer via the
//! `AFM_CORPUS_ROOT` environment variable.
//!
//! See ADR-0007 for the design rationale. Key design points:
//!
//! - **Metadata-free items.** No titles, no tiers, no SHA256. The sweep
//!   harness checks invariants (no panic, no leaked markers, well-formed
//!   output, round-trip stability); none of those depend on *what* the
//!   input is, only that it is some aozora-format text.
//! - **No lockfile.** The set of inputs is whatever the caller provides.
//!   Pinning a specific upstream corpus is explicitly rejected: it would
//!   mandate a particular content set on every contributor and conflate
//!   "golden ground-truth" with "stress-test volume".
//! - **Opt-in via environment.** With `AFM_CORPUS_ROOT` unset, sweep tests
//!   runtime-skip; they never hard-fail on missing corpus.
//!
//! Golden fixtures (exact expected-HTML checks for canonical works like
//! 罪と罰) live elsewhere (`spec/aozora/fixtures/`) and are not corpus
//! concerns.

#![forbid(unsafe_code)]

mod error;

pub use error::CorpusError;

/// A single candidate text for sweep invariants to check.
///
/// `bytes` is the file content as read from its source, in its original
/// encoding (typically Shift_JIS for aozora-format texts). Encoding
/// detection and decoding is the caller's responsibility (see
/// `afm-encoding`).
///
/// `label` is a human-readable identifier used only in diagnostic output
/// when an invariant fails. For filesystem sources this is conventionally
/// the path relative to the corpus root; for in-memory sources it is any
/// caller-chosen string.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CorpusItem {
    pub label: String,
    pub bytes: Vec<u8>,
}

impl CorpusItem {
    /// Construct an item. `label` is borrowed into an owned [`String`] so
    /// callers can pass `&str` or `String` transparently.
    #[must_use]
    pub fn new(label: impl Into<String>, bytes: Vec<u8>) -> Self {
        Self {
            label: label.into(),
            bytes,
        }
    }
}

/// Source of candidate texts for parser sweep tests.
///
/// Implementations are the only place `std::fs` and environment-variable
/// access appear in the test infrastructure. Downstream sweep harnesses
/// consume a `Box<dyn CorpusSource>` and remain I/O-free.
///
/// The `Send + Sync` bounds let callers wrap a source in `Arc` for
/// parallel sweep. Implementations must uphold those bounds — no
/// `Rc`, `Cell`, or `RefCell` in internal state.
pub trait CorpusSource: Send + Sync {
    /// Iterate candidate texts in implementation-defined order. Per-item
    /// errors (for example an unreadable file in a filesystem walk) are
    /// yielded inline so the caller can choose to skip, log, or fail
    /// fast.
    fn iter(&self) -> Box<dyn Iterator<Item = Result<CorpusItem, CorpusError>> + '_>;

    /// Human-readable provenance label for diagnostics. Examples:
    /// `"in-memory"`, `"vendored:spec/aozora/fixtures"`,
    /// `"filesystem:/home/user/aozora-corpus"`.
    fn provenance(&self) -> &str;
}

/// Construct the default corpus source from the process environment.
///
/// Reads `AFM_CORPUS_ROOT`. If set and points at an existing directory,
/// returns a filesystem-backed source rooted there. Otherwise returns
/// [`None`]; sweep tests treat that as "no corpus available, skip".
///
/// Availability of a source does not imply any particular content is
/// present. Callers must not assume the corpus contains any specific
/// work — they may only stream what is found and check invariants.
///
/// The concrete source implementations arrive in a follow-up commit
/// (M2-S2); in the meantime this always returns [`None`] so downstream
/// sweep harnesses written against the trait contract will runtime-skip
/// gracefully.
#[must_use]
pub fn from_env() -> Option<Box<dyn CorpusSource>> {
    None
}

#[cfg(test)]
mod tests {
    use core::fmt;

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
        assert!(item.bytes.is_empty());
    }

    #[test]
    fn corpus_item_is_debug_and_clone() {
        fn assert_debug_clone<T: fmt::Debug + Clone>() {}
        assert_debug_clone::<CorpusItem>();
    }

    #[test]
    fn from_env_returns_none_for_unconfigured_stub() {
        // The M2-S1 stub unconditionally returns None regardless of env state.
        // M2-S2 replaces this with an AFM_CORPUS_ROOT-aware implementation.
        assert!(from_env().is_none());
    }
}
