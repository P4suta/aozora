//! Wall-clock comparison of full parsing and the public editable-document API.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::disallowed_methods,
    reason = "profiling tool, not library code"
)]

use std::borrow::Cow;
use std::hint::black_box;
use std::process;
use std::time::Instant;

use aozora::encoding::decode_auto;
use aozora::{TextEdit, parse};

const BANDS: &[(&str, u64, u64)] = &[
    ("< 50 KiB", 0, 50 * 1024),
    ("50 KiB – 500 KiB", 50 * 1024, 500 * 1024),
    ("500 KiB – 2 MiB", 500 * 1024, 2 * 1024 * 1024),
    ("> 2 MiB", 2 * 1024 * 1024, u64::MAX),
];

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
    let Some(corpus) = aozora_corpus::from_env() else {
        println!("incremental_speedup: AOZORA_CORPUS_ROOT not set — nothing to measure.");
        process::exit(0);
    };

    let docs: Vec<String> = corpus
        .iter()
        .filter_map(Result::ok)
        .filter_map(|item| decode_auto(&item.bytes).ok().map(Cow::into_owned))
        .filter(|text| !text.is_empty())
        .collect();
    if docs.is_empty() {
        println!("incremental_speedup: corpus yielded 0 usable documents.");
        process::exit(0);
    }

    let mut bands = [Band::default(); BANDS.len()];
    for source in &docs {
        let band = &mut bands[band_of(source.len() as u64)];
        band.docs += 1;

        let mut at = source.len() / 2;
        while at < source.len() && !source.is_char_boundary(at) {
            at += 1;
        }
        let new_source = format!("{}x{}", &source[..at], &source[at..]);

        black_box(parse(new_source.as_str()));
        let started = Instant::now();
        black_box(parse(new_source.as_str()));
        band.full_ns += started.elapsed().as_nanos();

        let mut document = parse(source.as_str());
        let started = Instant::now();
        document
            .apply_edit(TextEdit::new(at..at, "x"))
            .expect("benchmark edit is in bounds and on a character boundary");
        band.edit_ns += started.elapsed().as_nanos();
        black_box(document.snapshot());
    }

    println!("=== incremental_speedup (full parse vs Document::apply_edit) ===\n");
    println!(
        "{:<18} {:>7} {:>10} {:>10} {:>9}",
        "band", "docs", "full µs", "edit µs", "speedup"
    );
    let mut total = Band::default();
    for (index, &(name, _, _)) in BANDS.iter().enumerate() {
        let band = bands[index];
        if band.docs == 0 {
            continue;
        }
        total.docs += band.docs;
        total.full_ns += band.full_ns;
        total.edit_ns += band.edit_ns;
        print_row(name, band);
    }
    println!("{:-<64}", "");
    print_row("all", total);
}

fn print_row(name: &str, band: Band) {
    let full_us = band.full_ns as f64 / band.docs as f64 / 1000.0;
    let edit_us = band.edit_ns as f64 / band.docs as f64 / 1000.0;
    let speedup = if edit_us > 0.0 {
        full_us / edit_us
    } else {
        0.0
    };
    println!(
        "{name:<18} {:>7} {full_us:>10.1} {edit_us:>10.1} {speedup:>8.2}x",
        band.docs
    );
}
