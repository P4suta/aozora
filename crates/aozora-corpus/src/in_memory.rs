//! [`InMemoryCorpus`] — build a corpus from explicit byte-string pairs.
//!
//! Intended for unit tests that exercise the [`CorpusSource`] contract
//! without touching the filesystem. The test constructs the exact set
//! of inputs it wants and hands the source to code under test as
//! `dyn CorpusSource`.

use crate::{CorpusError, CorpusItem, CorpusSource};

/// Corpus source populated from caller-supplied items.
///
/// The internal store is a `Vec<CorpusItem>` snapshotted at construction
/// time. Iteration yields clones of each item in insertion order. `Ok`
/// is the only outcome: an in-memory source cannot raise I/O errors.
#[derive(Debug, Clone)]
pub struct InMemoryCorpus {
    items: Vec<CorpusItem>,
    // Stored as `String` rather than returned as a `&'static str` literal
    // so the `provenance(&self) -> &str` method genuinely borrows from
    // `&self`, matching what the trait contract signals to callers (and
    // keeping symmetric shape with the filesystem/vendored impls, whose
    // provenance strings are path-derived).
    provenance: String,
}

impl InMemoryCorpus {
    /// Construct from an explicit item list.
    #[must_use]
    pub fn new(items: Vec<CorpusItem>) -> Self {
        Self {
            items,
            provenance: String::from("in-memory"),
        }
    }

    /// Construct from `(label, bytes)` pairs. Convenience over
    /// [`InMemoryCorpus::new`] when the caller has loose tuples rather
    /// than pre-built [`CorpusItem`] values.
    pub fn from_pairs<I, L, B>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (L, B)>,
        L: Into<String>,
        B: Into<Vec<u8>>,
    {
        let items = pairs
            .into_iter()
            .map(|(label, bytes)| CorpusItem::new(label, bytes.into()))
            .collect();
        Self {
            items,
            provenance: String::from("in-memory"),
        }
    }
}

impl CorpusSource for InMemoryCorpus {
    fn iter(&self) -> Box<dyn Iterator<Item = Result<CorpusItem, CorpusError>> + '_> {
        Box::new(self.items.iter().cloned().map(Ok))
    }

    fn provenance(&self) -> &str {
        &self.provenance
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_preserves_item_order() {
        let items = vec![
            CorpusItem::new("first", b"a".to_vec()),
            CorpusItem::new("second", b"bb".to_vec()),
            CorpusItem::new("third", b"ccc".to_vec()),
        ];
        let corpus = InMemoryCorpus::new(items);
        let labels: Vec<String> = corpus
            .iter()
            .map(|r| r.expect("in-memory never errors").label)
            .collect();
        assert_eq!(labels, vec!["first", "second", "third"]);
    }

    #[test]
    fn from_pairs_accepts_heterogeneous_input() {
        let corpus = InMemoryCorpus::from_pairs(vec![
            ("as-str", Vec::from(*b"bytes")),
            ("also-str", Vec::from(*b"more bytes")),
        ]);
        let collected: Vec<_> = corpus
            .iter()
            .map(|r| r.expect("in-memory never errors"))
            .collect();
        assert_eq!(collected.len(), 2);
        assert_eq!(collected[0].label, "as-str");
        assert_eq!(collected[1].bytes, b"more bytes".to_vec());
    }

    #[test]
    fn empty_corpus_yields_zero_items() {
        let corpus = InMemoryCorpus::new(Vec::new());
        assert_eq!(corpus.iter().count(), 0);
    }

    #[test]
    fn provenance_is_stable_label() {
        let corpus = InMemoryCorpus::new(Vec::new());
        assert_eq!(corpus.provenance(), "in-memory");
    }

    #[test]
    fn from_pairs_with_owned_strings() {
        // Exercise the String (not &str) branch of the Into<String> bound,
        // so the generic parameter's full shape is covered.
        let corpus = InMemoryCorpus::from_pairs(vec![(String::from("owned"), vec![0u8])]);
        let item = corpus
            .iter()
            .next()
            .expect("one item")
            .expect("no error for in-memory");
        assert_eq!(item.label, "owned");
        assert_eq!(item.bytes, vec![0u8]);
    }
}
