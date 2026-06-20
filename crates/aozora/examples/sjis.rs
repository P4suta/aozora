//! Aozora Bunko ships its corpus as Shift_JIS. The parser itself is
//! strictly UTF-8, so decode first through
//! [`aozora::encoding::decode_sjis`], then parse + serialize the
//! resulting UTF-8 text.
//!
//! Handbook recipe: <https://p4suta.github.io/aozora/getting-started/library.html>
//! (Library Quickstart → Shift_JIS input). Uses only the `aozora`
//! umbrella surface (`aozora::encoding` re-exports the encoding crate).
//!
//! Run with:
//!
//! ```text
//! cargo run --example sjis
//! ```

use aozora::Document;
use aozora::encoding::decode_sjis;

fn main() {
    // 「青空文庫」 encoded as Shift_JIS (two bytes per kanji).
    let sjis: &[u8] = &[0x90, 0xC2, 0x8B, 0xF3, 0x95, 0xB6, 0x8C, 0xC9];

    // `decode_sjis` is strict: it returns `Err(DecodeError)` on a
    // malformed byte sequence rather than substituting replacement
    // characters, so corrupted source surfaces loudly.
    let utf8 = decode_sjis(sjis).expect("valid Shift_JIS input");
    println!("decoded: {utf8}");

    // From here it is the ordinary UTF-8 path: hand the decoded string
    // to `Document` and parse as usual.
    let doc = Document::new(utf8);
    let tree = doc.parse();

    println!("--- to_html ---");
    println!("{}", tree.to_html());

    println!("--- serialize ---");
    println!("{}", tree.to_source());
}
