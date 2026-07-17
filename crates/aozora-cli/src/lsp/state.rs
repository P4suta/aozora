//! Per-document state — paragraph-first model.
//!
//! Tree-sitter parse is `O(doc-size)`, so the document is segmented into
//! `\n\n`-bounded paragraphs and only the edited paragraph is re-parsed;
//! rope text, line index, and gaiji spans are per-paragraph too.
//!
//! - [`DocBuffer`] (writers): `Vec<ParagraphBuffer>` behind a
//!   `parking_lot::Mutex`, each owning its `Rope` and tree-sitter `Tree`.
//! - [`DocSnapshot`] (readers): `Arc<[Arc<ParagraphSnapshot>]>` swapped via
//!   `ArcSwap` for wait-free loads. Unchanged paragraphs are `Arc::clone`d
//!   across generations, so a small edit rebuilds one paragraph, not the
//!   doc. Doc-level `&str` / line-index / gaiji views are lazy via `OnceLock`.

use std::collections::BTreeMap;
use std::env;
use std::fmt;
use std::mem;
use std::ops::Range;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use arc_swap::ArcSwap;
use parking_lot::Mutex;
use ropey::Rope;
use tokio::runtime::Handle;
use tokio::task::{AbortHandle, spawn_blocking};
use tree_sitter::Parser;

use crate::lsp::gaiji_spans::GaijiSpan;
use crate::lsp::line_index::LineIndex;
use crate::lsp::metrics::{Metrics, ParseSample};
use crate::lsp::paragraph::{
    MAX_PARAGRAPH_BYTES, ParagraphBuffer, ParagraphSnapshot, build_paragraph_snapshot,
    paragraph_byte_ranges,
};
use crate::lsp::parse_cache::{ParseCache, ReparseStats};
use crate::lsp::text_edit::{ByteEdit, EditError};
use crate::lsp::tree_sitter_doc::input_edit;

/// Slice `source` at `range`, build a new owned `Rope` from that
/// slice, and reparse it via `parser`. Used by every code path that
/// constructs a `ParagraphBuffer` from a substring of a larger Rope —
/// `DocBuffer::new`, `replace`, `apply_across_paragraphs`,
/// `maybe_resegment_around`. Centralised so the
/// `byte_slice → Rope::from → reparse` sequence lives in exactly one
/// place.
fn paragraph_from_rope_slice(
    source: &Rope,
    range: Range<usize>,
    parser: &mut Parser,
) -> ParagraphBuffer {
    let slice = source.byte_slice(range);
    let mut paragraph = ParagraphBuffer::new(Rope::from(slice));
    paragraph.reparse(parser);
    paragraph
}

// =====================================================================
// Mutable side: DocBuffer
// =====================================================================

/// Mutable per-document state. Held behind `OpenDocument::buffer`.
///
/// `paragraphs` is the only source-of-truth field. Doc-absolute
/// byte offsets and the total byte length are derived on demand by
/// walking the paragraphs (`O(N)` where N is paragraph count, not
/// document size). At LSP keystroke rates with paragraph counts in
/// the low hundreds this is comfortably under a microsecond per
/// `apply_one_edit` call. The reader-side
/// `DocSnapshot::paragraph_starts` keeps the cumulative-offset table
/// for handlers that need binary-search-by-byte; we don't carry a
/// separate copy here so the writer side stays slim and there's a
/// single place where `paragraph_starts` is recomputed (snapshot
/// build).
///
/// The tree-sitter `Parser` lives here (one per doc) — paragraphs
/// share it serially. Parsers are cheap to keep around but `!Sync`,
/// so we don't spin up one per paragraph.
pub(super) struct DocBuffer {
    pub paragraphs: Vec<ParagraphBuffer>,
    pub parser: Parser,
    /// Edits applied since the parse cache last ran, in cache-text
    /// (pre-edit) coordinates. Drained under this buffer's lock by
    /// [`OpenDocument::reparse_pending`]. Exactly one entry makes the
    /// incremental fast path eligible; anything else re-parses fully.
    pub pending_edits: Vec<ByteEdit>,
}

impl fmt::Debug for DocBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DocBuffer")
            .field("paragraphs", &self.paragraphs.len())
            .finish_non_exhaustive()
    }
}

impl DocBuffer {
    fn new(text: String) -> Self {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_aozora::LANGUAGE.into())
            .expect("tree-sitter-aozora language is compiled in");
        let rope = Rope::from(text);
        let ranges = paragraph_byte_ranges(&rope);
        let mut paragraphs: Vec<ParagraphBuffer> = ranges
            .into_iter()
            .map(|range| paragraph_from_rope_slice(&rope, range, &mut parser))
            .collect();
        if paragraphs.is_empty() {
            // Empty document — keep one empty paragraph so the
            // rest of the code can assume non-empty `paragraphs`.
            paragraphs.push(ParagraphBuffer::new(Rope::new()));
        }
        Self {
            paragraphs,
            parser,
            pending_edits: Vec::new(),
        }
    }

    /// Total byte length of the document — sum of paragraph sizes.
    /// `O(N)` in paragraph count; called at most once per
    /// `validate_edits` invocation so the cost is per-batch, not
    /// per-edit.
    fn total_bytes(&self) -> usize {
        self.paragraphs.iter().map(|p| p.text.len_bytes()).sum()
    }

    /// Apply a batch of edits. Returns `Some(())` on success and
    /// `None` if the batch failed validation (state unchanged).
    ///
    /// The batch is pre-validated against the doc-wide byte range
    /// invariants; per-edit application happens in REVERSE source
    /// order so each edit's pre-shift offsets stay valid against
    /// the still-pre-edit prefix.
    fn apply_edits(&mut self, edits: &[ByteEdit]) -> Option<()> {
        if let Err(err) = self.validate_edits(edits) {
            tracing::warn!(
                error = %err,
                text_bytes = self.total_bytes(),
                "rejecting incremental edit batch; document state unchanged",
            );
            return None;
        }
        for edit in edits.iter().rev() {
            self.apply_one_edit(edit);
        }
        // Record the batch (in pre-edit coordinates) so the debounced
        // reparse can drain it and reuse untouched segments. Pushed only
        // after `validate_edits` passed, so a rejected batch never
        // accumulates.
        self.pending_edits.extend_from_slice(edits);
        Some(())
    }

    fn validate_edits(&self, edits: &[ByteEdit]) -> Result<(), EditError> {
        let len = self.total_bytes();
        let mut prev_end = 0usize;
        for edit in edits {
            let start = edit.range.start;
            let end = edit.range.end;
            if end < start {
                return Err(EditError::InvertedRange { start, end });
            }
            if end > len {
                return Err(EditError::OutOfBounds {
                    start,
                    end,
                    source_len: len,
                });
            }
            if !self.is_char_boundary(start) || !self.is_char_boundary(end) {
                return Err(EditError::NonCharBoundary { start, end });
            }
            if start < prev_end {
                return Err(EditError::UnsortedOrOverlapping {
                    prev_end,
                    next_start: start,
                });
            }
            prev_end = end;
        }
        Ok(())
    }

    fn is_char_boundary(&self, doc_byte: usize) -> bool {
        let total = self.total_bytes();
        if doc_byte == 0 || doc_byte == total {
            return true;
        }
        if doc_byte > total {
            return false;
        }
        let (idx, local) = self.locate_byte(doc_byte);
        let rope = &self.paragraphs[idx].text;
        if local == 0 || local == rope.len_bytes() {
            return true;
        }
        let (chunk, chunk_byte_idx, _, _) = rope.chunk_at_byte(local);
        let in_chunk = local - chunk_byte_idx;
        chunk.is_char_boundary(in_chunk)
    }

    /// Resolve a doc-absolute byte to (`paragraph_idx`, `local_byte`).
    ///
    /// At a paragraph boundary `b == paragraphs[i].end ==
    /// paragraphs[i+1].start`, the byte is reported as belonging to
    /// the **right** paragraph (`paragraph_idx == i + 1`,
    /// `local_byte == 0`) — consistent with `paragraph_byte_ranges`,
    /// where the boundary byte is the inclusive start of the right
    /// half-open range. Only the doc-end byte (`b == total_bytes`)
    /// belongs to the last paragraph as `local_byte == len`, since
    /// there is no rightward paragraph to take ownership of it.
    ///
    /// `O(N)` walk over paragraphs (no cumulative-offset cache on
    /// the writer side). At LSP keystroke rates with paragraph
    /// counts in the low hundreds this stays sub-microsecond.
    fn locate_byte(&self, doc_byte: usize) -> (usize, usize) {
        let mut acc = 0usize;
        let last = self.paragraphs.len().saturating_sub(1);
        for (idx, paragraph) in self.paragraphs.iter().enumerate() {
            let len = paragraph.text.len_bytes();
            // `<` so the boundary at `acc + len` falls through to the
            // rightward paragraph, which then sees `local_byte == 0`.
            // The final paragraph also catches `acc + len` exactly via
            // the `idx == last` short-circuit, since there is no
            // rightward paragraph to take ownership of
            // `doc_byte == total_bytes`.
            if doc_byte < acc + len || idx == last {
                return (idx, doc_byte.saturating_sub(acc));
            }
            acc += len;
        }
        // Unreachable in practice — the `idx == last` arm always
        // matches in the loop above. Kept defensive for the
        // empty-doc case that DocBuffer::new pre-fills.
        (0, 0)
    }

    fn apply_one_edit(&mut self, edit: &ByteEdit) {
        let start = self.locate_byte(edit.range.start);
        let end = self.locate_byte(edit.range.end);
        if start.0 == end.0 {
            self.apply_within_paragraph(start.0, start.1..end.1, &edit.new_text);
        } else {
            self.apply_across_paragraphs(start, end, &edit.new_text);
        }
        self.maybe_resegment_around(start.0);
    }

    fn apply_within_paragraph(&mut self, idx: usize, local: Range<usize>, new_text: &str) {
        let paragraph = &mut self.paragraphs[idx];
        let start_char = paragraph.text.byte_to_char(local.start);
        let end_char = paragraph.text.byte_to_char(local.end);
        if end_char > start_char {
            paragraph.text.remove(start_char..end_char);
        }
        if !new_text.is_empty() {
            paragraph.text.insert(start_char, new_text);
        }
        let new_end_local = local.start + new_text.len();
        // The `InputEdit`'s byte offsets are paragraph-local — that's
        // what `ParagraphBuffer::apply_edit` expects.
        let ts_edit = input_edit(local.start, local.end, new_end_local);
        paragraph.apply_edit(&mut self.parser, ts_edit);
    }

    /// Cross-paragraph edit: build the merged Rope without
    /// materialising any intermediate `String`s. Re-segment the
    /// merged content and replace `paragraphs[start_para..=end_para]`
    /// with the resulting per-paragraph trees.
    ///
    /// **Why a full reparse over the merged region**: distinguishing
    /// "the boundary `\n\n` was deleted, paragraphs collapse" from
    /// "an edit straddled the boundary but produced the same shape"
    /// requires diffing the segmentation outcome against the prior
    /// shape, then matching trees by something other than tree id
    /// (since both old paragraphs' trees are stale relative to the
    /// new merged content). The per-paragraph reuse path on the
    /// snapshot side already handles "subsequent paragraphs reused
    /// via `Arc::clone` on the unaffected suffix"; this writer-side
    /// reparse pays at most `O(merged_size)`, which for typical
    /// boundary-spanning edits is bounded to ~10 KB.
    fn apply_across_paragraphs(
        &mut self,
        start: (usize, usize),
        end: (usize, usize),
        new_text: &str,
    ) {
        let (start_para, start_local) = start;
        let (end_para, end_local) = end;
        // Build the merged Rope by zero-copy `append` of slices from
        // the existing paragraphs' Ropes. The middle `new_text`
        // becomes a tiny owned Rope; everything else stays in
        // structural-share territory.
        let mut merged = Rope::from(self.paragraphs[start_para].text.byte_slice(..start_local));
        if !new_text.is_empty() {
            merged.append(Rope::from(new_text));
        }
        merged.append(Rope::from(
            self.paragraphs[end_para].text.byte_slice(end_local..),
        ));

        let ranges = paragraph_byte_ranges(&merged);
        let mut replacement: Vec<ParagraphBuffer> = ranges
            .into_iter()
            .map(|range| paragraph_from_rope_slice(&merged, range, &mut self.parser))
            .collect();
        if replacement.is_empty() {
            replacement.push(ParagraphBuffer::new(Rope::new()));
        }
        self.paragraphs.splice(start_para..=end_para, replacement);
    }

    /// If the paragraph at `idx` grew past the cap (due to an
    /// in-paragraph insert), re-split it by content and reparse the
    /// resulting pieces. Otherwise no-op.
    fn maybe_resegment_around(&mut self, idx: usize) {
        if idx >= self.paragraphs.len() {
            return;
        }
        let len = self.paragraphs[idx].text.len_bytes();
        if len <= MAX_PARAGRAPH_BYTES {
            return;
        }
        // Re-segment the paragraph's text by paragraph_byte_ranges
        // (will hard-cap at MAX_PARAGRAPH_BYTES).
        let text_rope = mem::replace(&mut self.paragraphs[idx].text, Rope::new());
        let ranges = paragraph_byte_ranges(&text_rope);
        if ranges.len() <= 1 {
            // Single-segment result — restore and return; the cap
            // hard-split was a no-op (paragraph is exactly cap-sized).
            self.paragraphs[idx].text = text_rope;
            self.paragraphs[idx].reparse(&mut self.parser);
            return;
        }
        let replacement: Vec<ParagraphBuffer> = ranges
            .into_iter()
            .map(|range| paragraph_from_rope_slice(&text_rope, range, &mut self.parser))
            .collect();
        self.paragraphs.splice(idx..=idx, replacement);
    }

    fn replace(&mut self, new_text: String) {
        let rope = Rope::from(new_text);
        let ranges = paragraph_byte_ranges(&rope);
        let mut paragraphs: Vec<ParagraphBuffer> = ranges
            .into_iter()
            .map(|range| paragraph_from_rope_slice(&rope, range, &mut self.parser))
            .collect();
        if paragraphs.is_empty() {
            paragraphs.push(ParagraphBuffer::new(Rope::new()));
        }
        self.paragraphs = paragraphs;
        // A wholesale replace invalidates the accumulated edits relative
        // to the parse cache's prior text; clearing them makes the next
        // reparse an honest full parse (an empty drain takes that path).
        self.pending_edits.clear();
    }
}

// =====================================================================
// Read side: DocSnapshot
// =====================================================================

/// Immutable read view of a document. Built from a [`DocBuffer`]
/// snapshot and atomically swapped into [`OpenDocument::snapshot`]. Reads
/// are wait-free (one `ArcSwap::load_full` + Arc clones).
pub(super) struct DocSnapshot {
    pub paragraphs: Arc<[Arc<ParagraphSnapshot>]>,
    /// `paragraph_starts[i]` = doc-absolute byte where paragraph `i`
    /// begins. Sorted ascending. Lets handlers binary-search a
    /// doc-absolute offset to a paragraph in `O(log n)`. Consulted only by
    /// [`DocSnapshot::paragraph_at`], today an in-module-test accessor.
    #[cfg(test)]
    pub paragraph_starts: Arc<[u32]>,
    pub total_bytes: u32,
    pub version: u64,

    // Lazy doc-level materialisations. Each `OnceLock` is populated
    // by the first call to its accessor; subsequent calls within the
    // lifetime of this `DocSnapshot` return the cached `Arc` for free.
    doc_text: OnceLock<Arc<str>>,
    doc_rope: OnceLock<Arc<Rope>>,
    doc_line_index: OnceLock<Arc<LineIndex>>,
    doc_gaiji_spans: OnceLock<Arc<BTreeMap<u32, Arc<GaijiSpan>>>>,
}

impl fmt::Debug for DocSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DocSnapshot")
            .field("version", &self.version)
            .field("paragraphs", &self.paragraphs.len())
            .field("total_bytes", &self.total_bytes)
            .finish_non_exhaustive()
    }
}

impl DocSnapshot {
    /// Doc-wide concatenated text, materialised on first request and
    /// cached for the rest of this snapshot's lifetime. Handlers that
    /// can iterate paragraphs directly should prefer the per-paragraph
    /// accessor and skip this O(n) materialisation entirely.
    #[must_use]
    pub(super) fn doc_text(&self) -> &Arc<str> {
        self.doc_text.get_or_init(|| {
            let total = self.total_bytes as usize;
            let mut buf = String::with_capacity(total);
            for paragraph in self.paragraphs.iter() {
                buf.push_str(&paragraph.text);
            }
            Arc::from(buf)
        })
    }

    /// Doc-wide [`Rope`] assembled by structural-share `append` of each
    /// paragraph's text, materialised on first request and cached for this
    /// snapshot's lifetime. Used by the `did_open` publish path to map
    /// diagnostic spans with `O(log n)` rope line lookups (via
    /// [`crate::lsp::doc_line_view::DocLineView::Rope`]) instead of an `O(doc)`
    /// [`LineIndex`] rebuild. Line metrics count only `\n` because the
    /// workspace pins ropey with its `unicode_lines` feature off, so this
    /// is byte-identical to indexing `doc_text()` with [`LineIndex`].
    #[must_use]
    pub(super) fn doc_rope(&self) -> &Arc<Rope> {
        self.doc_rope.get_or_init(|| {
            let mut rope = Rope::new();
            for paragraph in self.paragraphs.iter() {
                rope.append(Rope::from(&*paragraph.text));
            }
            Arc::new(rope)
        })
    }

    /// Doc-wide line index, lazily materialised. Built by re-scanning
    /// `doc_text()` (forces that materialisation as a side effect).
    #[must_use]
    pub(super) fn doc_line_index(&self) -> &Arc<LineIndex> {
        self.doc_line_index.get_or_init(|| {
            let text = self.doc_text();
            Arc::new(LineIndex::new(text))
        })
    }

    /// Doc-wide gaiji-span store keyed by `start_byte`. Concatenates
    /// each paragraph's pre-extracted spans (whose offsets are
    /// already doc-absolute, see `crate::lsp::paragraph`).
    #[must_use]
    pub(super) fn doc_gaiji_spans(&self) -> &Arc<BTreeMap<u32, Arc<GaijiSpan>>> {
        self.doc_gaiji_spans.get_or_init(|| {
            let mut map = BTreeMap::new();
            for paragraph in self.paragraphs.iter() {
                for span in paragraph.gaiji_spans.iter() {
                    map.insert(span.start_byte, Arc::clone(span));
                }
            }
            Arc::new(map)
        })
    }

    /// Find the paragraph index that contains `doc_byte`. Returns
    /// `None` only when the snapshot has zero paragraphs (which we
    /// avoid in practice — empty documents still have one
    /// zero-length paragraph).
    ///
    /// At a boundary `b == paragraph_starts[i + 1]`, `doc_byte`
    /// resolves to the **right** paragraph (index `i + 1`),
    /// matching `paragraph_byte_ranges`'s half-open ranges (where
    /// the boundary is the inclusive start of the right range).
    /// Only `doc_byte == total_bytes` resolves to the last
    /// paragraph, since no rightward paragraph exists.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn paragraph_at(&self, doc_byte: usize) -> Option<usize> {
        if self.paragraph_starts.is_empty() {
            return None;
        }
        let target = u32::try_from(doc_byte).unwrap_or(u32::MAX);
        let i = self
            .paragraph_starts
            .partition_point(|&s| s <= target)
            .saturating_sub(1);
        Some(i)
    }
}

fn build_snapshot(buffer: &DocBuffer, version: u64, prior: &DocSnapshot) -> Arc<DocSnapshot> {
    // Per-paragraph rebuild: for each paragraph in the new buffer,
    // try to reuse the prior snapshot's paragraph by tree-id match
    // (cheap: `Arc::clone` if matched, full materialisation if not).
    //
    // This is the hot-path payoff of the paragraph-first model: an
    // edit affecting paragraph K leaves paragraphs ≠ K with the
    // same Tree id, so we Arc::clone N - 1 paragraphs for free and
    // pay materialisation only on paragraph K.
    let mut paragraphs: Vec<Arc<ParagraphSnapshot>> = Vec::with_capacity(buffer.paragraphs.len());
    #[cfg(test)]
    let mut starts: Vec<u32> = Vec::with_capacity(buffer.paragraphs.len());
    let mut acc: u32 = 0;
    for (idx, paragraph) in buffer.paragraphs.iter().enumerate() {
        #[cfg(test)]
        starts.push(acc);
        let live_id = paragraph.tree_id();
        let new_start = acc as usize;
        let snap = match prior.paragraphs.get(idx) {
            Some(prior_p)
                if prior_p.tree_id == live_id
                    && prior_p.byte_range.len() == paragraph.text.len_bytes() =>
            {
                // Reuse: `shifted_to` handles both the in-place
                // (pure Arc bump) and shifted (share text/line_index/
                // tree, re-emit gaiji spans) cases internally.
                ParagraphSnapshot::shifted_to(prior_p, new_start)
            }
            _ => Arc::new(build_paragraph_snapshot(paragraph, new_start)),
        };
        let bytes = u32::try_from(paragraph.text.len_bytes()).unwrap_or(u32::MAX);
        acc = acc.saturating_add(bytes);
        paragraphs.push(snap);
    }
    if paragraphs.is_empty() {
        // Defensive: should never happen because DocBuffer
        // guarantees at least one paragraph, but DocSnapshot's
        // accessors degrade gracefully if it does.
        paragraphs.push(Arc::new(build_paragraph_snapshot(
            &ParagraphBuffer::new(Rope::new()),
            0,
        )));
        #[cfg(test)]
        starts.push(0);
    }
    Arc::new(DocSnapshot {
        paragraphs: paragraphs.into(),
        #[cfg(test)]
        paragraph_starts: starts.into(),
        total_bytes: acc,
        version,
        doc_text: OnceLock::new(),
        doc_rope: OnceLock::new(),
        doc_line_index: OnceLock::new(),
        doc_gaiji_spans: OnceLock::new(),
    })
}

fn empty_snapshot() -> Arc<DocSnapshot> {
    let empty_para = Arc::new(build_paragraph_snapshot(
        &ParagraphBuffer::new(Rope::new()),
        0,
    ));
    Arc::new(DocSnapshot {
        paragraphs: Arc::from(vec![empty_para]),
        #[cfg(test)]
        paragraph_starts: Arc::from(vec![0u32]),
        total_bytes: 0,
        version: 0,
        doc_text: OnceLock::new(),
        doc_rope: OnceLock::new(),
        doc_line_index: OnceLock::new(),
        doc_gaiji_spans: OnceLock::new(),
    })
}

// =====================================================================
// OpenDocument orchestrator
// =====================================================================

pub(super) struct OpenDocument {
    buffer: Mutex<DocBuffer>,
    /// Segment cache for the aozora semantic parse (diagnostics), held
    /// under its **own** lock — separate from `buffer` — so the debounced
    /// reparse never blocks the edit path. A reparse holds `parse` for the
    /// whole parse but `buffer` only for the µs it takes to clone the
    /// paragraph ropes and drain `pending_edits`. Holding `parse` across
    /// the drain also serialises reparses (single-flight, in schedule
    /// order), so the incremental fast path always sees a consistent prior
    /// segmentation and no two reparses race on the stored result.
    parse: Mutex<ParseCache>,
    snapshot: ArcSwap<DocSnapshot>,
    edit_version: AtomicU64,
    pub metrics: Arc<Metrics>,
    /// Abort handle for the most recently scheduled debounced publish
    /// task. Bounds in-flight debounce tasks to one per document under
    /// an edit flood (see [`Self::replace_debounce_task`]).
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
    /// Build a new `OpenDocument` and synchronously compute the initial
    /// snapshot.
    #[must_use]
    pub(super) fn new(text: String) -> Arc<Self> {
        let buffer = DocBuffer::new(text);
        let initial = build_snapshot(&buffer, 0, &empty_snapshot());
        let state = Arc::new(Self {
            buffer: Mutex::new(buffer),
            parse: Mutex::new(ParseCache::default()),
            snapshot: ArcSwap::from(initial),
            edit_version: AtomicU64::new(0),
            metrics: Arc::new(Metrics::default()),
            debounce_task: Mutex::new(None),
        });
        // Synchronous initial parse: `pending_edits` is empty, so this is
        // a full parse that populates the cache's segmentation and
        // diagnostics. `did_open`'s immediate publish reads them.
        state.run_parse_cache_reparse();
        state
    }

    /// Wait-free read of the current snapshot.
    #[must_use]
    pub(super) fn snapshot(&self) -> Arc<DocSnapshot> {
        self.snapshot.load_full()
    }

    pub(super) fn edit_version(&self) -> u64 {
        self.edit_version.load(Ordering::SeqCst)
    }

    /// Install the abort handle for the most recently scheduled
    /// debounced publish task, aborting the previous one if it is still
    /// pending. Bounds in-flight debounce tasks to one per document so
    /// an adversarial edit flood cannot accumulate sleeping tasks; the
    /// `edit_version` guard in the task body remains the correctness
    /// mechanism (only the latest version actually publishes).
    pub(super) fn replace_debounce_task(&self, handle: AbortHandle) {
        let prev = self.debounce_task.lock().replace(handle);
        if let Some(prev) = prev {
            prev.abort();
        }
    }

    pub(super) fn with_parse_cache<R>(&self, f: impl FnOnce(&ParseCache) -> R) -> R {
        let cache = self.parse.lock();
        f(&cache)
    }

    /// Apply a batch of edits and ratchet the snapshot.
    pub(super) fn apply_changes(self: &Arc<Self>, edits: &[ByteEdit]) -> Option<u64> {
        // Hold the buffer mutex only across the actual edit; drop it
        // explicitly before the metrics + version + snapshot-rebuild
        // tail so concurrent readers don't queue behind this writer
        // for any longer than the edit itself takes
        // (`clippy::significant_drop_tightening` enforces this).
        let mut buffer = self.buffer.lock();
        buffer.apply_edits(edits)?;
        drop(buffer);
        self.metrics.record_edit();
        let new_version = self.edit_version.fetch_add(1, Ordering::SeqCst) + 1;
        self.spawn_snapshot_rebuild(new_version);
        Some(new_version)
    }

    /// Replace the buffer wholesale.
    pub(super) fn replace_text(self: &Arc<Self>, new_text: String) -> u64 {
        let mut buffer = self.buffer.lock();
        buffer.replace(new_text);
        drop(buffer);
        self.metrics.record_edit();
        let new_version = self.edit_version.fetch_add(1, Ordering::SeqCst) + 1;
        self.spawn_snapshot_rebuild(new_version);
        new_version
    }

    /// Synchronous snapshot rebuild — used by tests and the bg
    /// blocking-pool task body. Holds the buffer mutex briefly to
    /// snapshot the paragraph state, then drops it before doing the
    /// per-paragraph snapshot construction (which only touches text
    /// already snapshot-ed).
    pub(super) fn rebuild_snapshot_now(&self) {
        let prior = self.snapshot.load_full();
        let candidate = {
            let buffer = self.buffer.lock();
            let version = self.edit_version.load(Ordering::SeqCst);
            build_snapshot(&buffer, version, &prior)
        };
        self.install_if_newer(&candidate);
    }

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

    fn spawn_snapshot_rebuild(self: &Arc<Self>, target_version: u64) {
        let this = Arc::clone(self);
        if Handle::try_current().is_ok() {
            spawn_blocking(move || {
                if this.snapshot.load().version >= target_version {
                    return;
                }
                this.rebuild_snapshot_now();
            });
        } else {
            this.rebuild_snapshot_now();
        }
    }

    /// Re-parse the live buffer through the incremental parse cache and return
    /// the parsed text as a doc-level [`Rope`], its diagnostics, and the
    /// `edit_version` that text reflects.
    ///
    /// The accumulated edits are forwarded to
    /// `ParseCache::reparse_incremental`, which splices its maintained
    /// `PieceSeq` incrementally on a single edit — LF or CRLF alike, via the
    /// sanitized-rope Mechanism B (#237 Tier 2) — and full-parses otherwise. The
    /// result is always identical to a from-scratch parse.
    ///
    /// The returned [`Rope`] is assembled by structural-share `append` of the
    /// cloned paragraph ropes (no byte copy), so the publish path can map
    /// diagnostic spans with `O(log n)` rope line lookups
    /// ([`crate::lsp::doc_line_view::DocLineView::Rope`]) instead of rebuilding an
    /// `O(doc)` [`LineIndex`] per keystroke (#237 Tier 2, Mechanism A). The same
    /// rope is fed straight to the cache, which splices its sanitized rope
    /// incrementally — no per-keystroke `O(doc)` `to_string` + `sanitize`.
    ///
    /// Locking is the load-bearing part: the `parse` lock is held across
    /// the whole call, so reparses are single-flight and serialise in
    /// schedule order — the stored result is monotonic. The `buffer`
    /// lock is taken only to clone the paragraph ropes (`O(1)` each via
    /// ropey structural sharing) and drain `pending_edits`, so the edit
    /// path is never blocked by the parse itself.
    pub(super) fn reparse_pending(&self) -> (Rope, Vec<aozora::Diagnostic>, u64) {
        // Acquire `parse` first and hold it across the drain + parse: this
        // serialises reparses (single-flight, in schedule order) so the
        // incremental fast path always sees a consistent prior segmentation
        // and the stored result is monotonic. Dropped right after the
        // parse, before recording stats.
        let mut cache = self.parse.lock();
        // Brief buffer lock: snapshot the paragraph ropes (cheap
        // structural-sharing clones) and drain the accumulated edits.
        let (ropes, edits, parsed_version) = {
            let mut buffer = self.buffer.lock();
            let ropes: Vec<Rope> = buffer.paragraphs.iter().map(|p| p.text.clone()).collect();
            let edits = mem::take(&mut buffer.pending_edits);
            drop(buffer);
            // Capture the version this parse reflects right after the drain
            // for the backend's publish guard. (`edit_version` is bumped
            // outside the buffer lock, so a one-behind read here at worst
            // defers a publish to the next debounce task — never wrong.)
            let parsed_version = self.edit_version.load(Ordering::SeqCst);
            (ropes, edits, parsed_version)
        };
        // Assemble a doc-level rope by structural-share `append` (O(#paragraphs
        // · log n), no byte copy). Returned to the publish path so it can build
        // a `DocLineView::Rope` with no LineIndex rebuild, and fed straight to
        // the cache — which splices its sanitized rope incrementally, with no
        // per-keystroke `O(doc)` `to_string` + `sanitize` (#237 Tier 2,
        // Mechanism B).
        let mut raw = Rope::new();
        for rope in ropes {
            raw.append(rope);
        }
        let (diagnostics, stats) = cache.reparse_incremental(&raw, &edits);
        drop(cache);
        self.record_parse_stats(stats);
        (raw, diagnostics, parsed_version)
    }

    /// Feed a reparse's statistics into the per-document metrics and warn
    /// when a parse blew past the slow-path threshold.
    fn record_parse_stats(&self, stats: ReparseStats) {
        self.metrics.record_parse(ParseSample {
            latency_us: stats.latency_us,
            cache_hits: stats.cache_hits,
            cache_misses: stats.cache_misses,
            cache_entries: stats.cache_entries_after,
            cache_bytes_estimate: stats.cache_bytes_estimate,
        });
        let threshold = slow_parse_threshold_us();
        if stats.latency_us > threshold {
            tracing::warn!(
                latency_us = stats.latency_us,
                threshold_us = threshold,
                parse_count = stats.parse_count,
                cache_hits = stats.cache_hits,
                cache_misses = stats.cache_misses,
                "parse exceeded slow-path threshold",
            );
        }
    }

    /// Full/initial reparse entry point used by `OpenDocument::new` and
    /// tests. Drains any pending edits (empty at construction ⇒ full
    /// parse) and records stats; callers that only need the cache
    /// populated discard the returned text and diagnostics.
    pub(super) fn run_parse_cache_reparse(&self) {
        drop(self.reparse_pending());
    }
}

fn slow_parse_threshold_us() -> u64 {
    env::var("AOZORA_LSP_SLOW_PARSE_US")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(100_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> Arc<OpenDocument> {
        OpenDocument::new(text.to_owned())
    }

    #[test]
    fn new_doc_publishes_initial_snapshot() {
        let state = doc("hello");
        let snap = state.snapshot();
        assert_eq!(&**snap.doc_text(), "hello");
        assert_eq!(snap.version, 0);
        assert!(snap.doc_gaiji_spans().is_empty());
    }

    #[test]
    fn apply_changes_ratchets_edit_version() {
        let state = doc("hello");
        let v = state
            .apply_changes(&[ByteEdit::new(5..5, " world".to_owned())])
            .expect("valid edit");
        assert_eq!(v, 1);
        assert_eq!(state.edit_version(), 1);
        let snap = state.snapshot();
        assert_eq!(&**snap.doc_text(), "hello world");
        assert_eq!(snap.version, 1);
    }

    fn diag_debug(ds: &[aozora::Diagnostic]) -> Vec<String> {
        ds.iter().map(|d| format!("{d:?}")).collect()
    }

    #[test]
    fn single_edit_debounce_reuses_flanking_nodes() {
        // LF-clean (source == sanitized) three-paragraph doc whose first
        // paragraph carries a ruby node; one interior edit in the plain middle
        // paragraph takes the incremental fast path, reusing the prefix
        // ruby — so the cumulative hit total and hit rate go non-zero (#237
        // Stage B'3).
        let src = "｜青空《あおぞら》のした。\n\n段落に。\n\n段落さん。";
        let state = doc(src);
        // One interior edit in the middle paragraph.
        let at = src.find("段落に").expect("middle paragraph") + "段落に".len();
        state
            .apply_changes(&[ByteEdit::new(at..at, "ん".to_owned())])
            .expect("valid edit");

        let (raw, diags, _ver) = state.reparse_pending();
        let text = raw.to_string();

        let snap = state.metrics.snapshot();
        assert!(
            snap.cache_hit_total > 0,
            "a single LF-clean interior edit reuses flanking nodes: {snap:?}",
        );
        assert!(
            snap.cache_hit_rate > 0.0,
            "a non-zero hit total drives a non-zero hit rate: {snap:?}",
        );
        // Diagnostics identical to a from-scratch parse of the edited text.
        let mut fresh = ParseCache::default();
        let (want, _) = fresh.reparse(&text);
        assert_eq!(diag_debug(&diags), diag_debug(&want));
    }

    #[test]
    fn multi_edit_window_falls_back_to_full_parse() {
        let src = "段落いち。\n\n段落に。\n\n段落さん。";
        let state = doc(src);
        // Two edits accumulate in the debounce window before the reparse.
        let a = src.find("いち").expect("first paragraph");
        state
            .apply_changes(&[ByteEdit::new(a..a, "Ｘ".to_owned())])
            .expect("valid edit 1");
        let snap = state.snapshot();
        let b = snap.doc_text().find("さん").expect("third paragraph");
        drop(snap);
        state
            .apply_changes(&[ByteEdit::new(b..b, "Ｙ".to_owned())])
            .expect("valid edit 2");

        let (raw, diags, _ver) = state.reparse_pending();
        let text = raw.to_string();

        // Neither the initial parse nor the multi-edit reparse reused a
        // segment, so the cumulative hit total stays zero.
        let metrics = state.metrics.snapshot();
        assert_eq!(
            metrics.cache_hit_total, 0,
            "multi-edit batch re-parses fully: {metrics:?}"
        );
        let mut fresh = ParseCache::default();
        let (want, _) = fresh.reparse(&text);
        assert_eq!(diag_debug(&diags), diag_debug(&want));
    }

    #[test]
    fn replace_text_clears_pending_and_full_parses() {
        let state = doc("古い段落。\n\nもうひとつ。");
        state
            .apply_changes(&[ByteEdit::new(0..0, "Ｘ".to_owned())])
            .expect("valid edit");
        // A wholesale replace must discard the accumulated edit so the next
        // reparse is an honest full parse of the replaced text (the
        // byte-equality guard also backstops correctness).
        state.replace_text("まったく新しい本文。".to_owned());

        let (raw, diags, _ver) = state.reparse_pending();
        let text = raw.to_string();
        assert_eq!(text, "まったく新しい本文。");
        let mut fresh = ParseCache::default();
        let (want, _) = fresh.reparse(&text);
        assert_eq!(diag_debug(&diags), diag_debug(&want));
    }

    #[test]
    fn replace_text_ratchets_edit_version() {
        let state = doc("hello");
        let v = state.replace_text("world".to_owned());
        assert_eq!(v, 1);
        let snap = state.snapshot();
        assert_eq!(&**snap.doc_text(), "world");
        assert_eq!(snap.version, 1);
    }

    #[test]
    fn rejected_edit_leaves_state_unchanged() {
        let state = doc("あ");
        let edit = ByteEdit::new(1..2, String::new());
        assert!(state.apply_changes(&[edit]).is_none());
        let snap = state.snapshot();
        assert_eq!(&**snap.doc_text(), "あ");
        assert_eq!(snap.version, 0);
        assert_eq!(state.edit_version(), 0);
    }

    #[test]
    fn snapshot_loads_are_lock_free_after_install() {
        let state = doc("｜青空《あおぞら》");
        let s1 = state.snapshot();
        let s2 = state.snapshot();
        assert!(Arc::ptr_eq(&s1, &s2));
    }

    #[test]
    fn paragraph_split_on_blank_line() {
        let state = doc("段落1\n\n段落2");
        let snap = state.snapshot();
        assert_eq!(snap.paragraphs.len(), 2, "{snap:?}");
        // Document-absolute first byte of paragraph 1 is right after
        // the blank-line boundary inside paragraph 0.
        assert!(snap.paragraph_starts[1] > 0);
    }

    #[test]
    fn within_paragraph_edit_only_touches_one_paragraph_snapshot() {
        let state = doc("段落1\n\n段落2\n\n段落3");
        let snap_before = state.snapshot();
        assert_eq!(snap_before.paragraphs.len(), 3);
        let para0_before = Arc::clone(&snap_before.paragraphs[0]);
        let text2_before = Arc::clone(&snap_before.paragraphs[2].text);
        let line2_before = Arc::clone(&snap_before.paragraphs[2].line_index);

        // Insert inside paragraph 1.
        let mid_para1 = "段落1\n\n段".len();
        state
            .apply_changes(&[ByteEdit::new(mid_para1..mid_para1, "X".to_owned())])
            .unwrap();
        let snap_after = state.snapshot();

        // Paragraph 0 is in-place + unchanged — pure Arc bump (same pointer).
        assert!(Arc::ptr_eq(&snap_after.paragraphs[0], &para0_before));
        // Paragraph 1 is a fresh Arc (its tree was reparsed).
        assert!(!Arc::ptr_eq(
            &snap_after.paragraphs[1],
            &snap_before.paragraphs[1]
        ));
        // Paragraph 2's outer Arc is fresh (because byte_range shifted)
        // BUT the inner text + line_index Arcs ARE shared with the
        // prior snapshot — the only newly-allocated piece is the
        // gaiji-spans list (with shifted offsets) plus the small
        // `Arc<ParagraphSnapshot>` itself.
        assert!(Arc::ptr_eq(&snap_after.paragraphs[2].text, &text2_before));
        assert!(Arc::ptr_eq(
            &snap_after.paragraphs[2].line_index,
            &line2_before
        ));
    }

    #[test]
    fn doc_text_caches_after_first_call() {
        let state = doc("hello\n\nworld");
        let snap = state.snapshot();
        let t1 = snap.doc_text();
        let t2 = snap.doc_text();
        assert!(Arc::ptr_eq(t1, t2), "OnceLock should cache");
        assert_eq!(&**t1, "hello\n\nworld");
    }

    #[test]
    fn paragraph_at_resolves_doc_byte() {
        let state = doc("一\n\n二\n\n三");
        let snap = state.snapshot();
        assert_eq!(snap.paragraph_at(0), Some(0));
        // After the first \n\n, we should be in paragraph 1.
        let after_first_blank = "一\n\n".len();
        assert_eq!(snap.paragraph_at(after_first_blank), Some(1));
    }

    /// Round-trip invariant: a sequence of in-place inserts at
    /// monotonically advancing offsets must leave the buffer
    /// byte-identical to the equivalent doc-built-from-text. This is
    /// the strongest cross-check between the paragraph-segmented
    /// edit path and the cold-start path.
    #[test]
    fn sequential_inserts_match_cold_start_text() {
        let state = doc("");
        let chunks = ["｜青空", "《", "あおぞら", "》", "の", "下"];
        let mut expected = String::new();
        for chunk in chunks {
            let pos = expected.len();
            state
                .apply_changes(&[ByteEdit::new(pos..pos, chunk.to_owned())])
                .expect("valid edit");
            expected.push_str(chunk);
        }
        assert_eq!(&**state.snapshot().doc_text(), &expected);
        // And a fresh OpenDocument built from the same final text must
        // produce the same paragraph shape.
        let cold = doc(&expected);
        assert_eq!(
            &**cold.snapshot().doc_text(),
            &**state.snapshot().doc_text(),
        );
    }

    /// Cross-paragraph delete that collapses the `\n\n` boundary
    /// between two paragraphs must merge them and keep doc text
    /// consistent. The snapshot must report exactly one paragraph
    /// after the merge.
    #[test]
    fn cross_paragraph_delete_collapses_boundary() {
        let state = doc("段落1\n\n段落2");
        let pre = state.snapshot();
        assert_eq!(pre.paragraphs.len(), 2);
        // Delete the entire `\n\n` boundary.
        let blank_at = "段落1".len();
        let edit = ByteEdit::new(blank_at..blank_at + 2, String::new());
        state.apply_changes(&[edit]).expect("valid edit");
        let post = state.snapshot();
        assert_eq!(&**post.doc_text(), "段落1段落2");
        assert_eq!(post.paragraphs.len(), 1, "{post:?}");
    }

    /// In-place insert inside an existing `\n\n` widens the gap but
    /// must NOT create a third paragraph (the boundary policy keeps
    /// blank-line runs collapsed to one boundary).
    #[test]
    fn insert_inside_blank_line_preserves_two_paragraphs() {
        let state = doc("一\n\n二");
        let blank_at = "一\n".len();
        let edit = ByteEdit::new(blank_at..blank_at, "\n".to_owned());
        state.apply_changes(&[edit]).expect("valid edit");
        let snap = state.snapshot();
        // Three newlines in a row → still two paragraphs.
        assert_eq!(&**snap.doc_text(), "一\n\n\n二");
        assert_eq!(snap.paragraphs.len(), 2, "{snap:?}");
    }

    /// Empty-text replace must leave the state in a queryable shape:
    /// one (empty) paragraph, zero total bytes, `paragraph_at(0)`
    /// returns Some(0). Pin so the empty-doc invariant stays valid.
    #[test]
    fn replace_with_empty_text_yields_one_empty_paragraph() {
        let state = doc("｜青空《あおぞら》");
        state.replace_text(String::new());
        let snap = state.snapshot();
        assert_eq!(&**snap.doc_text(), "");
        assert_eq!(snap.paragraphs.len(), 1);
        assert_eq!(snap.total_bytes, 0);
        assert_eq!(snap.paragraph_at(0), Some(0));
    }

    /// Boundary case: an edit at the exact end of the document must
    /// be accepted (it's an append) and must not fall through to the
    /// "out of bounds" rejection path.
    #[test]
    fn append_at_eof_is_accepted() {
        let state = doc("hello");
        let len = "hello".len();
        let edit = ByteEdit::new(len..len, " world".to_owned());
        assert!(state.apply_changes(&[edit]).is_some());
        assert_eq!(&**state.snapshot().doc_text(), "hello world");
    }

    /// Multiple sorted edits in one batch compose correctly. The
    /// reverse-order application inside `apply_one_edit` must not
    /// corrupt offsets for later edits whose ranges sit AFTER the
    /// first's.
    #[test]
    fn batched_edits_compose_in_source_order() {
        let state = doc("AAAA BBBB CCCC");
        let edits = vec![
            ByteEdit::new(0..4, "aa".to_owned()),
            ByteEdit::new(5..9, "bb".to_owned()),
            ByteEdit::new(10..14, "cc".to_owned()),
        ];
        state.apply_changes(&edits).expect("valid batch");
        assert_eq!(&**state.snapshot().doc_text(), "aa bb cc");
    }

    /// Round-trip pin: building a document from an exact `\n\n` run
    /// at the start of the buffer must keep the buffer byte-equal
    /// after a snapshot rebuild. Earlier paragraph-boundary regressions
    /// silently dropped leading newlines.
    #[test]
    fn leading_blank_paragraph_round_trips() {
        let s = "\n\n本文";
        let state = doc(s);
        assert_eq!(&**state.snapshot().doc_text(), s);
    }

    /// Pin the boundary policy for `paragraph_at`: a byte sitting at
    /// `paragraph_starts[i + 1]` (the inclusive start of the right
    /// paragraph) resolves to **the right paragraph**, matching
    /// `paragraph_byte_ranges`'s half-open ranges. Only the doc-end
    /// byte (`total_bytes`) resolves to the last paragraph.
    ///
    /// An earlier docstring claimed the boundary belongs to the LEFT
    /// paragraph; the actual behaviour was always RIGHT. This test
    /// pins the RIGHT behaviour explicitly so a future doc-following
    /// refactor cannot silently swap policies.
    #[test]
    fn paragraph_at_boundary_byte_belongs_to_right_paragraph() {
        let state = doc("段落1\n\n段落2");
        let snap = state.snapshot();
        // The two paragraphs in `paragraph_byte_ranges`'s split are:
        //   p0 = bytes 0..("段落1\n".len())   = 0..10
        //   p1 = bytes ("段落1\n".len())..end = 10..(text.len())
        // So byte 10 (the 2nd `\n`) belongs to p1.
        let boundary = "段落1\n".len();
        assert_eq!(snap.paragraph_at(boundary), Some(1));
        // Byte boundary - 1 (the 1st `\n`) belongs to p0.
        assert_eq!(snap.paragraph_at(boundary - 1), Some(0));
        // The doc-end byte resolves to the LAST paragraph (no
        // rightward paragraph to take ownership).
        let total = snap.doc_text().len();
        assert_eq!(snap.paragraph_at(total), Some(1));
    }

    /// `paragraph_at` past EOF returns the last paragraph index — a
    /// graceful clamp matching most LSP clients' "out of range
    /// position resolves to EOF" behaviour.
    #[test]
    fn paragraph_at_past_eof_clamps_to_last_paragraph() {
        let state = doc("a\n\nb");
        let snap = state.snapshot();
        let total = snap.doc_text().len();
        assert_eq!(
            snap.paragraph_at(total + 1000),
            Some(snap.paragraphs.len() - 1)
        );
    }

    /// Growing a single paragraph past [`MAX_PARAGRAPH_BYTES`] via an
    /// in-paragraph insert must trigger `maybe_resegment_around` and hard-
    /// split it, so one runaway never-blank-line paragraph can't balloon
    /// unboundedly (the tree-sitter reparse cost is paragraph-local).
    #[test]
    fn in_paragraph_growth_past_cap_resegments() {
        // Just under the cap → exactly one paragraph (no blank lines).
        let base = "a".repeat(MAX_PARAGRAPH_BYTES - 100);
        let state = doc(&base);
        assert_eq!(
            state.snapshot().paragraphs.len(),
            1,
            "an under-cap run is a single paragraph",
        );
        // Push it over the cap. With no blank line to split on, the hard
        // cap in `paragraph_byte_ranges` is the only thing that can re-split.
        let pos = base.len();
        state
            .apply_changes(&[ByteEdit::new(pos..pos, "b".repeat(300))])
            .expect("in-bounds insert applies");
        assert!(
            state.snapshot().paragraphs.len() >= 2,
            "over-cap paragraph must re-split, got {}",
            state.snapshot().paragraphs.len(),
        );
    }

    #[test]
    fn doc_snapshot_debug_is_concise() {
        let state = doc("a\n\nb");
        let rendered = format!("{:?}", state.snapshot());
        assert!(rendered.contains("DocSnapshot"), "{rendered}");
        assert!(rendered.contains("version"), "{rendered}");
    }

    // =================================================================
    // Mutation-kill reinforcements
    // =================================================================

    /// A `tracing::Subscriber` that counts `WARN`-level events on the
    /// current thread. `enabled` returns `true` so its
    /// `register_callsite` reports `Interest::always()` and every event
    /// reaches `event`.
    use std::sync::atomic::AtomicUsize;
    use tracing::span::{Attributes, Id, Record};
    use tracing::subscriber::with_default;

    struct WarnCounter {
        warns: Arc<AtomicUsize>,
    }

    impl tracing::Subscriber for WarnCounter {
        fn enabled(&self, _metadata: &tracing::Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _span: &Attributes<'_>) -> Id {
            Id::from_u64(1)
        }
        fn record(&self, _span: &Id, _values: &Record<'_>) {}
        fn record_follows_from(&self, _span: &Id, _follows: &Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            if *event.metadata().level() == tracing::Level::WARN {
                self.warns.fetch_add(1, Ordering::SeqCst);
            }
        }
        fn enter(&self, _span: &Id) {}
        fn exit(&self, _span: &Id) {}
    }

    /// Run `f` with a thread-local `WARN`-counting subscriber and return
    /// how many `WARN` events it emitted.
    fn count_warns(f: impl FnOnce()) -> usize {
        let warns = Arc::new(AtomicUsize::new(0));
        let subscriber = WarnCounter {
            warns: Arc::clone(&warns),
        };
        with_default(subscriber, f);
        warns.load(Ordering::SeqCst)
    }

    /// A `ReparseStats` whose only non-default field is `latency_us`.
    fn stats_with_latency(latency_us: u64) -> ReparseStats {
        ReparseStats {
            latency_us,
            ..ReparseStats::default()
        }
    }

    /// Pre-order tree-sitter node kinds.
    fn collect_tree_kinds(tree: &tree_sitter::Tree) -> Vec<String> {
        let mut out = Vec::new();
        let mut cursor = tree.walk();
        walk_tree_kinds(&mut cursor, &mut out);
        out
    }

    fn walk_tree_kinds(cursor: &mut tree_sitter::TreeCursor<'_>, out: &mut Vec<String>) {
        out.push(cursor.node().kind().to_owned());
        if cursor.goto_first_child() {
            loop {
                walk_tree_kinds(cursor, out);
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
            let popped = cursor.goto_parent();
            debug_assert!(popped, "every goto_first_child must have a matching parent");
        }
    }

    /// `DocBuffer`'s `Debug` names the struct and its `paragraphs` field.
    /// Kills the `fmt -> Ok(Default::default())` mutant (which renders
    /// an empty string).
    #[test]
    fn doc_buffer_debug_names_struct_and_fields() {
        let buffer = DocBuffer::new("hello".to_owned());
        let rendered = format!("{buffer:?}");
        assert!(rendered.contains("DocBuffer"), "{rendered}");
        assert!(rendered.contains("paragraphs"), "{rendered}");
    }

    /// `OpenDocument`'s `Debug` names the struct and its `edit_version`
    /// field. Kills the `fmt -> Ok(Default::default())` mutant.
    #[test]
    fn open_document_debug_names_struct_and_fields() {
        let state = doc("a");
        let rendered = format!("{state:?}");
        assert!(rendered.contains("OpenDocument"), "{rendered}");
        assert!(rendered.contains("edit_version"), "{rendered}");
    }

    /// `validate_edits` rejects an edit where exactly ONE endpoint sits
    /// off a UTF-8 char boundary. "あ" is 3 bytes; start=1 is inside the
    /// codepoint, end=3 is the document end. The `||`→`&&` mutant would
    /// require BOTH endpoints off-boundary and wrongly accept this edit.
    #[test]
    fn validate_edits_rejects_single_off_boundary_endpoint() {
        let buffer = DocBuffer::new("あ".to_owned());
        let edits = [ByteEdit::new(1..3, String::new())];
        let err = buffer
            .validate_edits(&edits)
            .expect_err("an off-boundary start must be rejected");
        assert!(
            matches!(err, EditError::NonCharBoundary { start: 1, end: 3 }),
            "{err:?}",
        );
    }

    /// A doc byte strictly past `total_bytes` is out of bounds and
    /// `is_char_boundary` must short-circuit to `false` at the
    /// `doc_byte > total` guard. The `>`→`==` mutant lets an
    /// out-of-bounds byte fall through into `chunk_at_byte`, which
    /// panics on the over-length local offset.
    #[test]
    fn is_char_boundary_past_total_is_false() {
        let buffer = DocBuffer::new("あ".to_owned());
        // total_bytes == 3; byte 8 is well past the end.
        assert!(!buffer.is_char_boundary(8));
    }

    /// A real char boundary deep inside a multi-chunk paragraph rope must
    /// report `true`. `is_char_boundary` maps the doc byte to an
    /// in-chunk offset via `local - chunk_byte_idx`; the `-`→`+` mutant
    /// overshoots the chunk length, so `str::is_char_boundary` returns
    /// `false` for a byte that is actually a boundary.
    #[test]
    fn is_char_boundary_true_deep_in_a_multi_chunk_paragraph() {
        // 20 000 ASCII bytes span many rope chunks (ropey caps a leaf at
        // ~1 KB), so byte 15 000 sits in a chunk with chunk_byte_idx > 0.
        let buffer = DocBuffer::new("a".repeat(20_000));
        assert!(buffer.is_char_boundary(15_000));
    }

    /// A doc byte at a paragraph boundary (== the left paragraph's byte
    /// length) resolves to the RIGHT paragraph at local offset 0. The
    /// `<`→`<=` mutant instead keeps it in the left paragraph at its
    /// end offset.
    #[test]
    fn locate_byte_reports_boundary_to_right_paragraph() {
        // "aa\n\nbb" → p0 = "aa\n" (3 bytes), p1 = "\nbb".
        let buffer = DocBuffer::new("aa\n\nbb".to_owned());
        assert_eq!(buffer.paragraphs.len(), 2);
        let len0 = buffer.paragraphs[0].text.len_bytes();
        assert!(len0 > 0);
        assert_eq!(buffer.locate_byte(len0), (1, 0));
    }

    /// Ruby-at-paragraph-start invariant: a within-paragraph insert at
    /// `local.start == 0` must be parsed incrementally into an explicit-ruby
    /// node.
    ///
    /// NOTE: this does NOT kill the `+`→`*` mutant on the new-end byte
    /// (`apply_within_paragraph`'s `local.start + new_text.len()`), which
    /// hands tree-sitter a `0 * 27 == 0` "nothing changed" hint. That mutant
    /// is a frozen equivalent (mutants-baseline.json): `ParagraphBuffer::
    /// apply_edit` re-parses against the authoritative post-edit rope
    /// (`chunk_callback(&self.text)`), and since the new text is longer than
    /// the old tree's coverage, tree-sitter re-lexes from position 0 and
    /// still yields the ruby — the corrupted hint changes only reuse
    /// efficiency, never the resulting tree. This test confirms exactly that
    /// (the ruby parses regardless of the hint), so it stands as a regression
    /// guard, not a mutant killer.
    #[test]
    fn within_paragraph_insert_reports_correct_new_end_to_tree_sitter() {
        use tree_sitter_aozora::kind::EXPLICIT_RUBY;
        let state = doc("本文。");
        state
            .apply_changes(&[ByteEdit::new(0..0, "｜青空《あおぞら》".to_owned())])
            .expect("valid in-paragraph insert");
        let snap = state.snapshot();
        assert_eq!(snap.paragraphs.len(), 1, "{snap:?}");
        let tree = snap.paragraphs[0].tree.as_ref().expect("paragraph tree");
        let kinds = collect_tree_kinds(tree);
        assert!(
            kinds.iter().any(|k| k.as_str() == EXPLICIT_RUBY),
            "the inserted ruby must be parsed incrementally: {kinds:?}",
        );
    }

    /// A cross-paragraph edit that replaces a boundary-spanning range
    /// with non-empty text must KEEP that text in the merged rope. The
    /// `delete !` mutant (`if new_text.is_empty()`) appends the
    /// replacement only when it is empty, i.e. never — dropping it.
    #[test]
    fn cross_paragraph_edit_keeps_replacement_text() {
        // "aaa\n\nbbb" → p0 = "aaa\n", p1 = "\nbbb". Range 1..7 starts in
        // p0 and ends in p1, so `apply_across_paragraphs` runs.
        let state = doc("aaa\n\nbbb");
        state
            .apply_changes(&[ByteEdit::new(1..7, "X".to_owned())])
            .expect("valid cross-paragraph edit");
        assert_eq!(&**state.snapshot().doc_text(), "aXb");
    }

    /// `spawn_snapshot_rebuild`'s in-task guard skips the rebuild only
    /// when the installed snapshot is already at/ahead of the target
    /// version (`>=`). The `>=`→`<` mutant inverts this and skips the
    /// rebuild precisely when the snapshot is BEHIND the target, so the
    /// stale version-0 snapshot survives an edit.
    #[test]
    fn spawn_snapshot_rebuild_installs_snapshot_when_behind_target() {
        use tokio::runtime::Builder;
        let rt = Builder::new_multi_thread()
            .worker_threads(1)
            .max_blocking_threads(1)
            .build()
            .expect("build tokio runtime");
        let state = doc("hello");
        let version = rt.block_on(async {
            let v = state
                .apply_changes(&[ByteEdit::new(5..5, " world".to_owned())])
                .expect("valid edit");
            // The rebuild was submitted to the single-thread blocking pool
            // during `apply_changes`; a second blocking task queued behind
            // it (FIFO, one thread) can only complete once the rebuild has
            // run — a deterministic barrier with no timing assumptions.
            spawn_blocking(|| {}).await.expect("barrier task");
            v
        });
        assert_eq!(state.snapshot().version, version);
        assert_eq!(&**state.snapshot().doc_text(), "hello world");
    }

    /// A parse latency strictly above the slow-path threshold warns
    /// exactly once. Kills `>`→`==` and `>`→`<` in `record_parse_stats`
    /// (both stop warning above the threshold).
    #[test]
    fn record_parse_stats_warns_strictly_above_threshold() {
        let state = doc("hello");
        let warns = count_warns(|| {
            state.record_parse_stats(stats_with_latency(150_000));
        });
        assert_eq!(warns, 1);
    }

    /// A latency exactly AT the threshold must not warn (strict `>`).
    /// Kills `>`→`>=` and `>`→`==` (both warn at equality). The default
    /// threshold is `100_000` µs.
    #[test]
    fn record_parse_stats_silent_at_exact_threshold() {
        let state = doc("hello");
        let warns = count_warns(|| {
            state.record_parse_stats(stats_with_latency(100_000));
        });
        assert_eq!(warns, 0);
    }

    /// With `AOZORA_LSP_SLOW_PARSE_US` unset, the slow-parse threshold is
    /// the 100 ms default. Pins the value against a body replaced by the
    /// constants `0` or `1`.
    #[test]
    fn slow_parse_threshold_defaults_to_100_000_us() {
        assert_eq!(slow_parse_threshold_us(), 100_000);
    }
}
