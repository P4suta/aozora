//! Authoring aid for `spec-vectors`: parse the source passed on argv (or
//! stdin) and print the four projections a conformance vector pins —
//! `serialize`, `html`, `nodes`, `pairs` — plus `diagnostics`.
//!
//! Run: `cargo run -p aozora --features wire --example vector_dump -- '［＃…］…'`
//! The emitted JSON fragments are pasted into a vector's `expected` block;
//! the human still hand-verifies them against the official 注記一覧 (the
//! de-circularisation contract — the parser dump is a convenience, not the
//! source of truth).

use std::env;
use std::io::{self, Read as _};

use aozora::Document;
use aozora::wire::{serialize_diagnostics, serialize_nodes, serialize_pairs};

fn main() {
    let source = env::args().nth(1).unwrap_or_else(|| {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf).expect("read stdin");
        buf
    });
    let doc = Document::new(source.clone());
    let tree = doc.parse();

    println!("--- source (debug) ---");
    println!("{source:?}");
    println!("--- serialize ---");
    println!("{:?}", tree.serialize());
    println!("--- html ---");
    println!("{:?}", tree.to_html());
    println!("--- nodes ---");
    println!("{}", serialize_nodes(&tree));
    println!("--- pairs ---");
    println!("{}", serialize_pairs(&tree));
    println!("--- diagnostics ---");
    println!("{}", serialize_diagnostics(tree.diagnostics()));
}
