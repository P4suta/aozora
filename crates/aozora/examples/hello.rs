//! Parse ruby and print HTML and canonical source.

fn main() {
    let doc = aozora::parse("｜青梅《おうめ》").expect("source fits parser span limit");
    let tree = doc.snapshot();

    println!("--- to_html ---");
    println!("{}", tree.to_html());

    println!("--- serialize ---");
    println!("{}", tree.to_source());
}
