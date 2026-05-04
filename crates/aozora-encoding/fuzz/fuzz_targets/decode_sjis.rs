//! Fuzz target — `aozora_encoding::decode_sjis` on arbitrary bytes.
//!
//! Arbitrary input bytes are fed into `decode_sjis`. Failures
//! (non-Shift_JIS input) are accepted and skip the iteration; we only
//! assert that the decoder never panics on adversarial input —
//! truncated trail bytes, lead-byte-at-EOF, malformed multi-byte
//! sequences. On successful decodes the result must be valid UTF-8.
//!
//! Run via `just fuzz-quick aozora-encoding decode_sjis` (or
//! `fuzz-deep` / `fuzz-marathon`).

#![no_main]

use aozora_encoding::decode_sjis;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = decode_sjis(data) else {
        return;
    };
    // `decode_sjis` returns `String`, which is structurally UTF-8 by
    // type — but assert anyway so any future refactor that broadens
    // the return type can't silently regress the contract.
    assert!(
        std::str::from_utf8(text.as_bytes()).is_ok(),
        "decode_sjis returned non-UTF-8 String for input bytes = {data:?}",
    );
});
