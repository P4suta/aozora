//! Fixed workloads measured by the repository's Callgrind performance gate.

use std::{env, hint::black_box, process::ExitCode};

use aozora::Document;
use aozora_bench::build_pathological_aozora;

const TOSA: &str = include_str!("../../aozora-conformance/fixtures/works/terada-tosa/source.txt");
const MATOI: &str =
    include_str!("../../aozora-conformance/fixtures/works/orikuchi-matoi/source.txt");
const PETER: &str = include_str!("../../aozora-conformance/fixtures/works/potter-peter/source.txt");
const PATHOLOGICAL_BYTES: usize = 64 * 1024;

fn parse(src: &str) -> usize {
    black_box(Document::new(black_box(src)).snapshot().diagnostics().len())
}

fn parse_then_html(src: &str) -> usize {
    black_box(Document::new(black_box(src)).snapshot().to_html().len())
}

fn run(case: &str) -> Option<usize> {
    match case {
        "parse-tosa" => Some(parse(TOSA)),
        "parse-matoi" => Some(parse(MATOI)),
        "parse-peter" => Some(parse(PETER)),
        "html-tosa" => Some(parse_then_html(TOSA)),
        "html-matoi" => Some(parse_then_html(MATOI)),
        "parse-dense" => Some(parse(&build_pathological_aozora(PATHOLOGICAL_BYTES))),
        _ => None,
    }
}

fn main() -> ExitCode {
    let Some(case) = env::args().nth(1) else {
        eprintln!("usage: perf_gate <case>");
        return ExitCode::from(2);
    };
    let Some(result) = run(&case) else {
        eprintln!("unknown case: {case}");
        return ExitCode::from(2);
    };
    println!("{result}");
    ExitCode::SUCCESS
}
