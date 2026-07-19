//! Fuzz target — `aozora::parse` on arbitrary UTF-8.
//!
//! Arbitrary bytes are decoded as UTF-8 (invalid sequences skip this
//! iteration). The resulting source text is parsed through the public
//! document API. Parsing must terminate without panicking and every
//! reported diagnostic span must be non-inverted.
//!
//! Run with the standard `just fuzz-{quick,deep,marathon,triage,
//! promote}` family from the workspace root, e.g.
//! `just fuzz-quick pipeline lex`.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(src) = core::str::from_utf8(data) else {
        return;
    };
    let out = aozora::parse(src).snapshot();
    for diag in out.diagnostics() {
        let span = diag.span();
        assert!(
            span.start <= span.end,
            "diagnostic span {:?} has start > end; src bytes = {data:?}",
            span,
        );
    }
});
