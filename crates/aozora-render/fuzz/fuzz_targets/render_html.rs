//! Fuzz target — `aozora_render::html::render_to_string` on arbitrary
//! UTF-8.
//!
//! Arbitrary bytes are decoded as UTF-8 (invalid sequences skip this
//! iteration). The source is lexed via `aozora_pipeline` and rendered
//! to HTML via `aozora_render::html`. Targets renderer panics, dangling
//! arena references, and the round-trip "no PUA sentinel survives in
//! the rendered HTML" invariant.
//!
//! Run via `just fuzz-quick aozora-render render_html` (or
//! `fuzz-deep` / `fuzz-marathon`).

#![no_main]

use aozora_pipeline::lex_into_arena;
use aozora_render::html::render_to_string;
use aozora_syntax::borrowed::Arena;
use libfuzzer_sys::fuzz_target;

/// PUA sentinel codepoints embedded by the lexer that the renderer
/// must consume — none should survive into rendered HTML.
const PUA_SENTINELS: [char; 4] = ['\u{E001}', '\u{E002}', '\u{E003}', '\u{E004}'];

fuzz_target!(|data: &[u8]| {
    let Ok(src) = core::str::from_utf8(data) else {
        return;
    };
    let arena = Arena::new();
    let lex_out = lex_into_arena(src, &arena);
    let html = render_to_string(&lex_out);
    for sentinel in PUA_SENTINELS {
        assert!(
            !html.contains(sentinel),
            "PUA sentinel {sentinel:?} leaked into rendered HTML for src bytes = {data:?}\n  html = {html:?}",
        );
    }
});
