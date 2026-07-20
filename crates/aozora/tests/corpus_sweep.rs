//! Walks `AOZORA_CORPUS_ROOT` and verifies every document parses
//! cleanly and round-trips through `parse ∘ serialize` (the I3
//! fixed-point invariant).
//!
//! `AOZORA_REQUIRE_CORPUS` turns an absent or empty corpus into a
//! release-gate failure.

use std::borrow::Cow;
use std::env;

use aozora::decode_auto;
use aozora_corpus::{CorpusItem, FilesystemCorpus, par_load_decoded};

#[test]
fn corpus_round_trip_is_a_fixed_point() {
    let Some(corpus) = configured_corpus() else {
        assert!(
            env::var_os("AOZORA_REQUIRE_CORPUS").is_none(),
            "AOZORA_CORPUS_ROOT must name a non-empty corpus for a release gate"
        );
        eprintln!("AOZORA_CORPUS_ROOT not set; corpus sweep not requested");
        return;
    };

    let mut count: usize = 0;
    let mut sjis_decoded: usize = 0;
    let mut diverged: Vec<(String, String)> = Vec::new();
    let mut undecodable = Vec::new();
    for outcome in par_load_decoded(&corpus, sweep_item) {
        match outcome.expect("corpus iteration must not error") {
            SweepOutcome::Undecodable(label) => undecodable.push(label),
            SweepOutcome::Checked {
                label,
                sjis,
                difference,
            } => {
                count += 1;
                sjis_decoded += usize::from(sjis);
                if let Some(difference) = difference {
                    diverged.push((label, difference));
                }
            }
        }
    }
    undecodable.sort_unstable();
    for label in undecodable {
        eprintln!("skip (neither UTF-8 nor Shift_JIS): {label}");
    }

    eprintln!(
        "corpus sweep: {count} docs walked ({sjis_decoded} decoded from Shift_JIS, the rest already UTF-8)"
    );

    diverged.sort_by(|a, b| a.0.cmp(&b.0));
    if !diverged.is_empty() {
        let list = diverged
            .iter()
            .map(|(l, h)| format!("{l}{h}"))
            .collect::<Vec<_>>()
            .join("\n  ");
        panic!("round-trip divergence(s):\n  {list}");
    }

    assert!(
        count > 0,
        "the corpus must contain at least one decodable document"
    );
    eprintln!("corpus sweep: round-trip fixed point holds");
}

enum SweepOutcome {
    Undecodable(String),
    Checked {
        label: String,
        sjis: bool,
        difference: Option<String>,
    },
}

fn configured_corpus() -> Option<FilesystemCorpus> {
    FilesystemCorpus::new(env::var_os(aozora_corpus::ENV_CORPUS_ROOT)?).ok()
}

fn sweep_item(item: CorpusItem) -> SweepOutcome {
    let Ok(utf8) = decode_auto(&item.bytes) else {
        return SweepOutcome::Undecodable(item.label);
    };
    let sjis = matches!(utf8, Cow::Owned(_));
    let serialized = aozora::parse(utf8)
        .expect("source fits parser span limit")
        .snapshot()
        .to_source();
    let serialized2 = aozora::parse(serialized.clone())
        .expect("source fits parser span limit")
        .snapshot()
        .to_source();
    let difference =
        (serialized != serialized2).then(|| first_diff_hint(&serialized, &serialized2));
    SweepOutcome::Checked {
        label: item.label,
        sjis,
        difference,
    }
}

/// A compact `…before│after…` window around the first byte where two
/// serializations diverge — enough to identify the offending construct.
fn first_diff_hint(a: &str, b: &str) -> String {
    let at = a
        .bytes()
        .zip(b.bytes())
        .position(|(x, y)| x != y)
        .unwrap_or_else(|| a.len().min(b.len()));
    let lo = floor_boundary(a, at.saturating_sub(24));
    let a_hi = ceil_boundary(a, (at + 24).min(a.len()));
    let b_hi = ceil_boundary(b, (at + 24).min(b.len()));
    format!(
        "  @{at}: …{}│ vs │{}…",
        &a[lo..a_hi],
        &b[floor_boundary(b, lo)..b_hi]
    )
}

fn floor_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_boundary(s: &str, mut i: usize) -> usize {
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}
