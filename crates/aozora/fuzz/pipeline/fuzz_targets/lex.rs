//! Exercises public parsing with arbitrary UTF-8 source.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(src) = core::str::from_utf8(data) else {
        return;
    };
    let out = aozora::parse(src).expect("fuzz input fits parser spans").snapshot();
    for diag in out.diagnostics() {
        let span = diag.span();
        assert!(
            span.start <= span.end,
            "diagnostic span {:?} has start > end; src bytes = {data:?}",
            span,
        );
    }
});
