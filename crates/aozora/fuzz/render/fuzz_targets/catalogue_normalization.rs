//! Checks serialization stability across directive normalization modes.

#![no_main]

use aozora::{DirectiveNormalization, SerializeOptions};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(src) = core::str::from_utf8(data) else {
        return;
    };
    if src.chars().any(|c| matches!(c, '\u{E001}'..='\u{E004}')) {
        return;
    }

    let snapshot = aozora::parse(src).expect("fuzz input fits parser spans").snapshot();

    let off = snapshot.to_source_with(
        SerializeOptions::default().directives(DirectiveNormalization::Off),
    );
    assert!(
        off == snapshot.to_source(),
        "Off normalization must equal default serialize for src bytes = {data:?}",
    );

    for directives in [
        DirectiveNormalization::Canonical,
        DirectiveNormalization::Degraded,
    ] {
        let opts = SerializeOptions::default().directives(directives);
        let once = snapshot.to_source_with(opts);
        let twice = aozora::parse(once.as_str())
            .expect("serialized fuzz input fits parser spans")
            .snapshot()
            .to_source_with(opts);
        assert!(
            once == twice,
            "normalization {directives:?} not idempotent for src bytes = {data:?}\n  \
             once  = {once:?}\n  twice = {twice:?}",
        );
    }
});
