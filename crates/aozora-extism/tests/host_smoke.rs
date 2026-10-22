//! Compares Extism plugin exports with the shared wire records using the built artifact.
#![cfg(feature = "host-smoke")]

use std::env;
use std::fs;
use std::path::PathBuf;

use aozora::json;
use extism::{Manifest, Plugin, Wasm};

const DEFAULT_WASM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/dist/aozora.wasm");

const CORPUS: [&str; 5] = [
    "plain",
    "｜青梅《おうめ》",
    "abc\u{E001}def",
    "［＃ここから2字下げ］あ［＃ここで字下げ終わり］",
    "※［＃「弓＋鰐のつくり」、第4水準2-84-40］",
];

#[expect(
    clippy::panic,
    clippy::expect_used,
    reason = "test setup requires a readable and valid plugin artifact"
)]
fn load_plugin() -> Plugin {
    let path = env::var_os("AOZORA_EXTISM_ARTIFACT")
        .map_or_else(|| PathBuf::from(DEFAULT_WASM_PATH), PathBuf::from);
    let bytes = fs::read(&path).unwrap_or_else(|error| {
        panic!(
            "could not read {}: {error}\nrun `just extism-build` first",
            path.display()
        );
    });
    let manifest = Manifest::new([Wasm::data(bytes)]);
    Plugin::new(&manifest, [], false).expect("instantiate aozora.wasm plugin")
}

#[test]
fn every_export_is_byte_identical_to_the_shared_authority() {
    let mut plugin = load_plugin();
    for src in CORPUS {
        let doc = aozora::parse(src.to_owned()).expect("source fits parser span limit");
        let tree = doc.snapshot();

        let html: &str = plugin.call("to_html", src).expect("to_html");
        assert_eq!(html, tree.to_html(), "to_html src: {src}");

        let serialized: &str = plugin.call("to_source", src).expect("to_source");
        assert_eq!(serialized, tree.to_source(), "to_source src: {src}");

        let diagnostics: &str = plugin
            .call("diagnostics_json", src)
            .expect("diagnostics_json");
        assert_eq!(
            diagnostics,
            json::diagnostics(tree.diagnostics()),
            "diagnostics_json src: {src}"
        );

        let nodes: &str = plugin.call("nodes_json", src).expect("nodes_json");
        assert_eq!(nodes, json::nodes(&tree), "nodes_json src: {src}");

        let pairs: &str = plugin.call("pairs_json", src).expect("pairs_json");
        assert_eq!(pairs, json::pairs(&tree), "pairs_json src: {src}");

        let containers: &str = plugin
            .call("container_pairs_json", src)
            .expect("container_pairs_json");
        assert_eq!(
            containers,
            json::container_pairs(&tree),
            "container_pairs_json src: {src}"
        );

        let gaiji: &str = plugin.call("gaiji_json", src).expect("gaiji_json");
        assert_eq!(gaiji, json::gaiji(&tree), "gaiji_json src: {src}");
    }
}

#[test]
fn static_exports_match_their_authority() {
    let mut plugin = load_plugin();

    let slugs: &str = plugin.call("slugs_json", "").expect("slugs_json");
    assert_eq!(slugs, json::slugs());

    let schema: &str = plugin.call("schema_version", "").expect("schema_version");
    assert_eq!(schema, json::SCHEMA_VERSION.to_string());

    let version: &str = plugin.call("version", "").expect("version");
    assert!(
        version.starts_with(env!("CARGO_PKG_VERSION")),
        "plugin version {version:?} does not start with the crate version"
    );
}
