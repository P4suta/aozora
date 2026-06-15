//! End-to-end ABI smoke test for the Extism plugin.
//!
//! The library unit tests prove the `logic` functions are byte-identical
//! to `aozora::wire`, but they call that logic in-process — they never
//! cross the Extism plugin boundary. This test does: it loads the
//! actually-built `dist/aozora.wasm` through the Extism **host** SDK
//! (wasmtime) and asserts every export returns bytes identical to
//! calling `aozora::wire` directly. That extends the cross-binding
//! guarantee (already asserted for FFI / WASM / `PyO3`) to the wasm
//! transport, mirroring what `smoke-ffi` does for the C ABI.
//!
//! Gated behind the `host-smoke` feature (which pulls wasmtime) so the
//! default `just test` / `just ci` path never compiles it. Run via
//! `just smoke-extism`, which builds the artifact first.
#![cfg(feature = "host-smoke")]

use std::fs;

use aozora::{Document, wire};
use extism::{Manifest, Plugin, Wasm};

/// The artifact `just extism-build` writes. Read at runtime (not
/// `include_bytes!`) so a normal compile never depends on the build
/// having run.
const WASM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/dist/aozora.wasm");

/// Same corpus as the library unit tests: plain text, a ruby span, a
/// PUA-collision diagnostic, and an indent container.
const CORPUS: [&str; 4] = [
    "plain",
    "｜青梅《おうめ》",
    "abc\u{E001}def",
    "［＃ここから2字下げ］あ［＃ここで字下げ終わり］",
];

fn load_plugin() -> Plugin {
    let bytes = fs::read(WASM_PATH).unwrap_or_else(|e| {
        panic!("could not read {WASM_PATH}: {e}\nrun `just extism-build` first");
    });
    let manifest = Manifest::new([Wasm::data(bytes)]);
    Plugin::new(&manifest, [], false).expect("instantiate aozora.wasm plugin")
}

#[test]
fn every_export_is_byte_identical_to_the_shared_authority() {
    let mut plugin = load_plugin();
    for src in CORPUS {
        let doc = Document::new(src.to_owned());
        let tree = doc.parse();

        let html: &str = plugin.call("to_html", src).expect("to_html");
        assert_eq!(html, tree.to_html(), "to_html src: {src}");

        let serialized: &str = plugin.call("serialize", src).expect("serialize");
        assert_eq!(serialized, tree.serialize(), "serialize src: {src}");

        let diagnostics: &str = plugin
            .call("diagnostics_json", src)
            .expect("diagnostics_json");
        assert_eq!(
            diagnostics,
            wire::serialize_diagnostics(tree.diagnostics()),
            "diagnostics_json src: {src}"
        );

        let nodes: &str = plugin.call("nodes_json", src).expect("nodes_json");
        assert_eq!(nodes, wire::serialize_nodes(&tree), "nodes_json src: {src}");

        let pairs: &str = plugin.call("pairs_json", src).expect("pairs_json");
        assert_eq!(pairs, wire::serialize_pairs(&tree), "pairs_json src: {src}");

        let containers: &str = plugin
            .call("container_pairs_json", src)
            .expect("container_pairs_json");
        assert_eq!(
            containers,
            wire::serialize_container_pairs(&tree),
            "container_pairs_json src: {src}"
        );
    }
}

#[test]
fn schema_version_export_matches_wire() {
    let mut plugin = load_plugin();
    let version: &str = plugin.call("schema_version", "").expect("schema_version");
    assert_eq!(version, wire::SCHEMA_VERSION.to_string());
}
