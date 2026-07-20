//! Fixed workloads measured by the repository's Callgrind performance gate.

use std::{env, hint::black_box, process::ExitCode};

use aozora::TextEdit;
use aozora_bench::{build_pathological_aozora, build_synthetic_aozora};
use iai_callgrind::client_requests::callgrind::toggle_collect;

const TOSA: &str = include_str!("../../aozora-conformance/fixtures/works/terada-tosa/source.txt");
const MATOI: &str =
    include_str!("../../aozora-conformance/fixtures/works/orikuchi-matoi/source.txt");
const PETER: &str = include_str!("../../aozora-conformance/fixtures/works/potter-peter/source.txt");
const PATHOLOGICAL_BYTES: usize = 64 * 1024;
const EDIT_BYTES: usize = 256 * 1024;
const LARGE_BYTES: usize = 1024 * 1024;
const EDIT_ITERATIONS: usize = 8;

fn parse(src: &str) -> usize {
    measured(|| {
        black_box(
            aozora::parse(black_box(src))
                .expect("source fits parser span limit")
                .snapshot()
                .diagnostics()
                .len(),
        )
    })
}

fn parse_then_html(src: &str) -> usize {
    measured(|| {
        black_box(
            aozora::parse(black_box(src))
                .expect("source fits parser span limit")
                .snapshot()
                .to_html()
                .len(),
        )
    })
}

fn full_edit_source() -> usize {
    let source = build_synthetic_aozora(EDIT_BYTES);
    measured(|| {
        let mut total = 0;
        for _ in 0..EDIT_ITERATIONS {
            let mut edited = source.clone();
            let at = edit_offset(&edited);
            edited.insert(at, 'x');
            total += parse_unmeasured(&edited);
        }
        total
    })
}

fn full_multiple_edit_source() -> usize {
    let mut source = build_synthetic_aozora(EDIT_BYTES);
    measured(|| {
        let mut total = 0;
        for _ in 0..EDIT_ITERATIONS {
            let first = source.find("\n\n").expect("paragraph boundary") + 2;
            let second = source.rfind("\n\n").expect("paragraph boundary");
            source.insert(second, 'y');
            source.insert(first, 'x');
            total += parse_unmeasured(&source);
        }
        total
    })
}

fn single_edit() -> usize {
    let source = build_synthetic_aozora(EDIT_BYTES);
    let at = edit_offset(&source);
    let mut document = aozora::parse(source).expect("source fits parser span limit");
    measured(|| {
        for _ in 0..EDIT_ITERATIONS {
            document
                .edit([TextEdit::new(at..at, "x")])
                .expect("valid insertion");
        }
        black_box(document.snapshot().diagnostics().len())
    })
}

fn multiple_edits() -> usize {
    let source = build_synthetic_aozora(EDIT_BYTES);
    let mut document = aozora::parse(source).expect("source fits parser span limit");
    measured(|| {
        for _ in 0..EDIT_ITERATIONS {
            let first = document.source().find("\n\n").expect("paragraph boundary") + 2;
            let second = document.source().rfind("\n\n").expect("paragraph boundary");
            document
                .edit([
                    TextEdit::new(first..first, "x"),
                    TextEdit::new(second..second, "y"),
                ])
                .expect("valid disjoint insertions");
        }
        black_box(document.snapshot().diagnostics().len())
    })
}

fn clone_snapshot() -> usize {
    let source = build_synthetic_aozora(EDIT_BYTES);
    let snapshot = aozora::parse(source)
        .expect("source fits parser span limit")
        .snapshot();
    measured(|| {
        let mut total = 0;
        for _ in 0..1000 {
            total += black_box(snapshot.clone()).source().len();
        }
        total
    })
}

fn parse_unmeasured(src: &str) -> usize {
    black_box(
        aozora::parse(black_box(src))
            .expect("source fits parser span limit")
            .snapshot()
            .diagnostics()
            .len(),
    )
}

fn measured<T>(operation: impl FnOnce() -> T) -> T {
    toggle_collect();
    let result = operation();
    toggle_collect();
    result
}

fn edit_offset(source: &str) -> usize {
    let middle = source.len() / 2;
    source[middle..]
        .find("\n\n")
        .map_or(middle, |relative| middle + relative + 2)
}

fn run(case: &str) -> Option<usize> {
    match case {
        "parse-tosa" => Some(parse(TOSA)),
        "parse-matoi" => Some(parse(MATOI)),
        "parse-peter" => Some(parse(PETER)),
        "html-tosa" => Some(parse_then_html(TOSA)),
        "html-matoi" => Some(parse_then_html(MATOI)),
        "parse-dense" => Some(parse(&build_pathological_aozora(PATHOLOGICAL_BYTES))),
        "parse-large" => Some(parse(&build_synthetic_aozora(LARGE_BYTES))),
        "full-edit-source" => Some(full_edit_source()),
        "full-multiple-edit-source" => Some(full_multiple_edit_source()),
        "edit-single" => Some(single_edit()),
        "edit-multiple" => Some(multiple_edits()),
        "snapshot-clone" => Some(clone_snapshot()),
        _ => None,
    }
}

fn main() -> ExitCode {
    let Some(case) = env::args().nth(1) else {
        eprintln!("usage: perf_gate <case>");
        return ExitCode::from(2);
    };
    let result = run(&case);
    let Some(result) = result else {
        eprintln!("unknown case: {case}");
        return ExitCode::from(2);
    };
    println!("{result}");
    ExitCode::SUCCESS
}
