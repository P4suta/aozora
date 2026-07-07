//! AST-free byte-spelling primitives.
//!
//! The shared, lifetime-free marker/tag/escape emitters that every renderer —
//! and the source splice layer — reuse. Each function takes only `Copy` scalar
//! payloads (`RegionFormat` / `Container` / `LineFormat` / `HeadingKind` /
//! `&str`), so the byte spelling of a construct is single-source.
//!
//! - `html` spells HTML: the block/paragraph state machine, the container /
//!   heading / line tag writers, and the text escaper.
//! - [`source`] spells canonical Aozora source: the container / layout-directive
//!   / heading markers reused by the serializer and the splice layer.

pub(crate) mod html;
pub mod source;
