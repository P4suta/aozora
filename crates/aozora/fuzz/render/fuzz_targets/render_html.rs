//! Checks HTML rendering with arbitrary UTF-8 source.

#![no_main]

use libfuzzer_sys::fuzz_target;

const PUA_SENTINELS: [char; 4] = ['\u{E001}', '\u{E002}', '\u{E003}', '\u{E004}'];

fuzz_target!(|data: &[u8]| {
    let Ok(src) = core::str::from_utf8(data) else {
        return;
    };
    if src.chars().any(|c| PUA_SENTINELS.contains(&c)) {
        return;
    }
    let html = aozora::parse(src).expect("fuzz input fits parser spans").snapshot().to_html();
    for sentinel in PUA_SENTINELS {
        assert!(
            !html.contains(sentinel),
            "PUA sentinel {sentinel:?} leaked into rendered HTML for src bytes = {data:?}\n  html = {html:?}",
        );
    }
});
