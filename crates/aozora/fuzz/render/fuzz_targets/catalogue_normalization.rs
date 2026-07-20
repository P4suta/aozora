//! Fuzz target — notation-hygiene normalization (`serialize_with` under the
//! Tier1 `Canonical` and Tier2 `Degraded` levels).
//!
//! Arbitrary UTF-8 source is lexed, then serialized under each normalization
//! level. Three invariants are checked over arbitrary input:
//!
//! - `Off` normalization is byte-identical to the default `serialize` — the
//!   opt-in catalogues never touch the default path.
//! - `Canonical` / `Degraded` never panic (guards the catalogue `strip_suffix`
//!   / char-boundary arithmetic behind the serializer).
//! - each level is a second-pass fixed point (the catalogue outputs are
//!   recognised, non-key spellings, so a further pass is a no-op) — the
//!   idempotency the `fmt --fix` / `render --degraded` write and render paths
//!   depend on.
//!
//! Run via `just fuzz-quick render catalogue_normalization` (or
//! `fuzz-deep` / `fuzz-marathon`).

#![no_main]

use aozora::{DirectiveNormalization, SerializeOptions};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(src) = core::str::from_utf8(data) else {
        return;
    };
    // As in `serialize_round_trip`: sources smuggling the parser-reserved PUA
    // sentinel range (U+E001..=U+E004) are not user-meaningful aozora source,
    // and the lexer is free to consume those markers — so the fixed-point
    // contract does not hold for them.
    if src.chars().any(|c| matches!(c, '\u{E001}'..='\u{E004}')) {
        return;
    }

    let snapshot = aozora::parse(src).expect("fuzz input fits parser spans").snapshot();

    // `Off` is byte-identical to the default serialize — the opt-in catalogues
    // never alter the default path.
    let off = snapshot.to_source_with(
        SerializeOptions::default().directives(DirectiveNormalization::Off),
    );
    assert!(
        off == snapshot.to_source(),
        "Off normalization must equal default serialize for src bytes = {data:?}",
    );

    // `Canonical` and `Degraded` never panic and are second-pass fixed points.
    for directives in [
        DirectiveNormalization::Canonical,
        DirectiveNormalization::Degraded,
    ] {
        let opts = SerializeOptions::default().directives(directives);
        let once = snapshot.to_source_with(opts);
        let twice = aozora::parse(once.as_str())
            .expect("serialized fuzz input fits parser spans")
            .snapshot()
            .to_source_with(opts);
        assert!(
            once == twice,
            "normalization {directives:?} not idempotent for src bytes = {data:?}\n  \
             once  = {once:?}\n  twice = {twice:?}",
        );
    }
});
