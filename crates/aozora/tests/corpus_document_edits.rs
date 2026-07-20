//! Corpus differential for edits through the public document API.

use std::env;

use aozora::{Snapshot, TextEdit, decode_auto};
use aozora_corpus::{CorpusItem, FilesystemCorpus, par_load_decoded};

#[test]
fn edited_documents_match_cold_parses() {
    let Some(corpus) = configured_corpus() else {
        assert!(
            env::var_os("AOZORA_REQUIRE_CORPUS").is_none(),
            "AOZORA_CORPUS_ROOT must name a non-empty corpus for a release gate"
        );
        eprintln!("AOZORA_CORPUS_ROOT not set; document edit differential not requested");
        return;
    };

    let filter = env::var("AOZORA_CORPUS_FILTER").ok();
    let outcomes = par_load_decoded(&corpus, |item| edit_outcome(item, filter.as_deref()));
    let mut checked = 0;
    let mut diverged = Vec::new();
    for outcome in outcomes {
        let Some((label, mismatches)) = outcome.expect("corpus iteration must not error") else {
            continue;
        };
        checked += 1;
        if !mismatches.is_empty() {
            if diverged.len() < 10 {
                eprintln!("{}: {}", label, mismatches.join(", "));
            }
            diverged.push(label);
        }
    }
    diverged.sort_unstable();

    assert!(
        checked > 0,
        "the corpus must contain at least one decodable document"
    );
    assert!(
        diverged.is_empty(),
        "document edit differential diverged for:\n{}",
        diverged.join("\n")
    );
}

fn configured_corpus() -> Option<FilesystemCorpus> {
    FilesystemCorpus::new(env::var_os(aozora_corpus::ENV_CORPUS_ROOT)?).ok()
}

fn edit_outcome(item: CorpusItem, filter: Option<&str>) -> Option<(String, Vec<&'static str>)> {
    if filter.is_some_and(|filter| !item.label.contains(filter)) {
        return None;
    }
    let source = decode_auto(&item.bytes).ok()?;
    (!source.is_empty()).then(|| {
        let mismatches = compare_edits(source.as_ref(), filter.is_some());
        (item.label, mismatches)
    })
}

fn compare_edits(source: &str, verbose: bool) -> Vec<&'static str> {
    let first = insertion_offset(source, source.len() / 3);
    let second = insertion_offset(source, source.len() * 2 / 3);
    let mut expected = source.to_owned();
    expected.insert(second, 'y');
    expected.insert(first, 'x');

    let mut edited = aozora::parse(source).expect("source fits parser span limit");
    edited
        .edit([
            TextEdit::new(first..first, "x"),
            TextEdit::new(second..second, "y"),
        ])
        .expect("generated insertions must be valid");
    let actual = edited.snapshot();
    let cold = aozora::parse(expected.as_str())
        .expect("source fits parser span limit")
        .snapshot();

    let mut mismatches = Vec::new();
    mismatch(&mut mismatches, actual.source() != expected, "source");
    mismatch(
        &mut mismatches,
        actual.normalized_source() != cold.normalized_source(),
        "normalized",
    );
    mismatch(
        &mut mismatches,
        !diagnostics_match(&actual, &cold, verbose),
        "diagnostics",
    );
    mismatch(&mut mismatches, actual.nodes() != cold.nodes(), "nodes");
    mismatch(&mut mismatches, actual.pairs() != cold.pairs(), "pairs");
    mismatch(
        &mut mismatches,
        actual.container_pairs() != cold.container_pairs(),
        "container-pairs",
    );
    mismatch(
        &mut mismatches,
        actual.literal_markup() != cold.literal_markup(),
        "literal-markup",
    );
    mismatch(
        &mut mismatches,
        actual.directives().collect::<Vec<_>>() != cold.directives().collect::<Vec<_>>(),
        "directives",
    );
    mismatch(
        &mut mismatches,
        actual.rubies().collect::<Vec<_>>() != cold.rubies().collect::<Vec<_>>(),
        "rubies",
    );
    mismatch(
        &mut mismatches,
        actual.gaiji_resolutions() != cold.gaiji_resolutions(),
        "gaiji",
    );
    mismatch(
        &mut mismatches,
        actual.to_source() != cold.to_source(),
        "source-render",
    );
    mismatch(
        &mut mismatches,
        actual.to_source_verbatim() != cold.to_source_verbatim(),
        "verbatim-render",
    );
    mismatch(&mut mismatches, actual.to_html() != cold.to_html(), "html");
    mismatches
}

fn insertion_offset(source: &str, mut offset: usize) -> usize {
    while !source.is_char_boundary(offset) {
        offset += 1;
    }
    offset
}

fn mismatch(found: &mut Vec<&'static str>, differs: bool, projection: &'static str) {
    if differs {
        found.push(projection);
    }
}

fn diagnostics_match(actual: &Snapshot, cold: &Snapshot, verbose: bool) -> bool {
    let rows = |snapshot: &Snapshot| {
        snapshot
            .diagnostics()
            .iter()
            .map(|diagnostic| {
                (
                    diagnostic.code(),
                    diagnostic.severity(),
                    diagnostic.source(),
                    diagnostic.span(),
                    diagnostic.to_string(),
                )
            })
            .collect::<Vec<_>>()
    };
    let actual = rows(actual);
    let cold = rows(cold);
    if actual != cold && verbose {
        eprintln!("actual diagnostics: {actual:#?}");
        eprintln!("cold diagnostics: {cold:#?}");
    }
    actual == cold
}
