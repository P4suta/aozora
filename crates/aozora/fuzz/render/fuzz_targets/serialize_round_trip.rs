//! Fuzz target — `aozora::render::serialize` idempotency.
//!
//! Arbitrary UTF-8 source is lexed once and serialized back. The
//! result is then re-lexed and re-serialized; the two outputs must
//! be byte-equal (I3 fixed-point invariant: `serialize` is idempotent
//! on its own output).
//!
//! Run via `just fuzz-quick render serialize_round_trip` (or
//! `fuzz-deep` / `fuzz-marathon`).

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(src) = core::str::from_utf8(data) else {
        return;
    };
    // Sources that carry the parser-reserved PUA sentinel range
    // (U+E001..U+E004) trigger `Diagnostic::SourceContainsPua` and the
    // lexer is free to consume those codepoints — they are reserved
    // markers, not user content. The serialize round-trip therefore
    // can't promise idempotency on such inputs: round 1's lex strips
    // them, leaving a different line shape for round 2's sanitize-stage
    // decorative-rule isolator to classify (see the
    // `crash-dcbadd08c7424e68f0820311a2cd78274aa87e52` regression
    // case). I3 is a contract over user-meaningful aozora source,
    // not over inputs that smuggle in lexer-internal markers.
    if src
        .chars()
        .any(|c| matches!(c, '\u{E001}'..='\u{E004}'))
    {
        return;
    }
    let first = aozora::parse(src).expect("fuzz input fits parser spans").snapshot().to_source();
    let second = aozora::parse(first.as_str())
        .expect("serialized fuzz input fits parser spans")
        .snapshot()
        .to_source();

    assert!(
        first == second,
        "I3 fixed-point broken for src bytes = {data:?}\n  first  = {first:?}\n  second = {second:?}",
    );
});
