//! Parser for aozora-flavored-markdown.
//!
//! The real parser is a fork of [comrak](https://github.com/kivikakk/comrak) vendored
//! at `/upstream/comrak`. This crate re-exports that parser and layers the Aozora Bunko
//! extensions (ruby, bouten, tate-chu-yoko, block annotations, gaiji, etc.) on top.
//!
//! At the M0 Spike milestone this is a stub: the `aozora` module pins the module layout
//! so later work can fill in the extension points without churn.

#![forbid(unsafe_code)]

pub mod aozora;

#[cfg(feature = "html")]
pub mod html;

/// Opaque handle returned by future parser entry points. Kept here to pin the API
/// contract even though the implementation is deferred until the comrak fork lands.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Options {
    /// Enable the Aozora Bunko extension set. Defaults to `true` for afm; disable to
    /// run the parser in plain CommonMark/GFM mode (for compatibility test suites).
    pub aozora: bool,
}

impl Options {
    #[must_use]
    pub const fn afm_default() -> Self {
        Self { aozora: true }
    }

    #[must_use]
    pub const fn commonmark_only() -> Self {
        Self { aozora: false }
    }
}
