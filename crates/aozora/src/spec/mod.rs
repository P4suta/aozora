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
pub(crate) use offset::{NormalizedOffset, SourceOffset};
pub use pair::{PairKind, PairLink};
#[cfg(test)]
pub(crate) use sentinels::Sentinel;
pub(crate) use sentinels::{
    BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, INLINE_SENTINEL,
};
pub(crate) use slugs::canonicalise_slug;
pub(crate) use slugs::{SLUGS, roman_slug};
pub use slugs::{SlugEntry, SlugFamily};
pub use span::Span;
pub(crate) use trigger::{TriggerKind, classify_trigger_bytes};
