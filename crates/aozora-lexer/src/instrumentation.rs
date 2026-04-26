//! Opt-in phase 3 sub-system instrumentation.
//!
//! Compiled in only when the `phase3-instrument` feature is on. When
//! enabled, every recogniser entry point inside [`crate::phase3_classify`]
//! constructs a [`SubsystemGuard`] on entry; on `Drop` the guard pushes
//! its elapsed nanoseconds into a thread-local [`TimingTable`] keyed by
//! [`Subsystem`].
//!
//! The `aozora-bench` `phase3_subsystems` example reads the table after
//! each document is processed via [`TimingTable::snapshot`] and
//! [`TimingTable::reset`].
//!
//! Default builds compile this module out entirely. The only public
//! API surface added by the feature is this module and its re-exports
//! (no changes to existing types).

#![cfg(feature = "phase3-instrument")]
#![allow(
    clippy::missing_panics_doc,
    reason = "instrumentation panics only on RefCell re-entry, an internal contract"
)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Instant;

/// Phase 3 recogniser subsystem identifier.
///
/// One variant per major recogniser entry point. The set is closed —
/// adding a new variant is an explicit decision because each variant
/// implies a corresponding `SubsystemGuard::new(...)` call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Subsystem {
    // -------------------------------------------------------------
    // Recogniser leaves (do not nest with each other)
    // -------------------------------------------------------------
    /// `recognize_ruby` — paired-quote ruby spans `｜base《reading》`.
    Ruby,
    /// `recognize_annotation` — bracket-hash annotation `［＃...］`.
    Annotation,
    /// `recognize_gaiji` — gaiji marker `※［＃...］`.
    Gaiji,
    /// `build_content_from_body` — segment construction + interning
    /// for ruby readings, bouten targets, warichu bodies, etc.
    BuildContent,
    /// `body_dispatcher` Aho-Corasick pattern lookup inside
    /// `classify_annotation_body` — covers the ~30 fixed body
    /// keywords (kaeriten, indent, warichu open/close, etc.).
    BodyDispatcher,

    // -------------------------------------------------------------
    // Framework / dispatch (may nest with leaves; their total double-
    // counts the leaf time; subtract leaves to get pure framework cost)
    // -------------------------------------------------------------
    /// Outer `ClassifyStream::next()` body — wraps every per-event
    /// dispatch including all recogniser calls. Total time minus
    /// the recogniser leaves' sum gives the pure dispatch overhead.
    IterDispatch,
    /// `forward_target_is_preceded` — per-call AC index lookup or
    /// substring scan over the source preceding the current open.
    ForwardTargetCheck,
    /// `install_forward_target_index_from_source` — one-time-per-
    /// document pre-pass that builds the source-byte AC quote-body
    /// index. Called once per `classify()` entry.
    ForwardIndexInstall,
    /// `append_to_frame` — per-event frame buffer push + nested
    /// pair-stack maintenance. Called on every event consumed
    /// inside an open frame.
    FrameAppend,
    /// `recognize_and_emit` — runs when the outermost open closes;
    /// dispatches into the per-PairKind recogniser. Wraps recogniser
    /// leaves so its time INCLUDES Ruby/Annotation/Gaiji/etc. — the
    /// dispatch overhead = recognize_and_emit - leaf_total.
    RecognizeAndEmit,
    /// `replay_unrecognised_body` — frames whose recogniser declined
    /// (or whose kind has no recogniser at top level) walk the body
    /// events back as Plain spans. One frame may yield many spans.
    ReplayBody,
    /// `open_frame` — initial frame allocation when an outer PairOpen
    /// appears at top level. Allocates the body buffer SmallVec.
    OpenFrame,
    /// `flush_plain_up_to` — emit any pending plain run on outer
    /// boundary (newline / Aozora yield). Cheap, but called per
    /// trigger event.
    FlushPlain,
    /// `try_ruby_emit` — wraps `recognize_ruby`. Pre-work: scan
    /// preceding source text for `｜` (potential O(N) per ruby on
    /// large pending plain runs), build synthetic event vec.
    TryRubyEmit,
    /// `try_bracket_emit` — wraps `recognize_annotation`. Pre-work:
    /// frame setup + sentinel padding decisions.
    TryBracketEmit,
    /// Outer next() loop body INCLUDING all sub-callees, MINUS the
    /// upstream `events.next()` call. Use to isolate "loop body work"
    /// from "PairStream pulling work".
    LoopBody,
    /// `events.next()` upstream pull. Should be O(1) for a Vec iterator
    /// or O(constant) for PairStream. If this is large, the issue is
    /// in the upstream iterator, not in classify itself.
    EventsNext,
}

impl Subsystem {
    /// Stable display label used in probe output.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Subsystem::Ruby => "recognize_ruby",
            Subsystem::Annotation => "recognize_annotation",
            Subsystem::Gaiji => "recognize_gaiji",
            Subsystem::BuildContent => "build_content_from_body",
            Subsystem::BodyDispatcher => "body_dispatcher",
            Subsystem::IterDispatch => "iter_dispatch (outer)",
            Subsystem::ForwardTargetCheck => "forward_target_is_preceded",
            Subsystem::ForwardIndexInstall => "install_forward_target_index",
            Subsystem::FrameAppend => "append_to_frame",
            Subsystem::RecognizeAndEmit => "recognize_and_emit",
            Subsystem::ReplayBody => "replay_unrecognised_body",
            Subsystem::OpenFrame => "open_frame",
            Subsystem::FlushPlain => "flush_plain_up_to",
            Subsystem::TryRubyEmit => "try_ruby_emit",
            Subsystem::TryBracketEmit => "try_bracket_emit",
            Subsystem::LoopBody => "loop_body (outer-events.next())",
            Subsystem::EventsNext => "events.next() (upstream pull)",
        }
    }

    /// Iteration order matching the human-friendly source-of-data order.
    /// Leaves first, then framework.
    #[must_use]
    pub fn ordered() -> [Subsystem; 17] {
        [
            Subsystem::Ruby,
            Subsystem::Annotation,
            Subsystem::Gaiji,
            Subsystem::BuildContent,
            Subsystem::BodyDispatcher,
            Subsystem::IterDispatch,
            Subsystem::EventsNext,
            Subsystem::LoopBody,
            Subsystem::RecognizeAndEmit,
            Subsystem::TryRubyEmit,
            Subsystem::TryBracketEmit,
            Subsystem::ReplayBody,
            Subsystem::OpenFrame,
            Subsystem::FlushPlain,
            Subsystem::FrameAppend,
            Subsystem::ForwardTargetCheck,
            Subsystem::ForwardIndexInstall,
        ]
    }

    /// Whether this subsystem is a "leaf" (does not call other
    /// instrumented entries) — used by probes to compute the
    /// pure-dispatch overhead.
    #[must_use]
    pub fn is_leaf(self) -> bool {
        matches!(
            self,
            Subsystem::Ruby
                | Subsystem::Annotation
                | Subsystem::Gaiji
                | Subsystem::BuildContent
                | Subsystem::BodyDispatcher
        )
    }
}

/// RAII guard that records elapsed time on Drop.
///
/// Construct one at the entry of a recogniser; the guard records
/// `Instant::now()` and on Drop logs `elapsed.as_nanos()` into the
/// thread-local [`TimingTable`] under its [`Subsystem`] key.
pub struct SubsystemGuard {
    kind: Subsystem,
    started: Instant,
}

impl SubsystemGuard {
    /// Start a new guard for `kind`. Records `Instant::now()` on
    /// construction; the elapsed duration is logged when the guard is
    /// dropped.
    #[must_use]
    pub fn new(kind: Subsystem) -> Self {
        Self {
            kind,
            started: Instant::now(),
        }
    }
}

impl Drop for SubsystemGuard {
    fn drop(&mut self) {
        let elapsed_ns = self.started.elapsed().as_nanos() as u64;
        TIMING_TABLE.with(|t| {
            let mut table = t.borrow_mut();
            *table.counts.entry(self.kind).or_insert(0) += 1;
            *table.total_ns.entry(self.kind).or_insert(0) += elapsed_ns;
        });
    }
}

/// Per-thread accumulation of `(call count, total ns)` per [`Subsystem`].
///
/// Created lazily in the `thread_local!` block; reset between corpus
/// docs by the `phase3_subsystems` probe via [`TimingTable::reset`].
#[derive(Debug, Clone, Default)]
pub struct TimingTable {
    /// Number of recogniser entries observed per subsystem.
    pub counts: HashMap<Subsystem, u64>,
    /// Total elapsed nanoseconds per subsystem.
    pub total_ns: HashMap<Subsystem, u64>,
}

impl TimingTable {
    /// Snapshot the current thread's table (cloned).
    ///
    /// Cheap: at most a 3-entry HashMap clone.
    #[must_use]
    pub fn snapshot() -> TimingTable {
        TIMING_TABLE.with(|t| t.borrow().clone())
    }

    /// Reset the current thread's table to all-zero.
    pub fn reset() {
        TIMING_TABLE.with(|t| {
            let mut table = t.borrow_mut();
            table.counts.clear();
            table.total_ns.clear();
        });
    }
}

thread_local! {
    static TIMING_TABLE: RefCell<TimingTable> = RefCell::new(TimingTable::default());
    static YIELD_COUNTERS: RefCell<YieldCounters> = RefCell::new(YieldCounters::default());
    static PENDING_SIZE_HIST: RefCell<PendingSizeHistogram> = RefCell::new(PendingSizeHistogram::default());
}

/// Histogram of `pending_outputs.len()` measured at every
/// `pending_outputs_pop_front()` call. Bucket boundaries chosen to
/// catch the "tiny vs large" distinction relevant to the N2 outlier
/// investigation.
#[derive(Debug, Clone, Copy, Default)]
pub struct PendingSizeHistogram {
    pub size_0: u64,
    pub size_1: u64,
    pub size_2_4: u64,
    pub size_5_15: u64,
    pub size_16_63: u64,
    pub size_64_255: u64,
    pub size_256_plus: u64,
    pub max_seen: u64,
}

pub fn record_pending_size(len: u64) {
    PENDING_SIZE_HIST.with(|h| {
        let mut hist = h.borrow_mut();
        match len {
            0 => hist.size_0 += 1,
            1 => hist.size_1 += 1,
            2..=4 => hist.size_2_4 += 1,
            5..=15 => hist.size_5_15 += 1,
            16..=63 => hist.size_16_63 += 1,
            64..=255 => hist.size_64_255 += 1,
            _ => hist.size_256_plus += 1,
        }
        if len > hist.max_seen {
            hist.max_seen = len;
        }
    });
}

impl PendingSizeHistogram {
    #[must_use]
    pub fn snapshot() -> PendingSizeHistogram {
        PENDING_SIZE_HIST.with(|h| *h.borrow())
    }
    pub fn reset() {
        PENDING_SIZE_HIST.with(|h| *h.borrow_mut() = PendingSizeHistogram::default());
    }
    pub fn total(self) -> u64 {
        self.size_0 + self.size_1 + self.size_2_4 + self.size_5_15
            + self.size_16_63 + self.size_64_255 + self.size_256_plus
    }
}

thread_local! {
    static REPLAY_SIZES: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
}

pub fn record_replay_body_size(size: u64) {
    REPLAY_SIZES.with(|v| v.borrow_mut().push(size));
}

pub fn snapshot_replay_sizes() -> Vec<u64> {
    REPLAY_SIZES.with(|v| v.borrow().clone())
}

pub fn reset_replay_sizes() {
    REPLAY_SIZES.with(|v| v.borrow_mut().clear());
}

/// Per-yield-kind histogram, populated at every span yield from
/// `ClassifyStream::next()`. Useful for spotting "this doc yields
/// 10× more Plain spans than expected" patterns. Each variant maps
/// to a `SpanKind` arm.
#[derive(Debug, Clone, Copy, Default)]
pub struct YieldCounters {
    pub plain: u64,
    pub newline: u64,
    pub aozora: u64,
    pub block_open: u64,
    pub block_close: u64,
}

/// Yield-kind tags used by `record_yield` to bump the appropriate
/// counter without forcing the lexer crate to depend on `SpanKind`'s
/// concrete variant set.
#[derive(Debug, Clone, Copy)]
pub enum YieldKind {
    Plain,
    Newline,
    Aozora,
    BlockOpen,
    BlockClose,
}

/// Bump the per-thread yield-kind counter. No-op when feature is OFF
/// (the function only exists under cfg).
pub fn record_yield(kind: YieldKind) {
    YIELD_COUNTERS.with(|c| {
        let mut counters = c.borrow_mut();
        match kind {
            YieldKind::Plain => counters.plain += 1,
            YieldKind::Newline => counters.newline += 1,
            YieldKind::Aozora => counters.aozora += 1,
            YieldKind::BlockOpen => counters.block_open += 1,
            YieldKind::BlockClose => counters.block_close += 1,
        }
    });
}

impl YieldCounters {
    /// Snapshot the current thread's yield counters.
    #[must_use]
    pub fn snapshot() -> YieldCounters {
        YIELD_COUNTERS.with(|c| *c.borrow())
    }

    /// Reset the current thread's yield counters to zero.
    pub fn reset() {
        YIELD_COUNTERS.with(|c| *c.borrow_mut() = YieldCounters::default());
    }

    /// Total yield count.
    #[must_use]
    pub fn total(self) -> u64 {
        self.plain + self.newline + self.aozora + self.block_open + self.block_close
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn guard_records_elapsed_on_drop() {
        TimingTable::reset();
        {
            let _g = SubsystemGuard::new(Subsystem::Ruby);
            thread::sleep(Duration::from_micros(10));
        }
        let snap = TimingTable::snapshot();
        assert_eq!(snap.counts.get(&Subsystem::Ruby).copied(), Some(1));
        assert!(snap.total_ns.get(&Subsystem::Ruby).copied().unwrap_or(0) > 0);
        TimingTable::reset();
    }

    #[test]
    fn snapshot_clone_does_not_share_state() {
        TimingTable::reset();
        {
            let _g = SubsystemGuard::new(Subsystem::Annotation);
        }
        let snap_a = TimingTable::snapshot();
        TimingTable::reset();
        let snap_b = TimingTable::snapshot();
        assert_eq!(snap_a.counts.get(&Subsystem::Annotation).copied(), Some(1));
        assert_eq!(snap_b.counts.get(&Subsystem::Annotation).copied(), None);
    }
}
