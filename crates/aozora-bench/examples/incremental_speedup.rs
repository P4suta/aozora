//! Wall-clock comparison of full parsing and the public editable-document API.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::disallowed_methods,
    reason = "profiling tool, not library code"
)]

use std::borrow::Cow;
use std::env;
use std::hint::black_box;
use std::process;
use std::time::Instant;

use aozora::decode_auto;
use aozora::{TextEdit, parse};

const BANDS: &[(&str, u64, u64)] = &[
    ("< 50 KiB", 0, 50 * 1024),
    ("50 KiB – 500 KiB", 50 * 1024, 500 * 1024),
    ("500 KiB – 2 MiB", 500 * 1024, 2 * 1024 * 1024),
    ("> 2 MiB", 2 * 1024 * 1024, u64::MAX),
];
const SPEEDUP_MIN_BYTES: u64 = 500 * 1024;

#[derive(Default, Clone, Copy)]
struct Band {
    docs: u64,
    full_ns: u128,
    edit_ns: u128,
}

fn band_of(len: u64) -> usize {
    BANDS
        .iter()
        .position(|&(_, lo, hi)| len >= lo && len < hi)
        .unwrap_or(0)
}

fn main() {
    let minimum = configured_ratio("AOZORA_INCREMENTAL_MIN_SPEEDUP");
    let maximum_slowdown = configured_ratio("AOZORA_INCREMENTAL_MAX_SLOWDOWN");
    let docs = load_documents(minimum);
    measure(&docs, minimum, maximum_slowdown);
}

fn configured_ratio(name: &str) -> Option<f64> {
    env::var(name).ok().map(|value| {
        value
            .parse::<f64>()
            .unwrap_or_else(|_| panic!("{name} must be a number"))
    })
}

fn load_documents(minimum: Option<f64>) -> Vec<String> {
    let Some(corpus) = aozora_corpus::from_env() else {
        eprintln!("incremental_speedup: AOZORA_CORPUS_ROOT is not set.");
        process::exit(i32::from(minimum.is_some()));
    };

    let docs = corpus
        .iter()
        .filter_map(Result::ok)
        .filter_map(|item| decode_auto(&item.bytes).ok().map(Cow::into_owned))
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>();
    if docs.is_empty() {
        eprintln!("incremental_speedup: corpus yielded 0 usable documents.");
        process::exit(i32::from(minimum.is_some()));
    }
    docs
}

fn measure(docs: &[String], minimum: Option<f64>, maximum_slowdown: Option<f64>) {
    let mut bands = [Band::default(); BANDS.len()];
    for source in docs {
        let mut at = source.len() / 2;
        while at < source.len() && !source.is_char_boundary(at) {
            at += 1;
        }
        let mut document = parse(source.as_str()).expect("source fits parser span limit");
        let initial = document.snapshot();
        if !initial.diagnostics().is_empty() || !initial.normalized_source().contains("\n\n") {
            continue;
        }
        document
            .edit([TextEdit::new(at..at, "w")])
            .expect("warm-up edit is valid");
        let warmed = format!("{}w{}", &source[..at], &source[at..]);
        let next_at = at + 1;
        let new_source = format!("{}x{}", &warmed[..next_at], &warmed[next_at..]);
        let band = &mut bands[band_of(new_source.len() as u64)];
        band.docs += 1;

        black_box(parse(new_source.as_str()).expect("source fits parser span limit"));
        let started = Instant::now();
        black_box(parse(new_source.as_str()).expect("source fits parser span limit"));
        band.full_ns += started.elapsed().as_nanos();

        let started = Instant::now();
        document
            .edit([TextEdit::new(next_at..next_at, "x")])
            .expect("benchmark edit is in bounds and on a character boundary");
        band.edit_ns += started.elapsed().as_nanos();
        black_box(document.snapshot());
    }

    println!("=== incremental_speedup (full parse vs Document::edit) ===\n");
    println!(
        "{:<18} {:>7} {:>10} {:>10} {:>9}",
        "band", "docs", "full µs", "edit µs", "speedup"
    );
    let mut total = Band::default();
    let mut small = Band::default();
    let mut large = Band::default();
    let mut failed = false;
    for (index, &(name, lower, _)) in BANDS.iter().enumerate() {
        let band = bands[index];
        if band.docs == 0 {
            continue;
        }
        total.docs += band.docs;
        total.full_ns += band.full_ns;
        total.edit_ns += band.edit_ns;
        print_row(name, band);
        if lower < SPEEDUP_MIN_BYTES {
            add_band(&mut small, band);
            failed |= maximum_slowdown.is_some_and(|maximum| speedup(band) < maximum.recip());
        } else {
            add_band(&mut large, band);
        }
    }
    println!("{:-<64}", "");
    print_row("all", total);
    print_row("< 500 KiB", small);
    if large.docs != 0 {
        print_row(">= 500 KiB", large);
    }
    if minimum.is_some() && large.docs == 0 {
        eprintln!("incremental_speedup: corpus yielded 0 eligible large documents.");
        failed = true;
    }
    failed |= minimum.is_some_and(|minimum| speedup(large) < minimum);
    if failed {
        eprintln!("incremental_speedup: performance contract failed.");
        process::exit(1);
    }
}

fn add_band(total: &mut Band, band: Band) {
    total.docs += band.docs;
    total.full_ns += band.full_ns;
    total.edit_ns += band.edit_ns;
}

fn print_row(name: &str, band: Band) {
    let full_us = band.full_ns as f64 / band.docs as f64 / 1000.0;
    let edit_us = band.edit_ns as f64 / band.docs as f64 / 1000.0;
    let speedup = speedup(band);
    println!(
        "{name:<18} {:>7} {full_us:>10.1} {edit_us:>10.1} {speedup:>8.2}x",
        band.docs
    );
}

fn speedup(band: Band) -> f64 {
    if band.edit_ns == 0 {
        return 0.0;
    }
    band.full_ns as f64 / band.edit_ns as f64
}
