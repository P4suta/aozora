//! Pin the handbook's CLI quickstart to the real diagnostic surface.
//!
//! `getting-started/cli.md` shows a worked `aozora check` diagnostic. The
//! load-bearing strings in that example — the dotted code, the message, and
//! the docs URL — are not transcribed by hand here: they are read from the
//! live [`aozora::Diagnostic`] the example demonstrates, exactly as `aozora
//! check` renders them. If any of them changes in `aozora-spec`, this test
//! fails until the handbook fence is regenerated, so the example can never
//! drift back into fiction.
//!
//! miette's *layout* (the box-drawing frame, the caret run, the help
//! line-wrapping) is deliberately NOT pinned — that is presentation art the
//! renderer owns and is free to reflow. Only the message / code / URL, which
//! appear verbatim and unwrapped in the fence, are asserted.
//!
//! The book is read at runtime with `fs::read` (not `include_str!`) so a
//! packaged crate published to crates.io — which ships no sibling
//! `aozora-book/` checkout — simply skips rather than failing to compile.

use std::fs;
use std::path::PathBuf;

use aozora::{Diagnostic, Span, codes};

/// The rendered CLI quickstart chapter, or `None` when this is a packaged
/// crate with no handbook checkout beside it.
fn cli_quickstart() -> Option<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("aozora-book")
        .join("src")
        .join("getting-started")
        .join("cli.md");
    fs::read_to_string(path).ok()
}

#[test]
fn quickstart_diagnostic_example_is_not_fiction() {
    let Some(book) = cli_quickstart() else {
        return; // packaged crate: no handbook checkout to pin against.
    };

    // The `｜青空《》` worked example demonstrates exactly this diagnostic. Its
    // message (thiserror `Display`), dotted code, and docs URL are the single
    // source of truth — `explain` reads help/url from the live miette impl, so
    // none of them can diverge from what `aozora check` actually prints.
    let diagnostic = Diagnostic::empty_ruby_reading(Span::new(0, 15));
    let message = diagnostic.to_string();
    let code = diagnostic.code();
    let info = Diagnostic::explain(codes::EMPTY_RUBY_READING)
        .expect("empty_ruby_reading is a known diagnostic code");

    assert!(
        book.contains(&message),
        "handbook CLI quickstart no longer shows the real diagnostic message \
         `{message}` — regenerate the example from `aozora check \
         --diagnostic-format human --color never`",
    );
    assert!(
        book.contains(code),
        "handbook CLI quickstart no longer shows the diagnostic code `{code}` \
         — regenerate the example from `aozora check`",
    );
    if let Some(url) = info.url.as_deref() {
        assert!(
            book.contains(url),
            "handbook CLI quickstart no longer shows the diagnostic URL `{url}` \
             — regenerate the example from `aozora check`",
        );
    }
}
