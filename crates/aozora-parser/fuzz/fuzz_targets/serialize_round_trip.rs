//! Fuzz target — I3 (`serialize ∘ parse`) fixed point on arbitrary UTF-8.
//!
//! First `serialize(parse(src))` canonicalises the source. A second
//! application must be byte-identical: oscillation means the
//! classifier and serializer disagree on the canonical form (a real
//! bug — see ADR-0008's round-trip contract).
//!
//! Run with: `just fuzz serialize_round_trip -- -runs=10000`

#![no_main]

use afm_parser::{Options, parse, serialize};
use comrak::Arena;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(src) = core::str::from_utf8(data) else {
        return;
    };
    let opts = Options::afm_default();
    let arena_a = Arena::new();
    let first = serialize(&parse(&arena_a, src, &opts));
    let arena_b = Arena::new();
    let second = serialize(&parse(&arena_b, &first, &opts));
    assert_eq!(
        first, second,
        "I3 fixed-point broken for src={src:?}: first vs second differ"
    );
});
