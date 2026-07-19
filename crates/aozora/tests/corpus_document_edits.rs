//! Corpus differential for edits through the public document API.

use aozora::encoding::decode_auto;
use aozora::{Document, TextEdit};

#[test]
fn edited_documents_match_cold_parses() {
    let Some(corpus) = aozora_corpus::from_env() else {
        eprintln!("AOZORA_CORPUS_ROOT not set; skipping document edit differential");
        return;
    };

    let mut checked = 0;
    let mut diverged = Vec::new();

    for item in corpus.iter() {
        let item = item.expect("corpus iteration must not error");
        let Ok(source) = decode_auto(&item.bytes) else {
            continue;
        };
        if source.is_empty() {
            continue;
        }

        let mut offset = source.len() / 2;
        while !source.is_char_boundary(offset) {
            offset += 1;
        }

        let mut expected = source.to_string();
        expected.insert(offset, 'x');

        let mut edited = Document::new(source.as_ref());
        edited
            .apply_edit(TextEdit::new(offset..offset, "x"))
            .expect("midpoint insertion must be valid");
        let actual = edited.snapshot();
        let cold = aozora::parse(expected.as_str()).snapshot();

        checked += 1;
        if actual.source() != expected
            || actual.to_source() != cold.to_source()
            || actual.to_html() != cold.to_html()
            || format!("{:?}", actual.diagnostics()) != format!("{:?}", cold.diagnostics())
        {
            diverged.push(item.label);
        }
    }

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
