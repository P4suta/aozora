//! Print diagnostics and partial HTML for malformed input.

fn main() {
    let source = "正しい段落。\n［＃ここから2字下げ";
    let doc = aozora::parse(source).expect("source fits parser span limit");
    let tree = doc.snapshot();

    let diagnostics = tree.diagnostics();
    println!("{} diagnostic(s)", diagnostics.len());

    for diag in diagnostics {
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

    println!("--- partial render ---");
    println!("{}", tree.to_html());
}
