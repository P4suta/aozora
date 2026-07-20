//! Diagnostics are non-fatal: the parser always produces a tree, even
//! from malformed input, and reports what it noticed through
//! [`aozora::Diagnostic`]. Here we feed it an unclosed `［＃` annotation
//! bracket and print every diagnostic's stable code, severity, and span.
//!
//! surface.
//!
//! Run with:
//!
//! ```text
//! cargo run --example diagnostics
//! ```

fn main() {
    // `［＃` opens an annotation that is never closed — the pairing
    // phase reaches end-of-input with the open delimiter still on its
    // stack and emits `aozora::lex::unclosed_bracket`.
    let source = "正しい段落。\n［＃ここから2字下げ";
    let doc = aozora::parse(source).expect("source fits parser span limit");
    let tree = doc.snapshot();

    let diagnostics = tree.diagnostics();
    println!("{} diagnostic(s)", diagnostics.len());

    for diag in diagnostics {
        // `code()` is a stable identifier (see the Diagnostics
        // catalogue); `severity()` returns the Error/Warning/Note
        // routing axis; `span()` is a byte-range into the source.
        let span = diag.span();
        println!(
            "[{:?}] {} @ {}..{}  ({:?})",
            diag.severity(),
            diag.code(),
            span.start,
            span.end,
            span.slice(source),
        );
    }

    // The parse still succeeded — rendering works regardless.
    println!("--- partial render ---");
    println!("{}", tree.to_html());
}
