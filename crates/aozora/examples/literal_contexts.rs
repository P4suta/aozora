//! Recover original notation bytes from source spans.

fn main() {
    let source = "冒頭｜青梅《おうめ》。［＃改ページ］次の章。";
    let doc = aozora::parse(source).expect("source fits parser span limit");
    let tree = doc.snapshot();

    for node in tree.nodes() {
        println!("{}", tree.slice(node.span()).expect("node span is valid"));
    }
}
