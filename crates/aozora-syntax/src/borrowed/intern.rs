//! Arena-backed string interner.
//!
//! Open-addressing hash table over `&'a str` slots whose payloads
//! point into a paired [`Arena`]. The first call to [`Interner::intern`]
//! with a given byte content allocates the string in the arena and
//! records the resulting `&'a str` in the table; every subsequent
//! call with byte-equal content reuses that pointer in O(1) amortised
//! time, with no further allocation.
//!
//! ## Why an interner here
//!
//! Aozora corpora are dense in **short, frequently repeated strings**
//! — single-mora ruby readings (`の`, `に`, `を`, `で`, `が`), bouten
//! kind keywords, container kind labels, kaeriten marks. A naive
//! `arena.alloc_str(s)` for every visit copies the same bytes into
//! the arena dozens to hundreds of times per document. Interning
//! collapses those duplicates into a single allocation; the table's
//! per-string overhead is one `&str` slot (16 bytes) plus a fraction
//! of probe load, which is more than recovered by the eliminated
//! duplicate-byte allocations on any document with non-trivial
//! repetition.
//!
//! ## Algorithm
//!
//! - Open addressing with linear probing.
//! - Power-of-two capacity (initial 256) so slot index is a single
//!   bitmask, no `%` modulo.
//! - Resize at 7/8 load factor; growth doubles capacity and rebuilds
//!   the table via fresh probing.
//! - **FxHash-style multiply-then-xor** mix on the byte stream:
//!   one `wrapping_mul` + one xor-shift per byte. Fast on short
//!   strings (< 32 bytes — the dominant length class for Aozora ruby
//!   readings); no per-call state allocation that std `SipHash` would
//!   incur.
//! - **Inline cache** for the most recent intern result. Long runs
//!   of identical interns (e.g. 100 consecutive `《の》` ruby readings
//!   in a poem) skip both the hash and the probe.
//! - **Length threshold**: strings longer than 64 bytes bypass the
//!   table entirely and go straight to `arena.alloc_str`. The hash +
//!   probe cost is dominated by the byte scan for long strings, and
//!   long strings are very rarely repeated in practice (raw
//!   annotations, gaiji descriptions). Skipping them avoids
//!   polluting the table with unique entries that will never be
//!   queried again.
//!
//! ## Memory model
//!
//! The interner does not own the underlying arena — it borrows it
//! immutably. This is sound because `Arena::alloc_str` takes
//! `&self` (bumpalo allows interior mutation through a shared
//! reference). A single arena can therefore back multiple interners
//! (e.g. one per worker in a parallel parse path), each with its own
//! probe table.

use bumpalo::collections::Vec as BumpVec;

use super::arena::Arena;

/// FxHash-style mix constant. The same constant rustc internally uses
/// for `FxHasher`; chosen for fast diffusion on short inputs.
const FX_PRIME: u64 = 0x517c_c1b7_2722_0a95;

/// Length threshold beyond which strings bypass the intern table.
/// Picked from corpus profile: ruby readings, kaeriten marks, and
/// container kind labels are all under 32 bytes; raw annotations and
/// gaiji descriptions can exceed 64 bytes and almost never repeat.
const INTERN_LENGTH_LIMIT: usize = 64;

/// Initial table capacity. Power of two so probe-index = `hash & mask`.
/// 256 covers the typical small-to-medium document without growth;
/// large (multi-MB) documents grow once or twice on the way to ~2048.
const INITIAL_CAPACITY: usize = 256;

/// Open-addressing intern table over arena-allocated strings.
#[derive(Debug)]
pub struct Interner<'a> {
    arena: &'a Arena,
    /// Slots: `None` = empty, `Some(&'a str)` = occupied.
    /// `BumpVec` keeps the table itself inside the arena, so the
    /// interner allocates exactly zero bytes outside the arena.
    table: BumpVec<'a, Option<&'a str>>,
    /// `capacity - 1`. `capacity` is always a power of two.
    mask: usize,
    /// Number of occupied slots.
    occupied: usize,
    /// Inline cache: last successfully-interned string. Long runs of
    /// identical interns short-circuit on this single pointer compare.
    ///
    /// A 2-slot LRU cache (intended to catch Ruby's alternating
    /// `(base, reading, base, reading, …)` access pattern) was tried
    /// and reverted. Corpus dedup ratio stayed at p50 0.275 /
    /// mean 0.308 (identical to the 1-slot baseline), throughput
    /// moved within noise. The pattern that would benefit —
    /// consecutive rubies sharing a base or reading
    /// — is rarer than the design assumed; distinct rubies on
    /// distinct words dominate.
    last: Option<&'a str>,
    /// Diagnostic counters. Updated on every intern call. Useful for
    /// benchmarks and the corpus-sweep dedup-ratio report.
    pub stats: InternStats,
}

/// Diagnostic counters surfaced by [`Interner::stats`].
#[derive(Debug, Clone, Copy, Default)]
pub struct InternStats {
    /// Total `intern` calls (every entry into the API).
    pub calls: u64,
    /// Calls served from the inline cache.
    pub cache_hits: u64,
    /// Calls that landed on an existing table entry (no allocation).
    pub table_hits: u64,
    /// Calls that allocated a new entry into the arena.
    pub allocs: u64,
    /// Calls that bypassed the table because the string exceeded
    /// [`INTERN_LENGTH_LIMIT`] — counted as an alloc as well.
    pub long_bypass: u64,
    /// Total resize events the table performed.
    pub resizes: u64,
    /// Total probe steps walked across all `intern` calls. Divided by
    /// `calls - cache_hits` gives the average probe length, the
    /// canonical hash-table health metric.
    pub probe_steps: u64,
}

impl<'a> Interner<'a> {
    /// Empty interner backed by `arena`. Initial capacity is
    /// [`INITIAL_CAPACITY`].
    #[must_use]
    pub fn new_in(arena: &'a Arena) -> Self {
        Self::with_capacity_in(INITIAL_CAPACITY, arena)
    }

    /// Empty interner backed by `arena`, with the table pre-sized to
    /// at least `capacity_hint` slots (rounded up to a power of two).
    /// Use when the caller can estimate the unique-string count up
    /// front (e.g. the lex driver knows the registry size).
    #[must_use]
    pub fn with_capacity_in(capacity_hint: usize, arena: &'a Arena) -> Self {
        let cap = capacity_hint.next_power_of_two().max(8);
        let mut table = BumpVec::with_capacity_in(cap, arena.bump());
        table.resize(cap, None);
        Self {
            arena,
            table,
            mask: cap - 1,
            occupied: 0,
            last: None,
            stats: InternStats::default(),
        }
    }

    /// Intern `s` and return a stable `&'a str` pointer to the arena
    /// copy. Subsequent calls with byte-equal content return the same
    /// pointer.
    pub fn intern(&mut self, s: &str) -> &'a str {
        self.stats.calls += 1;

        // Inline cache: long identical-intern runs short-circuit on
        // a single pointer-content compare. Equality on `&str`
        // compares lengths first then bytes, which is fast for the
        // typical mismatch.
        if let Some(cached) = self.last
            && cached == s
        {
            self.stats.cache_hits += 1;
            return cached;
        }

        let bytes = s.as_bytes();

        // Length threshold bypass — long strings rarely repeat and
        // hashing them costs more than the alloc they would save.
        if bytes.len() > INTERN_LENGTH_LIMIT {
            self.stats.long_bypass += 1;
            self.stats.allocs += 1;
            let dst = self.arena.alloc_str(s);
            self.last = Some(dst);
            return dst;
        }

        let hash = fx_hash(bytes);

        // Grow before probe if load factor would exceed 7/8.
        // Power-of-two table size makes load-factor check a single
        // multiply + compare; no division.
        if self.occupied.saturating_mul(8) >= self.table.len().saturating_mul(7) {
            self.grow();
        }

        #[allow(
            clippy::cast_possible_truncation,
            reason = "low bits of u64 hash extracted as usize on purpose"
        )]
        let mut idx = (hash as usize) & self.mask;
        loop {
            self.stats.probe_steps += 1;
            match self.table[idx] {
                Some(existing) if existing == s => {
                    self.stats.table_hits += 1;
                    self.last = Some(existing);
                    return existing;
                }
                None => {
                    let dst = self.arena.alloc_str(s);
                    self.table[idx] = Some(dst);
                    self.occupied += 1;
                    self.stats.allocs += 1;
                    self.last = Some(dst);
                    return dst;
                }
                Some(_) => idx = (idx + 1) & self.mask,
            }
        }
    }

    /// Number of unique strings currently held in the table.
    #[must_use]
    pub fn unique_strings(&self) -> usize {
        self.occupied
    }

    /// Current table capacity.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.table.len()
    }

    /// Average probe length per non-cache-hit lookup. Returns `0.0`
    /// when no probed lookups have occurred. Used by benchmarks to
    /// confirm the hash function and load-factor policy keep the
    /// table healthy.
    #[must_use]
    pub fn avg_probe_length(&self) -> f64 {
        let probed = self.stats.calls.saturating_sub(self.stats.cache_hits);
        if probed == 0 {
            0.0
        } else {
            #[allow(
                clippy::cast_precision_loss,
                reason = "probe count fits in f64 mantissa for any plausible workload"
            )]
            let avg = self.stats.probe_steps as f64 / probed as f64;
            avg
        }
    }

    /// Doubles capacity and rebuilds the table via fresh probing.
    fn grow(&mut self) {
        let new_cap = self.table.len().saturating_mul(2);
        let new_mask = new_cap - 1;
        let mut new_table: BumpVec<'a, Option<&'a str>> =
            BumpVec::with_capacity_in(new_cap, self.arena.bump());
        new_table.resize(new_cap, None);
        for s in self.table.iter().copied().flatten() {
            let h = fx_hash(s.as_bytes());
            #[allow(
                clippy::cast_possible_truncation,
                reason = "low bits of u64 hash extracted as usize on purpose"
            )]
            let mut idx = (h as usize) & new_mask;
            while new_table[idx].is_some() {
                idx = (idx + 1) & new_mask;
            }
            new_table[idx] = Some(s);
        }
        self.table = new_table;
        self.mask = new_mask;
        self.stats.resizes += 1;
    }
}

/// `wrapping_mul`-and-xor mix loop. Fast on short inputs (the dominant
/// case for Aozora ruby readings); avoids the per-call state setup
/// cost of std `SipHash`.
///
/// An 8-byte-chunk fast path with an xxHash-style avalanche was
/// tried and reverted. For the typical
/// 3-byte single-codepoint reading the avalanche's two extra
/// multiplications cost more than the per-byte loop saves; corpus
/// throughput moved within noise (-4 % to +2 % depending on band).
/// The byte loop fits in a few cycles on short inputs and is hard to
/// beat without a different hash family. Keeping the simple shape.
#[inline]
fn fx_hash(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0;
    for &b in bytes {
        h = h.rotate_left(5) ^ u64::from(b);
        h = h.wrapping_mul(FX_PRIME);
    }
    h
}

// Bump access for structures that need their own arena-backed storage
// (e.g. the [`Interner`]'s probe table) lives as an inherent method
// on Arena in `arena.rs`.

#[cfg(test)]
mod tests {
    use core::ptr;

    use super::*;

    fn arena() -> Arena {
        Arena::new()
    }

    #[test]
    fn empty_interner_has_no_unique_strings() {
        let a = arena();
        let i = Interner::new_in(&a);
        assert_eq!(i.unique_strings(), 0);
        assert_eq!(i.capacity(), INITIAL_CAPACITY);
    }

    #[test]
    fn intern_returns_stable_pointer_for_byte_equal_input() {
        let a = arena();
        let mut i = Interner::new_in(&a);
        let s1 = i.intern("hello");
        let s2 = i.intern("hello");
        assert!(
            ptr::eq(s1.as_ptr(), s2.as_ptr()),
            "byte-equal intern must return same arena pointer"
        );
        assert_eq!(i.unique_strings(), 1);
    }

    #[test]
    fn intern_returns_distinct_pointers_for_distinct_inputs() {
        let a = arena();
        let mut i = Interner::new_in(&a);
        let s1 = i.intern("hello");
        let s2 = i.intern("world");
        assert!(!ptr::eq(s1.as_ptr(), s2.as_ptr()));
        assert_eq!(i.unique_strings(), 2);
    }

    #[test]
    fn intern_handles_empty_string() {
        let a = arena();
        let mut i = Interner::new_in(&a);
        let s1 = i.intern("");
        let s2 = i.intern("");
        assert_eq!(s1, "");
        assert!(ptr::eq(s1.as_ptr(), s2.as_ptr()));
    }

    #[test]
    fn inline_cache_serves_repeated_calls_without_probe() {
        let a = arena();
        let mut i = Interner::new_in(&a);
        // First call probes; next 99 hit the inline cache.
        for _ in 0..100 {
            i.intern("repeated");
        }
        assert_eq!(i.unique_strings(), 1);
        assert!(i.stats.cache_hits >= 99);
    }

    #[test]
    fn long_strings_bypass_table_but_still_alloc() {
        let a = arena();
        let mut i = Interner::new_in(&a);
        // String beyond INTERN_LENGTH_LIMIT (64 bytes).
        let long: String = "x".repeat(128);
        let s = i.intern(&long);
        assert_eq!(s.len(), 128);
        // First long-string call took the bypass path (no cache hit
        // because the cache was empty). The bypass also primes the
        // inline cache so the second call short-circuits — cache + bypass
        // together cover both calls.
        let _ = i.intern(&long);
        assert_eq!(
            i.stats.long_bypass + i.stats.cache_hits,
            2,
            "every call must be served by either the cache or the bypass path"
        );
        assert_eq!(i.stats.long_bypass, 1, "only the first long call bypasses");
        assert_eq!(i.stats.cache_hits, 1, "second long call hits the cache");
        // No table slot consumed by long strings.
        assert_eq!(i.unique_strings(), 0);

        // A *different* long string forces a fresh bypass even if the
        // cache is primed (cache compares full content).
        let other: String = "y".repeat(128);
        let _ = i.intern(&other);
        assert_eq!(i.stats.long_bypass, 2);
    }

    #[test]
    fn many_unique_strings_trigger_resize() {
        let a = arena();
        let mut i = Interner::with_capacity_in(8, &a);
        // 8-slot table; resize at 7 occupied. Insert 100 unique
        // strings — capacity must grow to >= 256.
        for k in 0..100 {
            let s = format!("unique-string-{k}");
            i.intern(&s);
        }
        assert_eq!(i.unique_strings(), 100);
        assert!(i.capacity() >= 128);
        assert!(i.stats.resizes >= 4); // 8 -> 16 -> 32 -> 64 -> 128 -> 256
    }

    #[test]
    fn average_probe_length_stays_low_at_typical_load() {
        let a = arena();
        let mut i = Interner::new_in(&a);
        // Insert 100 unique short strings (well below 7/8 of 256).
        for k in 0..100 {
            let s = format!("k{k}");
            i.intern(&s);
        }
        // Average probe length must stay small for a healthy hash.
        // 100 entries in 256 slots = 39% load, expect <2 probes/lookup.
        assert!(
            i.avg_probe_length() < 2.0,
            "avg probe {} too high — hash function may be degenerate",
            i.avg_probe_length()
        );
    }

    #[test]
    fn aozora_corpus_short_readings_dedup_aggressively() {
        let a = arena();
        let mut i = Interner::new_in(&a);
        // Simulate Aozora corpus pattern: 5 unique short readings
        // hit 200 times each (700 total interns) — should land 5
        // unique entries with hundreds of cache+table hits.
        let readings = ["の", "に", "を", "で", "が"];
        for _ in 0..200 {
            for r in readings {
                i.intern(r);
            }
        }
        assert_eq!(i.unique_strings(), 5);
        // The dedup ratio (cache + table hits) / total calls should
        // be very high — 5 misses, 995 reuses.
        let reuses = i.stats.cache_hits + i.stats.table_hits;
        assert!(
            reuses >= 995,
            "expected >=995 reuses, got {reuses} (calls={})",
            i.stats.calls
        );
    }

    #[test]
    fn intern_preserves_utf8_bytes_exactly() {
        let a = arena();
        let mut i = Interner::new_in(&a);
        let inputs = ["青梅", "おうめ", "明治の頃", "※［＃ほげ］", "🍣"];
        for s in inputs {
            assert_eq!(i.intern(s), s);
        }
        assert_eq!(i.unique_strings(), inputs.len());
    }
}
