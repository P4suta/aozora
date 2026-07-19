use std::time::Instant;

use aozora::{Diagnostic, Document, Snapshot};
use ropey::Rope;

use crate::lsp::text_edit::ByteEdit;

pub(crate) const MAX_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;

#[inline]
pub(crate) const fn exceeds_document_cap(len: usize) -> bool {
    len > MAX_DOCUMENT_BYTES
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct ReparseStats {
    pub parse_count: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub cache_entries_after: u64,
    pub cache_bytes_estimate: u64,
    pub latency_us: u64,
}

#[derive(Debug, Default)]
pub(super) struct ParseCache {
    document: Option<Document>,
    snapshot: Option<Snapshot>,
}

impl ParseCache {
    #[cfg(test)]
    pub(crate) fn reparse(&mut self, text: &str) -> (Vec<Diagnostic>, ReparseStats) {
        self.reparse_incremental(&Rope::from(text), &[])
    }

    pub(super) fn reparse_incremental(
        &mut self,
        raw: &Rope,
        edits: &[ByteEdit],
    ) -> (Vec<Diagnostic>, ReparseStats) {
        let started = Instant::now();
        let text = raw.to_string();
        if text.is_empty() || exceeds_document_cap(text.len()) {
            self.document = None;
            self.snapshot = None;
            return (
                Vec::new(),
                ReparseStats {
                    cache_bytes_estimate: text.len() as u64,
                    latency_us: elapsed_us(started),
                    ..ReparseStats::default()
                },
            );
        }

        let reused = if let (Some(document), [edit]) = (&mut self.document, edits) {
            let edit = aozora::TextEdit::new(edit.range.clone(), edit.new_text.clone());
            document.apply_edit(edit).is_ok() && document.source() == text
        } else {
            false
        };
        if !reused {
            self.document = Some(aozora::parse(text));
        }
        let document = self.document.as_ref().expect("document was installed");
        let snapshot = document.snapshot();
        let diagnostics = snapshot.diagnostics().to_vec();
        self.snapshot = Some(snapshot);
        (
            diagnostics,
            ReparseStats {
                parse_count: 1,
                cache_hits: u64::from(reused),
                cache_misses: u64::from(!reused),
                cache_entries_after: 1,
                cache_bytes_estimate: document.source().len() as u64,
                latency_us: elapsed_us(started),
            },
        )
    }

    #[must_use]
    pub(super) fn diagnostics(&self) -> &[Diagnostic] {
        self.snapshot
            .as_ref()
            .map_or(&[], |snapshot| snapshot.diagnostics())
    }

    pub(super) fn with_snapshot<R>(&self, f: impl FnOnce(&Snapshot) -> R) -> Option<R> {
        self.snapshot.as_ref().map(f)
    }
}

// mutants::skip — wall-clock values are intentionally nondeterministic.
#[cfg_attr(test, mutants::skip)]
fn elapsed_us(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_micros())
        .unwrap_or(u64::MAX)
        .max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_is_applied_through_document() {
        let mut cache = ParseCache::default();
        cache.reparse("plain");
        let edit = ByteEdit {
            range: 0..5,
            new_text: "changed".to_owned(),
        };
        let (_, stats) = cache.reparse_incremental(&Rope::from("changed"), &[edit]);
        assert_eq!(stats.cache_hits, 1);
        assert_eq!(
            cache.with_snapshot(|snapshot| snapshot.source().to_owned()),
            Some("changed".to_owned())
        );
    }

    #[test]
    fn invalid_edit_falls_back_to_authoritative_source() {
        let mut cache = ParseCache::default();
        cache.reparse("plain");
        let edit = ByteEdit {
            range: 20..21,
            new_text: "x".to_owned(),
        };
        let (_, stats) = cache.reparse_incremental(&Rope::from("fresh"), &[edit]);
        assert_eq!(stats.cache_misses, 1);
        assert_eq!(
            cache.with_snapshot(|snapshot| snapshot.source().to_owned()),
            Some("fresh".to_owned())
        );
    }

    #[test]
    fn valid_edit_with_mismatched_authoritative_source_is_a_miss() {
        let mut cache = ParseCache::default();
        cache.reparse("plain");
        let edit = ByteEdit {
            range: 0..5,
            new_text: "changed".to_owned(),
        };
        let (_, stats) = cache.reparse_incremental(&Rope::from("different"), &[edit]);
        assert_eq!(stats.cache_hits, 0);
        assert_eq!(stats.cache_misses, 1);
        assert_eq!(
            cache.with_snapshot(|snapshot| snapshot.source().to_owned()),
            Some("different".to_owned())
        );
    }

    #[test]
    fn empty_and_oversize_inputs_clear_the_cache_and_report_size() {
        let mut cache = ParseCache::default();
        cache.reparse("plain");
        let (_, empty) = cache.reparse_incremental(&Rope::new(), &[]);
        assert_eq!(empty.cache_bytes_estimate, 0);
        assert!(empty.latency_us > 0);
        assert!(cache.with_snapshot(|_| ()).is_none());

        let oversized = "x".repeat(MAX_DOCUMENT_BYTES + 1);
        let (_, stats) = cache.reparse_incremental(&Rope::from(oversized.as_str()), &[]);
        assert_eq!(stats.cache_bytes_estimate, (MAX_DOCUMENT_BYTES + 1) as u64);
        assert!(cache.with_snapshot(|_| ()).is_none());
    }

    #[test]
    fn successful_reparse_reports_all_cache_statistics() {
        let mut cache = ParseCache::default();
        let (_, stats) = cache.reparse("plain");
        assert_eq!(
            stats,
            ReparseStats {
                parse_count: 1,
                cache_hits: 0,
                cache_misses: 1,
                cache_entries_after: 1,
                cache_bytes_estimate: 5,
                latency_us: stats.latency_us,
            }
        );
        assert!(stats.latency_us > 0);
    }

    #[test]
    fn size_cap_is_exclusive() {
        assert!(!exceeds_document_cap(MAX_DOCUMENT_BYTES));
        assert!(exceeds_document_cap(MAX_DOCUMENT_BYTES + 1));
    }
}
