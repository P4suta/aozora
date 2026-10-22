//! Wall-clock comparison of full parsing and the public editable-document API.

mod support;

use support::report_number;

use std::borrow::Cow;
use std::env;
use std::hint::black_box;
use std::process::ExitCode;
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

fn main() -> ExitCode {
    run().err().unwrap_or(ExitCode::SUCCESS)
}

fn run() -> Result<(), ExitCode> {
    let minimum = configured_ratio("AOZORA_INCREMENTAL_MIN_SPEEDUP");
    let maximum_slowdown = configured_ratio("AOZORA_INCREMENTAL_MAX_SLOWDOWN");
    let docs = load_documents(minimum)?;
    if docs.is_empty() {
        return Ok(());
    }
    measure(&docs, minimum, maximum_slowdown)?;
    Ok(())
}

#[expect(
    clippy::panic,
    reason = "benchmark inputs must meet parser and numeric preconditions"
)]
fn configured_ratio(name: &str) -> Option<f64> {
    env::var(name).ok().map(|value| {
        parse_ratio(&value).unwrap_or_else(|()| panic!("{name} must be a finite positive number"))
    })
}

fn parse_ratio(value: &str) -> Result<f64, ()> {
    value
        .parse::<f64>()
        .ok()
        .filter(|ratio| ratio.is_finite() && *ratio > 0.0)
        .ok_or(())
}

#[expect(
    clippy::print_stderr,
    reason = "benchmark commands report measurements and failures"
)]
fn load_documents(minimum: Option<f64>) -> Result<Vec<String>, ExitCode> {
    let Some(corpus) = aozora_corpus::from_env() else {
        eprintln!("incremental_speedup: AOZORA_CORPUS_ROOT is not set.");
        return if minimum.is_some() {
            Err(ExitCode::FAILURE)
        } else {
            Ok(Vec::new())
        };
    };

    let mut docs = Vec::new();
    let mut io_errors = 0;
    let mut decode_errors = 0;
    for item in corpus.iter() {
        match item {
            Ok(item) => match decode_auto(&item.bytes) {
                Ok(text) if !text.is_empty() => docs.push(Cow::into_owned(text)),
                Ok(_) => {}
                Err(_) => decode_errors += 1,
            },
            Err(_) => io_errors += 1,
        }
    }
    if io_errors != 0 {
        eprintln!(
            "incremental_speedup: refusing a partial-corpus measurement after {io_errors} I/O error(s)"
        );
        return Err(ExitCode::from(2));
    }
    if decode_errors != 0 {
        eprintln!("incremental_speedup: skipped {decode_errors} undecodable document(s)");
    }
    if docs.is_empty() {
        eprintln!("incremental_speedup: corpus yielded 0 usable documents.");
        return if minimum.is_some() {
            Err(ExitCode::FAILURE)
        } else {
            Ok(Vec::new())
        };
    }
    Ok(docs)
}

#[expect(
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "benchmark commands report measurements and stop on invalid benchmark inputs"
)]
fn measure(
    docs: &[String],
    minimum: Option<f64>,
    maximum_slowdown: Option<f64>,
) -> Result<(), ExitCode> {
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
        return Err(ExitCode::FAILURE);
    }
    Ok(())
}

fn add_band(total: &mut Band, band: Band) {
    total.docs += band.docs;
    total.full_ns += band.full_ns;
    total.edit_ns += band.edit_ns;
}

#[expect(
    clippy::print_stdout,
    reason = "benchmark commands report measurements and failures"
)]
fn print_row(name: &str, band: Band) {
    let full_us = report_number(band.full_ns) / report_number(band.docs) / 1000.0;
    let edit_us = report_number(band.edit_ns) / report_number(band.docs) / 1000.0;
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
    report_number(band.full_ns) / report_number(band.edit_ns)
}

#[cfg(test)]
mod tests {
    use super::parse_ratio;

    #[test]
    fn configured_contract_ratios_are_finite_and_positive() {
        assert_eq!(parse_ratio("1.25"), Ok(1.25));
        for value in ["NaN", "inf", "-1", "0", "invalid"] {
            assert_eq!(parse_ratio(value), Err(()), "{value}");
        }
    }
}
