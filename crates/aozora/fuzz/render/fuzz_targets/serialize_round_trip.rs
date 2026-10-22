//! Checks serialization convergence with arbitrary UTF-8 source.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(src) = core::str::from_utf8(data) else {
        return;
    };
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
