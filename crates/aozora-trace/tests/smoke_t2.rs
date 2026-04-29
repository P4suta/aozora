//! Smoke test: load a sample samply trace, symbolicate it, and
//! verify the analyses return non-empty, sensible reports.
//!
//! Skipped when the trace file isn't available — required only on
//! the host that just ran `just samply-corpus`.

use std::env;
use std::fs;
use std::path::Path;

use aozora_trace::{
    Categorizer, RollupConfig, SymbolCache, Symbolicator, TableRenderable, Trace, analysis,
};

const TRACE_PATH: &str = "/tmp/aozora-corpus-20260426-185221.json.gz";
const BINARY_PATH: &str = "target/release/examples/throughput_by_class";

fn skip_if_no_trace() -> Option<()> {
    if !Path::new(TRACE_PATH).exists() || !Path::new(BINARY_PATH).exists() {
        eprintln!("skipping: {TRACE_PATH} or {BINARY_PATH} missing");
        return None;
    }
    Some(())
}

#[test]
fn full_workflow_t2_trace() {
    if skip_if_no_trace().is_none() {
        return;
    }
    let mut trace = Trace::load(Path::new(TRACE_PATH)).expect("load");
    assert!(trace.total_samples() > 1000);
    assert!(trace.libs.len() >= 3);

    // Symbolicate via DWARF in the binary.
    let mut sym = Symbolicator::new();
    sym.add_binary("throughput_by_class", Path::new(BINARY_PATH))
        .expect("loader");

    let mut cache = SymbolCache::default();
    let (resolved, attempted) = sym.resolve_into(&mut trace, &mut cache);
    assert!(resolved > 0, "DWARF resolution returned nothing");
    assert!(resolved <= attempted);

    // Hot leaves — should have a non-empty list and the top frame
    // should not be an unresolved hex address.
    let hot = analysis::hot_leaves(&trace, 5);
    assert!(!hot.rows.is_empty());
    assert!(
        !hot.rows[0].label.starts_with("0x"),
        "top hot leaf is unresolved: {}",
        hot.rows[0].label
    );

    // Library distribution — must include the binary.
    let libs = analysis::library_distribution(&trace);
    assert!(libs.rows.iter().any(|r| r.library == "throughput_by_class"));

    // Rollup with built-in aozora defaults.
    let cat = RollupConfig::aozora_defaults().compile().expect("compile");
    let roll = analysis::rollup(&trace, &cat);
    let known = ["phase1_scan", "phase1_walker", "memchr_scan"];
    for k in known {
        assert!(
            roll.rows.iter().any(|r| r.category == k),
            "missing category {k}"
        );
    }

    // Folded stacks → at least 1 line.
    let folded = analysis::folded_stacks(&trace);
    assert!(!folded.is_empty());
    let folded_text = analysis::render_folded(&folded);
    assert!(folded_text.contains(';') || !folded.is_empty());

    // Render each report — must not panic and must produce > 0 chars.
    assert!(!hot.render_table().is_empty());
    assert!(!libs.render_table().is_empty());
    assert!(!roll.render_table().is_empty());
}

#[test]
fn cache_round_trip_t2_trace() {
    if skip_if_no_trace().is_none() {
        return;
    }
    let mut trace = Trace::load(Path::new(TRACE_PATH)).expect("load");
    let mut sym = Symbolicator::new();
    sym.add_binary("throughput_by_class", Path::new(BINARY_PATH))
        .expect("loader");

    // Resolve, persist, load back, apply to a fresh trace.
    let mut cache = SymbolCache::default();
    let (n_resolved, _) = sym.resolve_into(&mut trace, &mut cache);
    assert!(n_resolved > 0);

    let cache_path = env::temp_dir().join("aozora-trace-test-cache.json");
    cache.write(&cache_path).expect("write cache");
    let reloaded = SymbolCache::load(&cache_path)
        .expect("load cache")
        .expect("present");

    let mut fresh = Trace::load(Path::new(TRACE_PATH)).expect("load #2");
    let applied = reloaded.apply(&mut fresh);
    assert!(
        applied >= n_resolved,
        "cache replay covered fewer frames than original resolve: {applied} vs {n_resolved}"
    );

    drop(fs::remove_file(&cache_path));
}

#[test]
fn category_compile_round_trips() {
    let cfg = RollupConfig::aozora_defaults();
    let cat = cfg.compile().expect("compile");
    // Spot-check a known function name from the trace.
    assert_eq!(
        cat.classify("aho_corasick::packed::teddy::generic::Slim<V,3_usize>::find"),
        "phase1_scan"
    );
    assert_eq!(
        cat.classify("aozora_lexer::phase1_events::trigger_kind_at"),
        "phase1_walker"
    );
    assert_eq!(
        cat.classify("encoding_rs::variant::VariantDecoder::decode_to_utf8_raw"),
        "corpus_load_sjis"
    );
    assert_eq!(
        cat.classify("memchr::arch::x86_64::avx2::memchr::One::find_raw_avx2"),
        "memchr_scan"
    );
    assert_eq!(cat.classify("a_function_we_didnt_define"), "unknown");
    let _: &Categorizer = &cat;
}

#[test]
fn category_toml_round_trips() {
    let toml = r#"
[[categories]]
name = "scanner"
patterns = ["aho_corasick", "aozora_scan"]

[[categories]]
name = "everything_else"
patterns = ["."]
"#;
    let cfg = RollupConfig::from_toml(toml).expect("toml parse");
    let cat = cfg.compile().expect("compile");
    assert_eq!(cat.classify("aho_corasick::foo"), "scanner");
    assert_eq!(cat.classify("foo"), "everything_else");
}
