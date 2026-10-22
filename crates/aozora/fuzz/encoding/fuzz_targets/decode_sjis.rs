//! Exercises `aozora::decode_sjis` with arbitrary bytes.

#![no_main]

use aozora::decode_sjis;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = decode_sjis(data) else {
        return;
    };
    assert!(
        std::str::from_utf8(text.as_bytes()).is_ok(),
        "decode_sjis returned non-UTF-8 String for input bytes = {data:?}",
    );
});
