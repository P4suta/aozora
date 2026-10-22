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
