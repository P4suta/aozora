//! Fuzz target — `aozora_render::serialize::serialize` idempotency.
//!
//! Arbitrary UTF-8 source is lexed once and serialized back. The
//! result is then re-lexed and re-serialized; the two outputs must
//! be byte-equal (I3 fixed-point invariant: `serialize` is idempotent
//! on its own output).
//!
//! Run via `just fuzz-quick aozora-render serialize_round_trip` (or
//! `fuzz-deep` / `fuzz-marathon`).

#![no_main]

use aozora_pipeline::lex_into_arena;
use aozora_render::serialize::serialize;
use aozora_syntax::borrowed::Arena;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(src) = core::str::from_utf8(data) else {
        return;
    };
    let arena1 = Arena::new();
    let lex1 = lex_into_arena(src, &arena1);
    let first = serialize(&lex1);

    let arena2 = Arena::new();
    let lex2 = lex_into_arena(&first, &arena2);
    let second = serialize(&lex2);

    assert!(
        first == second,
        "I3 fixed-point broken for src bytes = {data:?}\n  first  = {first:?}\n  second = {second:?}",
    );
});
