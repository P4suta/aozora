#![no_main]

libfuzzer_sys::fuzz_target!(|_data: &[u8]| {});
