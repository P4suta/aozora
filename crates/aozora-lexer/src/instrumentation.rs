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
        }
    }

    /// Iteration order matching the human-friendly source-of-data order.
    /// Leaves first, then framework.
    #[must_use]
    pub fn ordered() -> [Subsystem; 9] {
        [
            Subsystem::Ruby,
            Subsystem::Annotation,
            Subsystem::Gaiji,
            Subsystem::BuildContent,
            Subsystem::BodyDispatcher,
            Subsystem::IterDispatch,
            Subsystem::ForwardTargetCheck,
            Subsystem::ForwardIndexInstall,
            Subsystem::FrameAppend,
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
