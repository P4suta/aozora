// Build script — compile the tree-sitter generated parser.c into a
// static library that bindings/rust/lib.rs links against.
//
// `parser.c` is committed to the repo (regenerated via
// `tree-sitter generate` from grammar.js — the pinned tree-sitter CLI
// embeds its own JS engine, so no node is needed). The regen is
// drift-gated by `xtask conformance grammar --check`. Downstream
// consumers only need a C toolchain.

fn main() {
    let src_dir = std::path::PathBuf::from("src");

    let mut config = cc::Build::new();
    config
        .include(&src_dir)
        .flag_if_supported("-Wno-unused-parameter")
        .flag_if_supported("-Wno-unused-but-set-variable")
        .flag_if_supported("-Wno-trigraphs");

    let parser_path = src_dir.join("parser.c");
    config.file(&parser_path);
    println!("cargo:rerun-if-changed={}", parser_path.display());

    config.compile("tree-sitter-aozora");
}
