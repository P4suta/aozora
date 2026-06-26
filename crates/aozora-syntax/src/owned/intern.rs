//! Owned, lifetime-free mirror of [`crate::borrowed::Interner`].
//!
//! Where the borrowed interner hands back a stable `&'a str` pointing into a
//! bump [`Arena`](crate::borrowed::Arena), the owned interner owns all bytes
//! in a single `String` and hands back a [`StrId`] index. It reproduces the
//! borrowed interner's two observable contracts:
//!
//! - **Dedup**: byte-equal `intern` calls return the same handle.
//! - **[`InternStats`]**: the dedup-ratio counters used by the corpus-sweep
//!   report (`calls`, `cache_hits`, `table_hits`, `allocs`) are reproduced
//!   exactly; the borrowed open-addressing table's `probe_steps` / `resizes`
//!   have no faithful analogue under a `HashMap` backing (see the
//!   `TODO(#237)` on [`StrInterner`] below).
//!
//! Backing differences from the borrowed interner are deliberate and noted
//! per item; the *handle contract* (dedup + resolve) is the invariant.

use std::collections::HashMap;

// Reused as-is from the borrowed interner — NOT duplicated.
// borrowed-source: crates/aozora-syntax/src/borrowed/intern.rs::InternStats
pub use crate::borrowed::InternStats;

/// Byte length beyond which the borrowed interner bypasses its probe table.
/// Mirrored here only so `InternStats::long_bypass` keeps the same meaning;
/// unlike the borrowed table, the owned interner still dedups long strings.
// borrowed-source: crates/aozora-syntax/src/borrowed/intern.rs::INTERN_LENGTH_LIMIT
const INTERN_LENGTH_LIMIT: usize = 64;

/// Stable handle to an interned string inside a [`StrInterner`].
///
/// Owned replacement for the `&'a str` that
/// [`Interner::intern`](crate::borrowed::Interner::intern) returns: a `u32`
/// index into the interner's `spans`, resolvable via
/// [`StrInterner::resolve`]. Mapping rule `&'src str (interned) -> StrId(u32)`.
///
/// `Hash`/`Ord` are derived so a `StrId` can key the owned node store's
/// auxiliary maps and sort deterministically; both are zero-cost on a `u32`.
// borrowed-source: the `&'a str` value returned by borrowed Interner::intern
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StrId(pub u32);

/// Owned, lifetime-free string interner.
///
/// Mirror of [`crate::borrowed::Interner`]. Deduplicates byte-equal strings
/// and returns a stable [`StrId`], owning every unique string's bytes in a
/// single `String` (`buf`) plus a `(start, len)` span per id (`spans`).
///
/// Derives `Clone` (the owned output may be cached/cloned by the #237 segment
/// cache; the borrowed `Interner` cannot, as it borrows an `Arena`). It does
/// **not** derive `Copy` (owns heap storage) nor `PartialEq`/`Eq`: the reused
/// `stats: InternStats` field does not implement `PartialEq`, so deriving it
/// here would not compile, and structural equality of an interner is not a
/// meaningful operation. `Default` is the trivially-empty interner.
///
/// TODO(#237): the borrowed interner's `INTERN_LENGTH_LIMIT` table bypass and
/// 1-slot inline cache are mirrored for stat parity, but the open-addressing
/// `probe_steps` / `resizes` counters have no `HashMap` analogue and remain 0.
/// Swap `HashMap` for `rustc_hash::FxHashMap` if/when that dep is added.
// borrowed-source: crates/aozora-syntax/src/borrowed/intern.rs::Interner<'a>
#[derive(Debug, Clone, Default)]
pub struct StrInterner {
    /// Every unique string's bytes, concatenated in intern order.
    /// Owns what the borrowed interner's `Arena` held by reference.
    buf: String,
    /// `(start, len)` byte span into `buf` for each id; `spans[id.0 as usize]`
    /// locates `StrId(id)`. Indexed by stable id, not by probe position.
    spans: Vec<(u32, u32)>,
    /// Build-time dedup map: byte content -> existing id. Reproduces the
    /// borrowed open-addressing table lookup that collapses byte-equal interns.
    dedup: HashMap<String, StrId>,
    /// Inline cache of the last interned id, mirroring the borrowed
    /// `last: Option<&'a str>` short-circuit so long identical runs count as
    /// `cache_hits`.
    last: Option<StrId>,
    /// Diagnostic counters; reused type, counted to match the borrowed
    /// interner's dedup-ratio reporting.
    pub stats: InternStats,
}

impl StrInterner {
    /// Empty interner. Owned analogue of `Interner::new_in`, minus the arena.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Intern `s`, returning a stable [`StrId`]. Byte-equal calls return the
    /// same id (dedup), reproducing `Interner::intern`'s pointer contract and
    /// its `InternStats` accounting.
    ///
    /// # Panics
    ///
    /// Panics if the interner's backing buffer would exceed `u32::MAX` bytes,
    /// a single interned string exceeds `u32::MAX` bytes, or the unique-string
    /// count exceeds `u32::MAX` — none reachable for any realistic document.
    pub fn intern(&mut self, s: &str) -> StrId {
        self.stats.calls += 1;

        // Inline cache: identical consecutive interns short-circuit.
        if let Some(id) = self.last
            && self.resolve(id) == s
        {
            self.stats.cache_hits += 1;
            return id;
        }

        // Dedup table lookup: byte-equal content reuses the existing id.
        if let Some(&id) = self.dedup.get(s) {
            self.stats.table_hits += 1;
            self.last = Some(id);
            return id;
        }

        // Fresh allocation: append bytes, record span, register id.
        let start =
            u32::try_from(self.buf.len()).expect("owned interner buffer exceeds u32 byte range");
        let len = u32::try_from(s.len()).expect("interned string exceeds u32 byte length");
        let id = StrId(
            u32::try_from(self.spans.len())
                .expect("owned interner unique-string count exceeds u32"),
        );
        self.buf.push_str(s);
        self.spans.push((start, len));
        self.dedup.insert(s.to_owned(), id);
        self.stats.allocs += 1;
        if s.len() > INTERN_LENGTH_LIMIT {
            // Stat parity with the borrowed table bypass; the owned interner
            // still dedups long strings (divergence noted on the struct).
            self.stats.long_bypass += 1;
        }
        self.last = Some(id);
        id
    }

    /// Resolve a [`StrId`] back to its interned bytes. Owned analogue of
    /// dereferencing the borrowed interner's returned `&'a str`.
    ///
    /// # Panics
    ///
    /// Panics if `id` was not produced by this interner.
    #[must_use]
    pub fn resolve(&self, id: StrId) -> &str {
        let (start, len) = self.spans[id.0 as usize];
        &self.buf[start as usize..start as usize + len as usize]
    }

    /// Number of unique strings held. Owned analogue of
    /// `Interner::unique_strings`.
    #[must_use]
    pub fn len(&self) -> usize {
        self.spans.len()
    }

    /// Whether the interner holds no strings.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // UNIT TEST 2 (interner cluster).
    #[test]
    fn intern_dedups_and_resolves_round_trip() {
        let mut i = StrInterner::new();

        // Same content -> same id.
        let a1 = i.intern("あ");
        let a2 = i.intern("あ");
        assert_eq!(a1, a2, "byte-equal intern must return the same id");

        // Resolve round-trips bytes exactly.
        assert_eq!(i.resolve(a1), "あ", "resolve must round-trip the bytes");

        // Distinct content -> distinct ids.
        let b = i.intern("い");
        assert_ne!(a1, b, "distinct content must yield distinct ids");
        assert_eq!(i.resolve(b), "い", "resolve must round-trip the bytes");

        // One unique string per distinct content.
        assert_eq!(i.len(), 2, "two distinct strings interned");
    }

    #[test]
    fn intern_reproduces_dedup_ratio_counters() {
        let mut i = StrInterner::new();
        let readings = ["の", "に", "を", "で", "が"];
        for _ in 0..200 {
            for r in readings {
                i.intern(r);
            }
        }
        assert_eq!(i.len(), 5, "five unique readings");
        assert_eq!(i.stats.calls, 1000, "every intern call counted");
        assert_eq!(i.stats.allocs, 5, "five fresh allocations");
        let reuses = i.stats.cache_hits + i.stats.table_hits;
        assert_eq!(reuses, 995, "remaining calls served from cache or table");
    }
}
