//! Decode Shift_JIS before parsing Aozora source.

use aozora::decode_sjis;

#[expect(
    clippy::expect_used,
    clippy::print_stdout,
    reason = "fixed example inputs must parse successfully and the example demonstrates stdout output"
)]
fn main() {
    let sjis: &[u8] = &[0x90, 0xC2, 0x8B, 0xF3, 0x95, 0xB6, 0x8C, 0xC9];

    let utf8 = decode_sjis(sjis).expect("valid Shift_JIS input");
    println!("decoded: {utf8}");

    let doc = aozora::parse(utf8).expect("source fits parser span limit");
    let tree = doc.snapshot();

    println!("--- to_html ---");
    println!("{}", tree.to_html());

    println!("--- serialize ---");
    println!("{}", tree.to_source());
}
