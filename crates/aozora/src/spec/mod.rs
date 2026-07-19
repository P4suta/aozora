//! Aozora Bunko notation — canonical specification vocabulary.
//!
//! The **single source of truth** for facts every other module in the crate
//! needs to agree on:
//!
//! - **PUA sentinel codepoints** — the four `U+E001..U+E004` markers the
//!   lexer injects into normalized text (see [`sentinels`]).
//! - **[`Span`]** — `(u32, u32)` byte-range over a UTF-8 source.
//! - **[`TriggerKind`]** — the set of Aozora notation marker characters
//!   (`｜《》［］＃※〔〕「」`) plus [`classify_trigger_bytes`], the
//!   `match` that maps a UTF-8 trigger byte sequence to its kind.
//! - **[`PairKind`]** — categories of balanced open/close delimiters.
//! - **[`Diagnostic`]** — every non-fatal observation any pipeline stage can emit.
//!
//! These types are shared by the `syntax`, `scan`, `pipeline`, and `render`
//! modules; the canonical set is re-exported at the crate root.

#![forbid(unsafe_code)]

pub(crate) mod diagnostic;
pub(crate) mod offset;
pub(crate) mod pair;
pub(crate) mod sentinels;
pub(crate) mod slugs;
pub(crate) mod span;
pub(crate) mod trigger;

#[cfg(test)]
pub(crate) use diagnostic::codes;
pub use diagnostic::{Diagnostic, DiagnosticInfo, DiagnosticSource, InternalCheckCode, Severity};
pub use offset::{NormalizedOffset, SourceOffset};
pub use pair::{PairKind, PairLink};
#[cfg(test)]
pub(crate) use sentinels::Sentinel;
pub(crate) use sentinels::{
    BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, INLINE_SENTINEL,
};
pub(crate) use slugs::canonicalise_slug;
pub use slugs::{RENDER_SLUGS, RenderSlug};
pub(crate) use slugs::{SLUGS, roman_slug};
pub use slugs::{SlugEntry, SlugFamily};
pub use span::Span;
pub(crate) use trigger::{TriggerKind, classify_trigger_bytes};
