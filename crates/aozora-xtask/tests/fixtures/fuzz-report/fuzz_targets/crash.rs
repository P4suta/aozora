#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    assert_ne!(data, b"boom", "the control input reached the engine");
});
