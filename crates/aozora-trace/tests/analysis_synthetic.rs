//! Synthetic, deterministic coverage for the pure-logic analysis
//! modules (`hot` / `compare` / `rollup` / `stacks` / `libs` /
//! `flame`), plus `categories`, `load`, and the sidecar `cache`.
//!
//! Every profile here is hand-built via the public [`Trace::from_json`]
//! constructor over tiny gecko-format JSON, so the assertions pin the
//! exact ranked / aggregated output with no I/O, no DWARF, and no
//! reliance on a recorded trace file being present.

// `matching_stacks` takes a compiled `Regex`; the synthetic filters
// here are deliberately simple literals/anchors, which clippy would
// otherwise flag as `trivial_regex`. They exercise the real regex
// path the analysis uses, so the literal form is intentional.
#![allow(
    clippy::trivial_regex,
    reason = "matching_stacks requires a Regex; simple filters exercise the real path"
)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::{env, fs, process};

use aozora_trace::analysis::{HotMode, RowKind};
use aozora_trace::{
    Categorizer, LibIdent, RollupConfig, SymbolCache, TableRenderable, Trace, analysis,
};
use regex::Regex;
use serde_json::{Value, json};

/// Floating-point comparison tolerance for percentage assertions.
const EPS: f64 = 1e-9;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < EPS
}

/// Build a single-threaded synthetic gecko profile.
///
/// - `strings`: the thread's string array (function names live here).
/// - `funcs`: `(name_idx, resource_idx)` per func-table row.
/// - `resources`: `lib_idx` per resource-table row (`None` ⇒ unattributed).
/// - `frames`: `(address, func_idx)` per frame-table row.
/// - `stacks`: `(prefix, frame_idx)` per stack-table row, leaf chain.
/// - `samples`: `(stack_idx, weight)` per sample (`None` stack ⇒ idle).
/// - `libs`: library names; each becomes a `Library` with that `name`.
#[allow(clippy::too_many_arguments, reason = "synthetic profile builder")]
fn build_trace(
    strings: &[&str],
    funcs: &[(usize, Option<usize>)],
    resources: &[Option<usize>],
    frames: &[(u64, usize)],
    stacks: &[(Option<usize>, usize)],
    samples: &[(Option<usize>, u64)],
    libs: &[&str],
) -> Trace {
    let string_array: Vec<Value> = strings.iter().map(|s| json!(s)).collect();
    let func_names: Vec<Value> = funcs.iter().map(|(n, _)| json!(n)).collect();
    let func_res: Vec<Value> = funcs.iter().map(|(_, r)| json!(r)).collect();
    let res_lib: Vec<Value> = resources.iter().map(|r| json!(r)).collect();
    let frame_addr: Vec<Value> = frames.iter().map(|(a, _)| json!(a)).collect();
    let frame_func: Vec<Value> = frames.iter().map(|(_, f)| json!(f)).collect();
    let stack_prefix: Vec<Value> = stacks.iter().map(|(p, _)| json!(p)).collect();
    let stack_frame: Vec<Value> = stacks.iter().map(|(_, f)| json!(f)).collect();
    let sample_stack: Vec<Value> = samples.iter().map(|(s, _)| json!(s)).collect();
    let sample_weight: Vec<Value> = samples.iter().map(|(_, w)| json!(w)).collect();
    let lib_objs: Vec<Value> = libs
        .iter()
        .map(|name| json!({ "name": name, "path": format!("/lib/{name}") }))
        .collect();

    let json = json!({
        "libs": lib_objs,
        "threads": [{
            "name": "main",
            "tid": 1,
            "isMainThread": true,
            "stringArray": string_array,
            "stackTable": { "prefix": stack_prefix, "frame": stack_frame },
            "frameTable": { "address": frame_addr, "func": frame_func },
            "funcTable": { "name": func_names, "resource": func_res },
            "resourceTable": { "lib": res_lib },
            "samples": { "stack": sample_stack, "weight": sample_weight },
        }],
    });
    Trace::from_json(&json, PathBuf::from("synthetic")).expect("synthetic load")
}

/// Two-frame stack: leaf `b` called by root `a`, both in lib 0.
/// Three samples land on `b`'s leaf, one is idle (no stack).
fn simple_trace() -> Trace {
    build_trace(
        &["a", "b"],
        &[(0, Some(0)), (1, Some(0))],
        &[Some(0)],
        &[(0x10, 0), (0x20, 1)],
        // stack 0 = a(root); stack 1 = b with prefix a.
        &[(None, 0), (Some(0), 1)],
        // 3 samples on leaf b, 1 idle.
        &[(Some(1), 1), (Some(1), 1), (Some(1), 1), (None, 1)],
        &["mybin"],
    )
}

// ---- hot_leaves / hot_inclusive ------------------------------------

#[test]
fn hot_leaves_counts_only_the_leaf_frame() {
    let trace = simple_trace();
    let report = analysis::hot_leaves(&trace, 10);
    assert!(matches!(report.mode, HotMode::Leaf), "expected Leaf mode");
    // 4 samples total (3 on b + 1 idle), all weight 1.
    assert_eq!(report.total_samples, 4, "total weight across samples");
    // Only `b` is ever a leaf; `a` is never the leaf.
    assert_eq!(report.rows.len(), 1, "exactly one leaf function");
    let row = &report.rows[0];
    assert_eq!(row.label, "b", "leaf label");
    assert_eq!(row.incl_samples, 3, "3 samples landed on b");
    assert_eq!(row.self_samples, 3, "leaf mode: self == incl");
    assert!(close(row.incl_pct, 75.0), "3/4 = 75%, got {}", row.incl_pct);
    assert!(close(row.self_pct, 75.0), "leaf self pct equals incl");
}

#[test]
fn hot_inclusive_counts_every_distinct_frame_on_stack() {
    let trace = simple_trace();
    let report = analysis::hot_inclusive(&trace, 10);
    assert!(
        matches!(report.mode, HotMode::Inclusive),
        "expected Inclusive mode"
    );
    let by_label: HashMap<&str, &analysis::HotRow> =
        report.rows.iter().map(|r| (r.label.as_str(), r)).collect();
    // Both a and b appear on the 3 sampled stacks.
    let a = by_label.get("a").expect("a present inclusively");
    let b = by_label.get("b").expect("b present inclusively");
    assert_eq!(a.incl_samples, 3, "a is on 3 stacks");
    assert_eq!(a.self_samples, 0, "a is never the leaf");
    assert_eq!(b.incl_samples, 3, "b is on 3 stacks");
    assert_eq!(b.self_samples, 3, "b is the leaf on all 3");
    assert!(close(a.self_pct, 0.0), "a never leaf ⇒ 0% self");
}

#[test]
fn hot_leaves_truncates_to_n_and_sorts_desc_with_label_tiebreak() {
    // Three leaves with counts 5, 3, 3. The two 3-count rows tie and
    // must order alphabetically (label ascending) as the tiebreak.
    let trace = build_trace(
        &["zebra", "alpha", "mid"],
        &[(0, Some(0)), (1, Some(0)), (2, Some(0))],
        &[Some(0)],
        &[(1, 0), (2, 1), (3, 2)],
        &[(None, 0), (None, 1), (None, 2)],
        &[
            (Some(0), 5), // zebra: 5
            (Some(1), 3), // alpha: 3
            (Some(2), 3), // mid:   3
        ],
        &["bin"],
    );
    let report = analysis::hot_leaves(&trace, 2);
    assert_eq!(report.rows.len(), 2, "truncated to n=2");
    assert_eq!(report.rows[0].label, "zebra", "highest count first");
    // Of the tied (alpha=3, mid=3) only alpha survives truncation —
    // it sorts first alphabetically.
    assert_eq!(report.rows[1].label, "alpha", "tie broken by label asc");
}

#[test]
fn hot_leaves_weights_samples() {
    // A single leaf hit by samples whose weights sum to 10.
    let trace = build_trace(
        &["leaf"],
        &[(0, Some(0))],
        &[Some(0)],
        &[(0, 0)],
        &[(None, 0)],
        &[(Some(0), 7), (Some(0), 3)],
        &["bin"],
    );
    let report = analysis::hot_leaves(&trace, 5);
    assert_eq!(report.total_samples, 10, "weights sum to 10");
    assert_eq!(report.rows[0].incl_samples, 10, "weighted leaf count");
    assert!(close(report.rows[0].self_pct, 100.0), "single leaf = 100%");
}

#[test]
fn hot_leaves_empty_trace_is_zeroed() {
    let trace = build_trace(&["f"], &[(0, None)], &[], &[(0, 0)], &[(None, 0)], &[], &[]);
    let report = analysis::hot_leaves(&trace, 5);
    assert_eq!(report.total_samples, 0, "no samples");
    assert!(report.rows.is_empty(), "no rows for empty sample stream");
    // Rendering an empty report must still succeed.
    assert!(
        !report.render_table().is_empty(),
        "empty report still renders a header"
    );
}

// ---- RowKind classification ----------------------------------------

#[test]
fn row_kind_unresolved_for_hex_label() {
    // A leaf whose label resolves to a hex address (empty string in
    // the string table ⇒ frame_label falls back to 0x<addr>).
    let trace = build_trace(
        &[""],
        &[(0, Some(0))],
        &[Some(0)],
        &[(0xdead, 0)],
        &[(None, 0)],
        &[(Some(0), 1)],
        &["bin"],
    );
    let report = analysis::hot_leaves(&trace, 5);
    assert_eq!(report.rows[0].label, "0xdead", "hex fallback label");
    assert_eq!(
        report.rows[0].kind,
        RowKind::Unresolved,
        "hex labels classify as Unresolved"
    );
    assert_eq!(RowKind::Unresolved.tag(), "??", "unresolved tag");
}

#[test]
fn row_kind_leaf_hot_for_pure_leaf() {
    // Single leaf = self == incl == 100% ⇒ LeafHot.
    let trace = simple_trace();
    let report = analysis::hot_leaves(&trace, 5);
    assert_eq!(report.rows[0].kind, RowKind::LeafHot, "self≈incl ⇒ LeafHot");
    assert_eq!(RowKind::LeafHot.tag(), "LH", "leaf-hot tag");
}

#[test]
fn row_kind_trampoline_high_incl_near_zero_self() {
    // Root frame `root` is on every one of 200 stacks (≈100% incl)
    // but is never itself the leaf (0% self) ⇒ Trampoline.
    let mut samples: Vec<(Option<usize>, u64)> = Vec::new();
    for _ in 0..200 {
        samples.push((Some(1), 1)); // leaf stack
    }
    let trace = build_trace(
        &["root", "worker"],
        &[(0, Some(0)), (1, Some(0))],
        &[Some(0)],
        &[(1, 0), (2, 1)],
        &[(None, 0), (Some(0), 1)],
        &samples,
        &["bin"],
    );
    let report = analysis::hot_inclusive(&trace, 10);
    let root = report
        .rows
        .iter()
        .find(|r| r.label == "root")
        .expect("root row");
    assert!(close(root.incl_pct, 100.0), "root on every stack");
    assert!(close(root.self_pct, 0.0), "root never the leaf");
    assert_eq!(root.kind, RowKind::Trampoline, "high incl, ~0 self");
    assert_eq!(RowKind::Trampoline.tag(), "EP", "trampoline tag");
}

#[test]
fn row_kind_hot_and_wrapper_by_leaf_ratio() {
    // Frame `mid` appears on 10 stacks inclusively; it is the leaf on
    // 6 of them (ratio 0.6 ⇒ Hot) in one trace, and on 2 of them
    // (ratio 0.2 ⇒ Wrapper) in another.
    //
    // Layout: root -> mid -> deep. Some samples land on `mid` (leaf),
    // some on `deep` (leaf); `mid` is inclusive on all.
    let make = |mid_leaf: u64, deep_leaf: u64| -> Trace {
        let mut samples: Vec<(Option<usize>, u64)> = Vec::new();
        for _ in 0..mid_leaf {
            samples.push((Some(1), 1)); // leaf = mid
        }
        for _ in 0..deep_leaf {
            samples.push((Some(2), 1)); // leaf = deep
        }
        build_trace(
            &["root", "mid", "deep"],
            &[(0, Some(0)), (1, Some(0)), (2, Some(0))],
            &[Some(0)],
            &[(1, 0), (2, 1), (3, 2)],
            // stack0=root, stack1=mid(prefix root), stack2=deep(prefix mid)
            &[(None, 0), (Some(0), 1), (Some(1), 2)],
            &samples,
            &["bin"],
        )
    };
    // 6 mid-leaf + 4 deep-leaf ⇒ mid: incl 10, self 6 ⇒ ratio 0.6 ⇒ Hot.
    let hot = analysis::hot_inclusive(&make(6, 4), 10);
    let mid_hot = hot.rows.iter().find(|r| r.label == "mid").expect("mid");
    assert_eq!(mid_hot.incl_samples, 10, "mid inclusive on all 10");
    assert_eq!(mid_hot.self_samples, 6, "mid is leaf on 6");
    assert_eq!(mid_hot.kind, RowKind::Hot, "ratio 0.6 ⇒ Hot");
    assert_eq!(RowKind::Hot.tag(), "HW", "hot tag");

    // 2 mid-leaf + 8 deep-leaf ⇒ mid: incl 10, self 2 ⇒ ratio 0.2 ⇒ Wrapper.
    let wrap = analysis::hot_inclusive(&make(2, 8), 10);
    let mid_wrap = wrap.rows.iter().find(|r| r.label == "mid").expect("mid");
    assert_eq!(mid_wrap.self_samples, 2, "mid leaf on 2");
    assert_eq!(mid_wrap.kind, RowKind::Wrapper, "ratio 0.2 ⇒ Wrapper");
    assert_eq!(RowKind::Wrapper.tag(), "WR", "wrapper tag");
}

#[test]
fn hot_report_render_contains_legend_and_columns() {
    let trace = simple_trace();
    let leaf = analysis::hot_leaves(&trace, 5).render_table();
    assert!(leaf.contains("HOT LEAF"), "leaf-mode title");
    assert!(leaf.contains("kind legend"), "legend appended");
    assert!(leaf.contains("function"), "function column header");
    let incl = analysis::hot_inclusive(&trace, 5).render_table();
    assert!(incl.contains("HOT INCLUSIVE"), "inclusive-mode title");
}

// ---- rollup --------------------------------------------------------

fn two_pattern_categorizer() -> Categorizer {
    let toml = r#"
[[categories]]
name = "scan"
patterns = ["aho_corasick", "aozora_scan"]

[[categories]]
name = "alloc"
patterns = ["malloc", "alloc::vec"]
"#;
    RollupConfig::from_toml(toml)
        .expect("toml parse")
        .compile()
        .expect("compile")
}

#[test]
fn rollup_buckets_leaves_into_categories_in_declaration_order() {
    // Leaves: aho_corasick::find (scan), malloc (alloc), my_own (unknown).
    let trace = build_trace(
        &["aho_corasick::find", "malloc", "my_own"],
        &[(0, Some(0)), (1, Some(0)), (2, Some(0))],
        &[Some(0)],
        &[(1, 0), (2, 1), (3, 2)],
        &[(None, 0), (None, 1), (None, 2)],
        &[
            (Some(0), 4), // scan
            (Some(1), 3), // alloc
            (Some(2), 1), // unknown
        ],
        &["bin"],
    );
    let report = analysis::rollup(&trace, &two_pattern_categorizer());
    assert_eq!(report.total_samples, 8, "4+3+1");
    // Declared categories come first in declaration order, even when
    // a category had hits ordered later.
    assert_eq!(report.rows[0].category, "scan", "first declared category");
    assert_eq!(report.rows[1].category, "alloc", "second declared");
    assert_eq!(report.rows[0].samples, 4, "scan samples");
    assert_eq!(report.rows[0].distinct_funcs, 1, "one scan function");
    assert!(close(report.rows[0].pct, 50.0), "4/8 = 50%");
    // `unknown` (leftover) is appended after declared categories.
    let unknown = report
        .rows
        .iter()
        .find(|r| r.category == "unknown")
        .expect("unknown bucket present");
    assert_eq!(unknown.samples, 1, "one unknown sample");
}

#[test]
fn rollup_emits_zero_sample_categories_for_stable_order() {
    // No leaf matches `alloc`; it must still be emitted with 0 samples.
    let trace = build_trace(
        &["aho_corasick::x"],
        &[(0, Some(0))],
        &[Some(0)],
        &[(1, 0)],
        &[(None, 0)],
        &[(Some(0), 2)],
        &["bin"],
    );
    let report = analysis::rollup(&trace, &two_pattern_categorizer());
    let alloc = report
        .rows
        .iter()
        .find(|r| r.category == "alloc")
        .expect("alloc still listed");
    assert_eq!(alloc.samples, 0, "no alloc samples");
    assert_eq!(alloc.distinct_funcs, 0, "no distinct alloc funcs");
    assert!(close(alloc.pct, 0.0), "0% for empty category");
}

#[test]
fn rollup_distinct_funcs_counts_unique_labels() {
    // Two distinct scan functions both bucket into `scan`.
    let trace = build_trace(
        &["aho_corasick::a", "aozora_scan::b"],
        &[(0, Some(0)), (1, Some(0))],
        &[Some(0)],
        &[(1, 0), (2, 1)],
        &[(None, 0), (None, 1)],
        &[(Some(0), 1), (Some(1), 1)],
        &["bin"],
    );
    let report = analysis::rollup(&trace, &two_pattern_categorizer());
    let scan = report
        .rows
        .iter()
        .find(|r| r.category == "scan")
        .expect("scan row");
    assert_eq!(scan.distinct_funcs, 2, "two distinct scan funcs");
    assert_eq!(scan.samples, 2, "summed samples");
}

#[test]
fn rollup_empty_trace_total_zero_and_renders() {
    let trace = build_trace(&["f"], &[(0, None)], &[], &[(0, 0)], &[(None, 0)], &[], &[]);
    let report = analysis::rollup(&trace, &two_pattern_categorizer());
    assert_eq!(report.total_samples, 0, "no samples");
    // Declared categories are still present, all zero.
    assert!(
        report.rows.iter().all(|r| r.samples == 0),
        "all categories zero"
    );
    assert!(report.render_table().contains("Category rollup"), "title");
}

// ---- categories (classify) -----------------------------------------

#[test]
fn categorizer_first_match_wins_in_declaration_order() {
    let cat = RollupConfig::aozora_defaults().compile().expect("compile");
    // AVX2 movemask is declared under scan ABOVE the generic
    // core_* buckets — first-match-wins must attribute it to phase1.
    assert_eq!(
        cat.classify("core::core_arch::x86::avx2::_mm256_movemask_epi8"),
        "scan",
        "avx2 intrinsic attributed to scan, not core_*"
    );
}

#[test]
fn categorizer_anchored_libc_patterns() {
    let cat = RollupConfig::aozora_defaults().compile().expect("compile");
    // `^malloc$` is anchored: bare `malloc` matches, `my_malloc` does not.
    assert_eq!(cat.classify("malloc"), "alloc_libc_heap", "anchored malloc");
    assert_ne!(
        cat.classify("my_malloc_wrapper"),
        "alloc_libc_heap",
        "anchored pattern does not match substring"
    );
    assert_eq!(
        cat.classify("__libc_malloc_impl"),
        "alloc_libc_heap",
        "prefix-anchored __libc_malloc"
    );
}

#[test]
fn categorizer_default_buckets_spot_checks() {
    let cat = RollupConfig::aozora_defaults().compile().expect("compile");
    let cases = [
        ("aozora_pipeline::lexer::sanitize::run", "sanitize"),
        ("aozora_pipeline::lexer::pair::pair", "pair"),
        ("aozora_pipeline::lexer::classify::recognise", "classify"),
        ("aozora_syntax::ast::intern::StrInterner::intern", "intern"),
        ("memchr::memmem::find", "memchr_scan"),
        ("encoding_rs::Decoder::decode", "corpus_load_sjis"),
        ("aozora_corpus::iter::walk", "corpus_walk"),
        ("bumpalo::Bump::alloc", "alloc_bumpalo_arena"),
        ("__memcpy_avx_unaligned", "alloc_memcpy_memmove"),
        ("alloc::vec::Vec::push", "alloc_rust_std"),
        ("aozora_render::html::emit", "rendering"),
        ("hashbrown::map::HashMap::insert", "hashing"),
        ("core::ptr::write_volatile", "core_ptr_ops"),
        ("core::fmt::Formatter::write_str", "core_misc"),
    ];
    for (name, expected) in cases {
        assert_eq!(cat.classify(name), expected, "classify {name}");
    }
    assert_eq!(
        cat.classify("totally::unknown::symbol"),
        "unknown",
        "fallback bucket"
    );
}

#[test]
fn categorizer_category_names_are_in_declaration_order() {
    let cat = RollupConfig::aozora_defaults().compile().expect("compile");
    let names = cat.category_names();
    assert_eq!(names.first().copied(), Some("scan"), "first category");
    assert!(
        names.contains(&"rendering"),
        "rendering category is present"
    );
}

#[test]
fn rollup_config_from_toml_errors_on_bad_shapes() {
    // Missing top-level [[categories]].
    let err = RollupConfig::from_toml("foo = 1").expect_err("must reject");
    assert!(
        err.to_string().contains("categories"),
        "error mentions categories: {err}"
    );
    // Category missing `name`.
    let err = RollupConfig::from_toml("[[categories]]\npatterns=[\"x\"]")
        .expect_err("missing name rejected");
    assert!(err.to_string().contains("name"), "mentions name: {err}");
    // Category missing `patterns`.
    let err = RollupConfig::from_toml("[[categories]]\nname=\"x\"")
        .expect_err("missing patterns rejected");
    assert!(
        err.to_string().contains("patterns"),
        "mentions patterns: {err}"
    );
}

#[test]
fn rollup_config_compile_rejects_bad_regex() {
    let cfg = RollupConfig::from_toml("[[categories]]\nname=\"x\"\npatterns=[\"(unclosed\"]")
        .expect("toml ok");
    let err = cfg.compile().expect_err("bad regex must fail to compile");
    assert!(err.to_string().contains("regex"), "regex error: {err}");
}

// ---- libs ----------------------------------------------------------

#[test]
fn library_distribution_attributes_by_leaf_frame_library() {
    // Leaf `b` in lib 0 (binA), leaf `c` in lib 1 (binB).
    let trace = build_trace(
        &["a", "b", "c"],
        &[(0, Some(0)), (1, Some(0)), (2, Some(1))],
        &[Some(0), Some(1)],
        &[(1, 0), (2, 1), (3, 2)],
        &[(None, 0), (Some(0), 1), (Some(0), 2)],
        &[
            (Some(1), 6), // leaf b ⇒ binA
            (Some(2), 2), // leaf c ⇒ binB
        ],
        &["binA", "binB"],
    );
    let report = analysis::library_distribution(&trace);
    assert_eq!(report.total_samples, 8, "6+2");
    // Sorted descending by samples ⇒ binA first.
    assert_eq!(report.rows[0].library, "binA", "binA dominates");
    assert_eq!(report.rows[0].samples, 6, "binA samples");
    assert!(close(report.rows[0].pct, 75.0), "6/8 = 75%");
    assert_eq!(report.rows[1].library, "binB", "binB second");
    assert!(close(report.rows[1].pct, 25.0), "2/8 = 25%");
}

#[test]
fn library_distribution_buckets_idle_and_unresolvable_as_unattributed() {
    // One sample idle (no stack), one leaf whose func has no resource.
    let trace = build_trace(
        &["a", "orphan"],
        &[(0, Some(0)), (1, None)], // orphan func: no resource ⇒ no lib
        &[Some(0)],
        &[(1, 0), (2, 1)],
        &[(None, 0), (None, 1)],
        &[
            (None, 1),    // idle ⇒ unattributed
            (Some(1), 1), // leaf orphan ⇒ unattributed (no lib)
        ],
        &["bin"],
    );
    let report = analysis::library_distribution(&trace);
    let unattr = report
        .rows
        .iter()
        .find(|r| r.library == "(unattributed)")
        .expect("unattributed bucket");
    assert_eq!(unattr.samples, 2, "both idle and orphan ⇒ unattributed");
    assert!(close(unattr.pct, 100.0), "all samples unattributed");
}

#[test]
fn library_distribution_ties_break_by_name() {
    // Two libs with equal sample counts ⇒ alphabetical order.
    let trace = build_trace(
        &["x", "y"],
        &[(0, Some(0)), (1, Some(1))],
        &[Some(0), Some(1)],
        &[(1, 0), (2, 1)],
        &[(None, 0), (None, 1)],
        &[(Some(0), 1), (Some(1), 1)],
        &["zlib", "alib"],
    );
    let report = analysis::library_distribution(&trace);
    assert_eq!(report.rows[0].library, "alib", "tie ⇒ name asc");
    assert_eq!(report.rows[1].library, "zlib", "second by name");
    assert!(
        report.render_table().contains("Library distribution"),
        "title"
    );
}

// ---- stacks (matching_stacks) --------------------------------------

#[test]
fn matching_stacks_groups_identical_stacks_and_filters() {
    // Two distinct stacks: [b,a] (matches "b") seen twice, and [c,a]
    // (no "b") seen once. Filter = "b".
    let trace = build_trace(
        &["a", "b", "c"],
        &[(0, Some(0)), (1, Some(0)), (2, Some(0))],
        &[Some(0)],
        &[(1, 0), (2, 1), (3, 2)],
        // stack0=a, stack1=b(prefix a), stack2=c(prefix a)
        &[(None, 0), (Some(0), 1), (Some(0), 2)],
        &[
            (Some(1), 1), // [b,a]
            (Some(1), 1), // [b,a] again
            (Some(2), 1), // [c,a] — no b
        ],
        &["bin"],
    );
    let re = Regex::new("^b$").expect("regex");
    let report = analysis::matching_stacks(&trace, &re, 10);
    assert_eq!(report.total_samples, 3, "3 samples total");
    assert_eq!(report.matched_samples, 2, "only the two b-stacks match");
    assert_eq!(report.stacks.len(), 1, "identical b-stacks merged");
    let top = &report.stacks[0];
    assert_eq!(top.samples, 2, "merged sample count");
    // Frames are leaf-first: b then a.
    assert_eq!(top.frames, vec!["b", "a"], "leaf-first frame order");
    assert!(close(top.pct, 2.0 / 3.0 * 100.0), "2/3 of total");
}

#[test]
fn matching_stacks_matches_non_leaf_frame() {
    // Stack [leaf, mid, root]; filter on `mid` (an interior frame).
    let trace = build_trace(
        &["root", "mid", "leaf"],
        &[(0, Some(0)), (1, Some(0)), (2, Some(0))],
        &[Some(0)],
        &[(1, 0), (2, 1), (3, 2)],
        &[(None, 0), (Some(0), 1), (Some(1), 2)],
        &[(Some(2), 1)],
        &["bin"],
    );
    let re = Regex::new("mid").expect("regex");
    let report = analysis::matching_stacks(&trace, &re, 10);
    assert_eq!(report.matched_samples, 1, "interior frame matched");
    assert_eq!(
        report.stacks[0].frames,
        vec!["leaf", "mid", "root"],
        "full leaf-first chain"
    );
}

#[test]
fn matching_stacks_no_match_yields_empty() {
    let trace = simple_trace();
    let re = Regex::new("does_not_exist").expect("regex");
    let report = analysis::matching_stacks(&trace, &re, 10);
    assert_eq!(report.matched_samples, 0, "nothing matched");
    assert!(report.stacks.is_empty(), "no stacks");
    assert!(report.total_samples > 0, "but total still counted");
    // Render must show the 0.00% line without panicking.
    assert!(report.render_table().contains("0.00%"), "zero-pct line");
}

#[test]
fn matching_stacks_truncates_to_limit() {
    // Three distinct matching stacks; limit to 2 keeps the heaviest.
    let trace = build_trace(
        &["root", "p", "q", "r"],
        &[(0, Some(0)), (1, Some(0)), (2, Some(0)), (3, Some(0))],
        &[Some(0)],
        &[(1, 0), (2, 1), (3, 2), (4, 3)],
        // stacks p,q,r all child of root
        &[(None, 0), (Some(0), 1), (Some(0), 2), (Some(0), 3)],
        &[
            (Some(1), 5), // [p,root]
            (Some(2), 3), // [q,root]
            (Some(3), 1), // [r,root]
        ],
        &["bin"],
    );
    // Match everything (all frame labels are non-empty words).
    let re = Regex::new("root").expect("regex");
    let report = analysis::matching_stacks(&trace, &re, 2);
    assert_eq!(report.stacks.len(), 2, "truncated to limit 2");
    assert_eq!(report.stacks[0].samples, 5, "heaviest first");
    assert_eq!(report.stacks[1].samples, 3, "second heaviest");
    assert!(
        report.render_table().contains("#1"),
        "renders ranked stacks"
    );
}

// ---- flame (folded_stacks / render_folded) -------------------------

#[test]
fn folded_stacks_are_root_first_and_deduplicated() {
    // Stack [b(leaf), a(root)] hit twice ⇒ one folded row "a;b 2".
    let trace = simple_trace();
    let folded = analysis::folded_stacks(&trace);
    // simple_trace has 3 samples on the same b-stack (the idle sample
    // is dropped).
    assert_eq!(folded.len(), 1, "one distinct folded stack");
    assert_eq!(folded[0].stack, vec!["a", "b"], "root-first order");
    assert_eq!(folded[0].samples, 3, "summed weight");
    let text = analysis::render_folded(&folded);
    assert_eq!(text, "a;b 3\n", "folded line format");
}

#[test]
fn folded_stacks_sorted_descending_by_weight() {
    let trace = build_trace(
        &["root", "heavy", "light"],
        &[(0, Some(0)), (1, Some(0)), (2, Some(0))],
        &[Some(0)],
        &[(1, 0), (2, 1), (3, 2)],
        &[(None, 0), (Some(0), 1), (Some(0), 2)],
        &[
            (Some(1), 2), // [root, heavy]
            (Some(2), 5), // [root, light]
        ],
        &["bin"],
    );
    let folded = analysis::folded_stacks(&trace);
    assert_eq!(folded.len(), 2, "two distinct stacks");
    assert_eq!(folded[0].samples, 5, "heaviest folded first");
    assert_eq!(folded[0].stack, vec!["root", "light"], "the 5-weight stack");
    assert_eq!(folded[1].samples, 2, "lighter second");
}

#[test]
fn render_folded_sanitises_semicolons_in_labels() {
    // A label containing `;` would corrupt the folded format; it must
    // be rewritten to `:`.
    let trace = build_trace(
        &["a;b"],
        &[(0, Some(0))],
        &[Some(0)],
        &[(0, 0)],
        &[(None, 0)],
        &[(Some(0), 1)],
        &["bin"],
    );
    let folded = analysis::folded_stacks(&trace);
    let text = analysis::render_folded(&folded);
    assert_eq!(text, "a:b 1\n", "semicolon in label rewritten to colon");
}

#[test]
fn render_folded_empty_is_empty_string() {
    let trace = build_trace(&["f"], &[(0, None)], &[], &[(0, 0)], &[(None, 0)], &[], &[]);
    let folded = analysis::folded_stacks(&trace);
    assert!(folded.is_empty(), "no stacks for empty sample stream");
    assert_eq!(analysis::render_folded(&folded), "", "empty render");
}

// ---- compare -------------------------------------------------------

#[test]
fn compare_classifies_shifted_appeared_disappeared() {
    // before: leaves f(8), g(2)  ⇒ f=80%, g=20%
    // after:  leaves f(2), h(8)  ⇒ f=20%, h=80%
    // ⇒ f shifted (-60), g disappeared (-20), h appeared (+80).
    let before = build_trace(
        &["f", "g"],
        &[(0, Some(0)), (1, Some(0))],
        &[Some(0)],
        &[(1, 0), (2, 1)],
        &[(None, 0), (None, 1)],
        &[(Some(0), 8), (Some(1), 2)],
        &["bin"],
    );
    let after = build_trace(
        &["f", "h"],
        &[(0, Some(0)), (1, Some(0))],
        &[Some(0)],
        &[(1, 0), (2, 1)],
        &[(None, 0), (None, 1)],
        &[(Some(0), 2), (Some(1), 8)],
        &["bin"],
    );
    let report = analysis::compare(&before, &after, 10);
    assert_eq!(report.before_total, 10, "before total");
    assert_eq!(report.after_total, 10, "after total");
    let by: HashMap<&str, &analysis::ComparisonRow> =
        report.rows.iter().map(|r| (r.label.as_str(), r)).collect();

    // `ChangeStatus` is not re-exported from the crate, so status is
    // asserted via the rendered text (see
    // `compare_render_shows_status_and_signed_delta`). Here we pin the
    // numeric percentages/deltas that *determine* the status.
    let f = by.get("f").expect("f present in both");
    assert!(close(f.before_pct, 80.0), "f before 80%");
    assert!(close(f.after_pct, 20.0), "f after 20%");
    assert!(close(f.delta_pct, -60.0), "f delta -60");

    let g = by.get("g").expect("g only-before (disappeared)");
    assert!(close(g.before_pct, 20.0), "g before 20%");
    assert!(close(g.after_pct, 0.0), "g after 0% ⇒ disappeared");

    let h = by.get("h").expect("h only-after (appeared)");
    assert!(close(h.before_pct, 0.0), "h before 0% ⇒ appeared");
    assert!(close(h.delta_pct, 80.0), "h delta +80");
}

#[test]
fn compare_sorts_by_abs_delta_and_truncates() {
    // before f=50,g=50 ; after f=90,g=10 ⇒ |Δf|=40 > |Δg|=40 (equal),
    // build an asymmetric case so the order is unambiguous:
    // before f=80,g=20 ; after f=90,g=10 ⇒ |Δf|=10, |Δg|=10 still tie.
    // Use three funcs for a clear ranking.
    let before = build_trace(
        &["f", "g", "k"],
        &[(0, Some(0)), (1, Some(0)), (2, Some(0))],
        &[Some(0)],
        &[(1, 0), (2, 1), (3, 2)],
        &[(None, 0), (None, 1), (None, 2)],
        &[(Some(0), 5), (Some(1), 4), (Some(2), 1)], // 50/40/10
        &["bin"],
    );
    let after = build_trace(
        &["f", "g", "k"],
        &[(0, Some(0)), (1, Some(0)), (2, Some(0))],
        &[Some(0)],
        &[(1, 0), (2, 1), (3, 2)],
        &[(None, 0), (None, 1), (None, 2)],
        &[(Some(0), 1), (Some(1), 4), (Some(2), 5)], // 10/40/50
        &["bin"],
    );
    let report = analysis::compare(&before, &after, 1);
    assert_eq!(report.rows.len(), 1, "truncated to top=1");
    // f: 50→10 (Δ -40), k: 10→50 (Δ +40), g: 40→40 (Δ 0).
    // |Δf| == |Δk| == 40 — whichever wins, it must be one of them,
    // and definitely not g (Δ 0).
    let top = &report.rows[0];
    assert!(
        top.label == "f" || top.label == "k",
        "top row is a 40-point mover, got {}",
        top.label
    );
    assert!(close(top.delta_pct.abs(), 40.0), "top |Δ| is 40");
}

#[test]
fn compare_render_shows_status_and_signed_delta() {
    let before = build_trace(
        &["f"],
        &[(0, Some(0))],
        &[Some(0)],
        &[(1, 0)],
        &[(None, 0)],
        &[(Some(0), 1)],
        &["bin"],
    );
    let after = build_trace(
        &["g"],
        &[(0, Some(0))],
        &[Some(0)],
        &[(1, 0)],
        &[(None, 0)],
        &[(Some(0), 1)],
        &["bin"],
    );
    let text = analysis::compare(&before, &after, 10).render_table();
    assert!(text.contains("Trace comparison"), "title");
    assert!(text.contains("appeared"), "g appeared");
    assert!(text.contains("disappeared"), "f disappeared");
    // Signed delta formatting: appeared (+100.00) and disappeared (-100.00).
    assert!(text.contains("+100.00"), "positive delta has + sign");
    assert!(text.contains("-100.00"), "negative delta has - sign");
}

#[test]
fn compare_identical_traces_show_zero_delta() {
    let trace = simple_trace();
    let report = analysis::compare(&trace, &trace, 10);
    assert!(
        report.rows.iter().all(|r| close(r.delta_pct, 0.0)),
        "identical traces ⇒ zero deltas"
    );
    // Every row is present on both sides (before_pct > 0 and after_pct > 0),
    // which is the precondition for the Shifted classification.
    assert!(
        report
            .rows
            .iter()
            .all(|r| r.before_pct > 0.0 && r.after_pct > 0.0),
        "every row present in both ⇒ shifted"
    );
}

// ---- load (synthetic gecko JSON) -----------------------------------

#[test]
fn load_missing_libs_field_errors() {
    let json = json!({ "threads": [] });
    let err = Trace::from_json(&json, PathBuf::from("x")).expect_err("missing libs");
    assert!(
        err.to_string().contains("libs"),
        "field name in error: {err}"
    );
}

#[test]
fn load_missing_threads_field_errors() {
    let json = json!({ "libs": [] });
    let err = Trace::from_json(&json, PathBuf::from("x")).expect_err("missing threads");
    assert!(
        err.to_string().contains("threads"),
        "field name in error: {err}"
    );
}

#[test]
fn load_stack_table_column_length_mismatch_errors() {
    let json = json!({
        "libs": [],
        "threads": [{
            "name": "t",
            "stringArray": ["x"],
            // prefix has 2 entries, frame has 1 ⇒ mismatch.
            "stackTable": { "prefix": [null, null], "frame": [0] },
            "frameTable": { "address": [0], "func": [0] },
            "funcTable": { "name": [0], "resource": [null] },
            "resourceTable": { "lib": [null] },
            "samples": { "stack": [], "weight": [] },
        }],
    });
    let err = Trace::from_json(&json, PathBuf::from("x")).expect_err("mismatch");
    assert!(
        err.to_string().contains("stackTable"),
        "mismatch field: {err}"
    );
}

#[test]
fn load_frame_table_column_length_mismatch_errors() {
    let json = json!({
        "libs": [],
        "threads": [{
            "name": "t",
            "stringArray": ["x"],
            "stackTable": { "prefix": [null], "frame": [0] },
            // address 2 entries, func 1 ⇒ mismatch.
            "frameTable": { "address": [0, 1], "func": [0] },
            "funcTable": { "name": [0], "resource": [null] },
            "resourceTable": { "lib": [null] },
            "samples": { "stack": [], "weight": [] },
        }],
    });
    let err = Trace::from_json(&json, PathBuf::from("x")).expect_err("frame mismatch");
    assert!(
        err.to_string().contains("frameTable"),
        "frame mismatch field: {err}"
    );
}

#[test]
fn load_func_table_column_length_mismatch_errors() {
    let json = json!({
        "libs": [],
        "threads": [{
            "name": "t",
            "stringArray": ["x"],
            "stackTable": { "prefix": [null], "frame": [0] },
            "frameTable": { "address": [0], "func": [0] },
            // name 2 entries, resource 1 ⇒ mismatch.
            "funcTable": { "name": [0, 0], "resource": [null] },
            "resourceTable": { "lib": [null] },
            "samples": { "stack": [], "weight": [] },
        }],
    });
    let err = Trace::from_json(&json, PathBuf::from("x")).expect_err("func mismatch");
    assert!(
        err.to_string().contains("funcTable"),
        "func mismatch field: {err}"
    );
}

#[test]
fn load_defaults_weight_to_one_when_column_absent() {
    // No `weight` column ⇒ every sample weighs 1.
    let json = json!({
        "libs": [{ "name": "bin", "path": "/bin", "codeId": "abc" }],
        "threads": [{
            "name": "t",
            "tid": 7,
            "isMainThread": true,
            "stringArray": ["leaf"],
            "stackTable": { "prefix": [null], "frame": [0] },
            "frameTable": { "address": [16], "func": [0] },
            "funcTable": { "name": [0], "resource": [0] },
            "resourceTable": { "lib": [0] },
            "samples": { "stack": [0, 0, 0] },
        }],
    });
    let trace = Trace::from_json(&json, PathBuf::from("x")).expect("load");
    assert_eq!(trace.total_samples(), 3, "3 samples regardless of weight");
    let report = analysis::hot_leaves(&trace, 5);
    assert_eq!(report.total_samples, 3, "default weight 1 each");
    // Library + thread metadata parsed.
    assert_eq!(trace.libs[0].name, "bin", "library name parsed");
    assert_eq!(trace.libs[0].code_id, "abc", "codeId parsed");
    assert_eq!(trace.threads[0].tid, 7, "tid parsed");
    assert!(trace.threads[0].is_main, "isMainThread parsed");
}

#[test]
fn load_null_stack_index_is_idle_sample() {
    // A sample with a null stack is idle — counted in total but landing
    // nowhere.
    let json = json!({
        "libs": [],
        "threads": [{
            "name": "t",
            "stringArray": ["leaf"],
            "stackTable": { "prefix": [null], "frame": [0] },
            "frameTable": { "address": [0], "func": [0] },
            "funcTable": { "name": [0], "resource": [null] },
            "resourceTable": { "lib": [null] },
            "samples": { "stack": [null, 0], "weight": [1, 1] },
        }],
    });
    let trace = Trace::from_json(&json, PathBuf::from("x")).expect("load");
    assert_eq!(trace.threads[0].samples.len(), 2, "two samples");
    assert!(
        trace.threads[0].samples[0].stack_idx.is_none(),
        "first sample idle"
    );
    assert_eq!(
        trace.threads[0].samples[1].stack_idx,
        Some(0),
        "second sample has stack 0"
    );
}

#[test]
fn trace_is_fully_symbolicated_reflects_resolved_state() {
    let trace = simple_trace();
    // Fresh load: nothing resolved.
    assert!(
        !trace.is_fully_symbolicated(),
        "fresh trace is unsymbolicated"
    );
    // frame_label falls back to the raw string-table name.
    assert_eq!(trace.threads[0].frame_label(1), "b", "raw label fallback");
    // frame_address reads the frame table.
    assert_eq!(trace.threads[0].frame_address(1), 0x20, "frame address");
}

#[test]
fn load_samples_time_length_mismatch_errors() {
    // `time` column shorter than the `stack` column ⇒ length mismatch.
    let json = json!({
        "libs": [],
        "threads": [{
            "name": "t",
            "stringArray": ["x"],
            "stackTable": { "prefix": [null], "frame": [0] },
            "frameTable": { "address": [0], "func": [0] },
            "funcTable": { "name": [0], "resource": [null] },
            "resourceTable": { "lib": [null] },
            // 2 stacks but only 1 timestamp ⇒ mismatch.
            "samples": { "stack": [0, 0], "time": [1.0] },
        }],
    });
    let err = Trace::from_json(&json, PathBuf::from("x")).expect_err("time mismatch");
    assert!(
        err.to_string().contains("samples.time"),
        "time length mismatch field: {err}"
    );
}

/// Minimal valid gecko profile JSON as a string, for the on-disk
/// loader tests below.
const MINIMAL_TRACE_JSON: &str = r#"{
  "libs": [{ "name": "bin", "path": "/bin" }],
  "threads": [{
    "name": "t",
    "stringArray": ["leaf"],
    "stackTable": { "prefix": [null], "frame": [0] },
    "frameTable": { "address": [16], "func": [0] },
    "funcTable": { "name": [0], "resource": [0] },
    "resourceTable": { "lib": [0] },
    "samples": { "stack": [0, 0], "weight": [1, 1] }
  }]
}"#;

#[test]
fn load_plain_json_file_from_disk() {
    let path = env::temp_dir().join(format!("aozora-trace-load-{}.json", process::id()));
    fs::write(&path, MINIMAL_TRACE_JSON).expect("write trace json");
    let trace = Trace::load(&path).expect("load plain json");
    assert_eq!(trace.total_samples(), 2, "two samples loaded from disk");
    assert_eq!(trace.libs[0].name, "bin", "library parsed from disk");
    drop(fs::remove_file(&path));
}

#[test]
fn load_gzipped_json_file_from_disk() {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write as _;

    let path = env::temp_dir().join(format!("aozora-trace-load-{}.json.gz", process::id()));
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(MINIMAL_TRACE_JSON.as_bytes())
        .expect("gz write");
    let gz_bytes = encoder.finish().expect("gz finish");
    fs::write(&path, gz_bytes).expect("write gz trace");

    let trace = Trace::load(&path).expect("load gzipped json");
    assert_eq!(trace.total_samples(), 2, "two samples from gzipped trace");
    drop(fs::remove_file(&path));
}

#[test]
fn load_nonexistent_file_is_io_error() {
    let path = env::temp_dir().join("aozora-trace-definitely-missing-trace.json");
    drop(fs::remove_file(&path)); // ensure absent
    let err = Trace::load(&path).expect_err("missing file must error");
    assert!(
        err.to_string().contains("io error"),
        "io error variant: {err}"
    );
}

#[test]
fn load_malformed_json_file_is_json_error() {
    let path = env::temp_dir().join(format!("aozora-trace-bad-{}.json", process::id()));
    fs::write(&path, "{ not valid json").expect("write bad json");
    let err = Trace::load(&path).expect_err("malformed json must error");
    assert!(
        err.to_string().contains("json parse error"),
        "json error variant: {err}"
    );
    drop(fs::remove_file(&path));
}

#[test]
fn rollup_config_from_toml_file_reads_disk() {
    let toml = "[[categories]]\nname = \"scan\"\npatterns = [\"aho_corasick\"]\n";
    let path = env::temp_dir().join(format!("aozora-trace-cats-{}.toml", process::id()));
    fs::write(&path, toml).expect("write toml");
    let cfg = RollupConfig::from_toml_file(&path).expect("read toml file");
    let cat = cfg.compile().expect("compile");
    assert_eq!(
        cat.classify("aho_corasick::find"),
        "scan",
        "from-file config"
    );
    drop(fs::remove_file(&path));
}

// ---- cache (sidecar symbol cache) ----------------------------------

#[test]
fn cache_record_then_apply_resolves_matching_frames() {
    let mut trace = simple_trace();
    let lib_name = trace.libs[0].name.clone();
    let debug_id = trace.libs[0].debug_id.clone();
    let addr = trace.threads[0].frame_address(1); // leaf b address

    let mut cache = SymbolCache::default();
    cache.record(
        LibIdent {
            name: &lib_name,
            debug_id: &debug_id,
        },
        addr,
        "demangled::b".to_owned(),
    );
    let resolved = cache.apply(&mut trace);
    assert_eq!(resolved, 1, "one frame resolved by address");
    assert!(trace.threads[0].resolved[1].is_some(), "frame 1 resolved");
    // frame_label now prefers the resolved name.
    assert_eq!(
        trace.threads[0].frame_label(1),
        "demangled::b",
        "resolved label wins over raw"
    );
}

#[test]
fn cache_apply_skips_debug_id_mismatch() {
    let mut trace = simple_trace();
    // Give the trace lib a non-empty debug_id that the cache won't match.
    trace.libs[0].debug_id = "TRACE_ID".to_owned();
    let lib_name = trace.libs[0].name.clone();
    let addr = trace.threads[0].frame_address(1);

    let mut cache = SymbolCache::default();
    cache.record(
        LibIdent {
            name: &lib_name,
            debug_id: "OTHER_ID",
        },
        addr,
        "wrong".to_owned(),
    );
    let resolved = cache.apply(&mut trace);
    assert_eq!(resolved, 0, "mismatched debug_id ⇒ no resolution");
    assert!(
        trace.threads[0].resolved[1].is_none(),
        "frame stays unresolved"
    );
}

#[test]
fn cache_apply_allows_empty_debug_id() {
    // An empty cached debug_id is treated as a wildcard (matches any).
    let mut trace = simple_trace();
    trace.libs[0].debug_id = "ANYTHING".to_owned();
    let lib_name = trace.libs[0].name.clone();
    let addr = trace.threads[0].frame_address(1);

    let mut cache = SymbolCache::default();
    cache.record(
        LibIdent {
            name: &lib_name,
            debug_id: "", // wildcard
        },
        addr,
        "ok".to_owned(),
    );
    assert_eq!(cache.apply(&mut trace), 1, "empty cached id is a wildcard");
}

#[test]
fn cache_write_load_round_trip() {
    let mut cache = SymbolCache::default();
    cache.record(
        LibIdent {
            name: "bin",
            debug_id: "ID",
        },
        0x40,
        "func_at_0x40".to_owned(),
    );
    let dir = env::temp_dir();
    let path = dir.join(format!(
        "aozora-trace-cache-test-{}.symbols.json",
        process::id()
    ));
    cache.write(&path).expect("write cache");
    let loaded = SymbolCache::load(&path)
        .expect("load cache")
        .expect("cache present");
    let lib = loaded.libs.get("bin").expect("bin entry");
    assert_eq!(lib.debug_id, "ID", "debug_id round-trips");
    assert_eq!(
        lib.by_address.get("64").map(String::as_str),
        Some("func_at_0x40"),
        "address keyed as decimal string"
    );
    drop(fs::remove_file(&path));
}

#[test]
fn cache_load_missing_file_is_none() {
    let path = env::temp_dir().join("aozora-trace-cache-definitely-absent.symbols.json");
    drop(fs::remove_file(&path)); // ensure absent
    let result = SymbolCache::load(&path).expect("load is Ok");
    assert!(result.is_none(), "absent file ⇒ Ok(None)");
}

#[test]
fn cache_record_is_idempotent_and_updates_debug_id() {
    let mut cache = SymbolCache::default();
    cache.record(
        LibIdent {
            name: "bin",
            debug_id: "OLD",
        },
        0x10,
        "first".to_owned(),
    );
    // Re-record same lib with a new debug_id + a second address.
    cache.record(
        LibIdent {
            name: "bin",
            debug_id: "NEW",
        },
        0x20,
        "second".to_owned(),
    );
    let lib = cache.libs.get("bin").expect("bin entry");
    assert_eq!(lib.debug_id, "NEW", "debug_id replaced on re-record");
    assert_eq!(lib.by_address.len(), 2, "both addresses retained");
}

#[test]
fn cache_sidecar_path_strips_gz_and_appends_symbols_json() {
    let p = SymbolCache::sidecar_path_for(Path::new("/tmp/trace.json.gz"));
    assert_eq!(
        p,
        PathBuf::from("/tmp/trace.symbols.json"),
        "gz stripped, stem + .symbols.json"
    );
    // Plain .json (no .gz) path.
    let p2 = SymbolCache::sidecar_path_for(Path::new("/tmp/trace.json"));
    assert_eq!(
        p2,
        PathBuf::from("/tmp/trace.symbols.json"),
        "plain json ⇒ stem + .symbols.json"
    );
}

// ---- JSON serialization (`xtask trace … --format json`) ------------
//
// Every report derives `Serialize`; the `--format json` path emits the
// same typed data. These pin the wire shape: struct fields are
// camelCase, unit enums are snake_case, and the output round-trips.

#[test]
fn hot_report_serializes_camelcase_fields_and_snakecase_enums() {
    let report = analysis::hot_leaves(&simple_trace(), 5);
    let v = serde_json::to_value(&report).expect("HotReport serializes");
    assert_eq!(v["mode"], json!("leaf"), "HotMode ⇒ snake_case");
    assert_eq!(v["totalSamples"], json!(4), "total_samples ⇒ totalSamples");
    let row = &v["rows"][0];
    assert_eq!(row["label"], json!("b"));
    assert_eq!(row["kind"], json!("leaf_hot"), "RowKind ⇒ snake_case");
    assert!(
        row.get("inclSamples").is_some(),
        "incl_samples ⇒ inclSamples"
    );
    assert!(row.get("selfPct").is_some(), "self_pct ⇒ selfPct");
    assert!(
        row.get("incl_samples").is_none(),
        "no snake_case field leaks through"
    );
}

#[test]
fn hot_inclusive_mode_serializes_as_inclusive() {
    let v = serde_json::to_value(analysis::hot_inclusive(&simple_trace(), 5)).expect("serializes");
    assert_eq!(v["mode"], json!("inclusive"));
}

#[test]
fn rollup_report_serializes_distinct_funcs_as_camelcase() {
    let trace = build_trace(
        &["aho_corasick::find"],
        &[(0, Some(0))],
        &[Some(0)],
        &[(1, 0)],
        &[(None, 0)],
        &[(Some(0), 4)],
        &["bin"],
    );
    let v = serde_json::to_value(analysis::rollup(&trace, &two_pattern_categorizer()))
        .expect("RollupReport serializes");
    assert_eq!(v["totalSamples"], json!(4));
    let scan = v["rows"]
        .as_array()
        .expect("rows array")
        .iter()
        .find(|r| r["category"] == json!("scan"))
        .expect("scan row");
    assert_eq!(
        scan["distinctFuncs"],
        json!(1),
        "distinct_funcs ⇒ camelCase"
    );
    assert_eq!(scan["samples"], json!(4));
}

#[test]
fn library_report_serializes_camelcase() {
    let trace = build_trace(
        &["a", "b"],
        &[(0, Some(0)), (1, Some(0))],
        &[Some(0)],
        &[(1, 0), (2, 1)],
        &[(None, 0), (Some(0), 1)],
        &[(Some(1), 6)],
        &["binA"],
    );
    let v = serde_json::to_value(analysis::library_distribution(&trace)).expect("serializes");
    assert_eq!(v["totalSamples"], json!(6));
    assert_eq!(v["rows"][0]["library"], json!("binA"));
    assert_eq!(v["rows"][0]["samples"], json!(6));
}

#[test]
fn matched_stacks_report_serializes_camelcase() {
    let trace = build_trace(
        &["a", "b"],
        &[(0, Some(0)), (1, Some(0))],
        &[Some(0)],
        &[(1, 0), (2, 1)],
        &[(None, 0), (Some(0), 1)],
        &[(Some(1), 2)],
        &["bin"],
    );
    let re = Regex::new("^b$").expect("regex");
    let v = serde_json::to_value(analysis::matching_stacks(&trace, &re, 10)).expect("serializes");
    assert_eq!(v["filter"], json!("^b$"));
    assert_eq!(v["totalSamples"], json!(2));
    assert_eq!(v["matchedSamples"], json!(2), "matched_samples ⇒ camelCase");
    assert_eq!(v["stacks"][0]["frames"], json!(["b", "a"]), "leaf-first");
}

#[test]
fn comparison_report_serializes_status_and_camelcase_totals() {
    let before = build_trace(
        &["f"],
        &[(0, Some(0))],
        &[Some(0)],
        &[(1, 0)],
        &[(None, 0)],
        &[(Some(0), 1)],
        &["bin"],
    );
    let after = build_trace(
        &["g"],
        &[(0, Some(0))],
        &[Some(0)],
        &[(1, 0)],
        &[(None, 0)],
        &[(Some(0), 1)],
        &["bin"],
    );
    let v = serde_json::to_value(analysis::compare(&before, &after, 10)).expect("serializes");
    assert_eq!(v["beforeTotal"], json!(1), "before_total ⇒ beforeTotal");
    assert_eq!(v["afterTotal"], json!(1), "after_total ⇒ afterTotal");
    let statuses: Vec<&Value> = v["rows"]
        .as_array()
        .expect("rows array")
        .iter()
        .map(|r| &r["status"])
        .collect();
    assert!(statuses.contains(&&json!("appeared")), "g appeared");
    assert!(statuses.contains(&&json!("disappeared")), "f disappeared");
}

#[test]
fn folded_stacks_serialize_as_stack_and_samples() {
    let v = serde_json::to_value(analysis::folded_stacks(&simple_trace())).expect("serializes");
    assert_eq!(v[0]["stack"], json!(["a", "b"]), "root-first stack array");
    assert_eq!(v[0]["samples"], json!(3));
}

#[test]
fn report_round_trips_through_pretty_json_string() {
    // `emit` uses `to_string_pretty`; the output must re-parse.
    let s =
        serde_json::to_string_pretty(&analysis::hot_leaves(&simple_trace(), 5)).expect("pretty");
    let parsed: Value = serde_json::from_str(&s).expect("re-parses");
    assert_eq!(parsed["totalSamples"], json!(4));
}
