#![expect(
    clippy::expect_used,
    reason = "the LSP document map is read only after the matching insertion or lookup"
)]

use std::collections::BTreeMap;
use std::env;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use arc_swap::ArcSwap;
use parking_lot::Mutex;
use ropey::Rope;
use tokio::task::AbortHandle;

use crate::lsp::gaiji_spans::{GaijiSpan, extract_gaiji_spans};
use crate::lsp::line_index::LineIndex;
use crate::lsp::metrics::{Metrics, ParseSample};
use crate::lsp::text_edit::ByteEdit;

pub(super) struct DocSnapshot {
    core: aozora::Snapshot,
    pub total_bytes: u32,
    pub version: u64,
    doc_rope: OnceLock<Arc<Rope>>,
    doc_line_index: OnceLock<Arc<LineIndex>>,
    doc_gaiji_spans: OnceLock<Arc<BTreeMap<u32, Arc<GaijiSpan>>>>,
}

impl fmt::Debug for DocSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DocSnapshot")
            .field("version", &self.version)
            .field("total_bytes", &self.total_bytes)
            .finish_non_exhaustive()
    }
}

impl DocSnapshot {
    fn new(core: aozora::Snapshot, version: u64) -> Arc<Self> {
        Arc::new(Self {
            total_bytes: u32::try_from(core.source().len()).unwrap_or(u32::MAX),
            core,
            version,
            doc_rope: OnceLock::new(),
            doc_line_index: OnceLock::new(),
            doc_gaiji_spans: OnceLock::new(),
        })
    }

    #[must_use]
    pub(super) fn core(&self) -> &aozora::Snapshot {
        &self.core
    }

    #[must_use]
    pub(super) fn doc_text(&self) -> &str {
        self.core.source()
    }

    #[must_use]
    pub(super) fn doc_rope(&self) -> &Arc<Rope> {
        self.doc_rope
            .get_or_init(|| Arc::new(Rope::from(self.core.source())))
    }

    #[must_use]
    pub(super) fn doc_line_index(&self) -> &Arc<LineIndex> {
        self.doc_line_index
            .get_or_init(|| Arc::new(LineIndex::new(self.core.source())))
    }

    #[must_use]
    pub(super) fn doc_gaiji_spans(&self) -> &Arc<BTreeMap<u32, Arc<GaijiSpan>>> {
        self.doc_gaiji_spans.get_or_init(|| {
            Arc::new(
                extract_gaiji_spans(&self.core)
                    .iter()
                    .map(|span| (span.start_byte, Arc::clone(span)))
                    .collect(),
            )
        })
    }
}

pub(super) struct OpenDocument {
    document: Mutex<aozora::Document>,
    snapshot: ArcSwap<DocSnapshot>,
    edit_version: AtomicU64,
    pub metrics: Arc<Metrics>,
    debounce_task: Mutex<Option<AbortHandle>>,
}

impl fmt::Debug for OpenDocument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenDocument")
            .field("edit_version", &self.edit_version.load(Ordering::Relaxed))
            .field("snapshot_version", &self.snapshot.load().version)
            .finish_non_exhaustive()
    }
}

impl OpenDocument {
    #[must_use]
    pub(super) fn new(text: String) -> Arc<Self> {
        let started = Instant::now();
        let document = aozora::parse(text).expect("LSP document fits parser spans");
        let core = document.snapshot();
        let source_bytes = core.source().len();
        let initial = DocSnapshot::new(core, 0);
        let state = Arc::new(Self {
            document: Mutex::new(document),
            snapshot: ArcSwap::from(initial),
            edit_version: AtomicU64::new(0),
            metrics: Arc::new(Metrics::default()),
            debounce_task: Mutex::new(None),
        });
        state.record_parse(started, false, source_bytes);
        state
    }

    #[must_use]
    pub(super) fn snapshot(&self) -> Arc<DocSnapshot> {
        self.snapshot.load_full()
    }

    pub(super) fn edit_version(&self) -> u64 {
        self.edit_version.load(Ordering::SeqCst)
    }

    pub(super) fn replace_debounce_task(&self, handle: AbortHandle) {
        let previous = self.debounce_task.lock().replace(handle);
        if let Some(previous) = previous {
            previous.abort();
        }
    }

    pub(super) fn apply_changes(self: &Arc<Self>, edits: &[ByteEdit]) -> Option<u64> {
        let started = Instant::now();
        let edits = edits
            .iter()
            .map(|edit| aozora::TextEdit::new(edit.range.clone(), edit.new_text.clone()));
        let mut document = self.document.lock();
        document.edit(edits).ok()?;
        let core = document.snapshot();
        let source_bytes = core.source().len();
        drop(document);
        self.metrics.record_edit();
        self.record_parse(started, true, source_bytes);
        let version = self.edit_version.fetch_add(1, Ordering::SeqCst) + 1;
        self.install(core, version);
        Some(version)
    }

    pub(super) fn replace_text(self: &Arc<Self>, new_text: String) -> u64 {
        let started = Instant::now();
        let document = aozora::parse(new_text).expect("LSP document fits parser spans");
        let core = document.snapshot();
        let source_bytes = core.source().len();
        *self.document.lock() = document;
        self.metrics.record_edit();
        self.record_parse(started, false, source_bytes);
        let version = self.edit_version.fetch_add(1, Ordering::SeqCst) + 1;
        self.install(core, version);
        version
    }

    #[cfg(test)]
    pub(super) fn rebuild_snapshot_now(&self) {
        let core = self.document.lock().snapshot();
        let version = self.edit_version.load(Ordering::SeqCst);
        self.install(core, version);
    }

    fn install(&self, core: aozora::Snapshot, version: u64) {
        let candidate = DocSnapshot::new(core, version);
        self.snapshot.rcu(|current| {
            if candidate.version >= current.version {
                Arc::clone(&candidate)
            } else {
                Arc::clone(current)
            }
        });
    }

    #[cfg(test)]
    fn install_if_newer(&self, candidate: &Arc<DocSnapshot>) -> bool {
        let mut installed = false;
        self.snapshot.rcu(|current| {
            if candidate.version >= current.version {
                installed = true;
                Arc::clone(candidate)
            } else {
                installed = false;
                Arc::clone(current)
            }
        });
        installed
    }

    pub(super) fn publication_snapshot(&self) -> (Rope, Vec<aozora::Diagnostic>, u64) {
        let snapshot = self.snapshot();
        (
            Rope::from(snapshot.doc_text()),
            snapshot.core().diagnostics().to_vec(),
            snapshot.version,
        )
    }

    fn record_parse(&self, started: Instant, cache_hit: bool, source_bytes: usize) {
        let latency_us = u64::try_from(started.elapsed().as_micros())
            .unwrap_or(u64::MAX)
            .max(1);
        let cache_hits = u64::from(cache_hit);
        let cache_misses = u64::from(!cache_hit);
        self.metrics.record_parse(ParseSample {
            latency_us,
            cache_hits,
            cache_misses,
            cache_entries: 1,
            cache_bytes_estimate: source_bytes as u64,
        });
        let threshold_us = slow_parse_threshold_us();
        if parse_is_slow(latency_us, threshold_us) {
            tracing::warn!(
                latency_us,
                threshold_us,
                cache_hits,
                cache_misses,
                "parse exceeded slow-path threshold",
            );
        }
    }
}

const fn parse_is_slow(latency_us: u64, threshold_us: u64) -> bool {
    latency_us > threshold_us
}

fn parse_slow_threshold(value: Option<&str>) -> u64 {
    value
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(100_000)
}

fn slow_parse_threshold_us() -> u64 {
    parse_slow_threshold(env::var("AOZORA_LSP_SLOW_PARSE_US").ok().as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_flow_through_core_document() {
        let state = OpenDocument::new("plain".to_owned());
        assert_eq!(
            state.apply_changes(&[ByteEdit {
                range: 0..5,
                new_text: "changed".to_owned(),
            }]),
            Some(1)
        );
        assert_eq!(state.snapshot().doc_text(), "changed");
        assert_eq!(state.snapshot().core().source(), "changed");
    }

    #[test]
    fn open_document_debug_reports_versions() {
        let state = OpenDocument::new("plain".to_owned());
        let debug = format!("{state:?}");
        assert!(debug.contains("edit_version: 0"), "{debug}");
        assert!(debug.contains("snapshot_version: 0"), "{debug}");
    }

    #[test]
    fn rejected_edit_is_atomic() {
        let state = OpenDocument::new("あ".to_owned());
        assert_eq!(
            state.apply_changes(&[ByteEdit {
                range: 1..2,
                new_text: "x".to_owned(),
            }]),
            None
        );
        assert_eq!(state.snapshot().doc_text(), "あ");
    }

    #[test]
    fn parse_metrics_distinguish_incremental_hits_from_full_parse_misses() {
        let state = OpenDocument::new("plain".to_owned());
        let initial = state.metrics.snapshot();
        assert_eq!(initial.cache_hit_total, 0);
        assert_eq!(initial.cache_miss_total, 1);

        assert_eq!(
            state.apply_changes(&[ByteEdit {
                range: 0..5,
                new_text: "changed".to_owned(),
            }]),
            Some(1),
        );
        let incremental = state.metrics.snapshot();
        assert_eq!(incremental.cache_hit_total, 1);
        assert_eq!(incremental.cache_miss_total, 1);

        state.replace_text("replaced".to_owned());
        let replaced = state.metrics.snapshot();
        assert_eq!(replaced.cache_hit_total, 1);
        assert_eq!(replaced.cache_miss_total, 2);
    }

    #[test]
    fn older_snapshot_cannot_replace_newer() {
        let state = OpenDocument::new("new".to_owned());
        let candidate = DocSnapshot::new(aozora::parse("old").expect("small source").snapshot(), 0);
        state.replace_text("newer".to_owned());
        assert!(!state.install_if_newer(&candidate));
        assert_eq!(state.snapshot().doc_text(), "newer");
    }

    #[test]
    fn slow_parse_threshold_defaults_and_overrides() {
        assert_eq!(parse_slow_threshold(None), 100_000);
        assert_eq!(parse_slow_threshold(Some("42")), 42);
        assert_eq!(parse_slow_threshold(Some("invalid")), 100_000);
    }

    #[test]
    fn slow_parse_threshold_is_exclusive() {
        assert!(!parse_is_slow(41, 42));
        assert!(!parse_is_slow(42, 42));
        assert!(parse_is_slow(43, 42));
    }
}
