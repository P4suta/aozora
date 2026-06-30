//! Owned, lifetime-free string interner.
//!
//! Owns all interned bytes in a single `String` and hands back a [`StrId`]
//! index. It offers two observable contracts:
//!
//! - **Dedup**: byte-equal `intern` calls return the same handle.
//! - **[`InternStats`]**: the full counter set the corpus-sweep dedup-ratio
//!   report reads (`calls`, `cache_hits`, `table_hits`, `allocs`,
//!   `long_bypass`, `resizes`, `probe_steps`). The probe table is a real
//!   open-addressing algorithm, so the hash-health counters carry real
//!   signal here, not zero placeholders.
//!
//! ## Backing
//!
//! Open addressing with linear probing over a `Vec<Option<StrId>>` probe
//! table. Each slot stores a [`StrId`], resolved against `buf` + `spans` to
//! compare bytes on a probe. A `fx_hash` mix, power-of-two capacity, 7/8
//! load-factor resize, 1-slot inline cache, and 64-byte table bypass round
//! out the design.
//!
//! The *handle contract* (dedup + resolve) is the invariant.

/// FxHash-style mix constant. The same constant rustc internally uses for
/// `FxHasher`; chosen for fast diffusion on short inputs.
const FX_PRIME: u64 = 0x517c_c1b7_2722_0a95;

/// `wrapping_mul`-and-xor mix loop. Fast on short inputs (the dominant case for
/// Aozora ruby readings); avoids the per-call state setup cost of std
/// `SipHash`. Single-authority hash mix shared by the owned interner and any
/// other open-addressing table over the same byte streams.
#[inline]
pub(crate) fn fx_hash(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0;
    for &b in bytes {
        h = h.rotate_left(5) ^ u64::from(b);
        h = h.wrapping_mul(FX_PRIME);
    }
    h
}

/// Diagnostic counters surfaced by an interner's `stats`.
///
/// Lifetime-free `Copy` counter set the corpus-sweep dedup-ratio report reads
/// (`calls`, `cache_hits`, `table_hits`, `allocs`, `long_bypass`, `resizes`,
/// `probe_steps`).
#[derive(Debug, Clone, Copy, Default)]
pub struct InternStats {
    /// Total `intern` calls (every entry into the API).
    pub calls: u64,
    /// Calls served from the inline cache.
    pub cache_hits: u64,
    /// Calls that landed on an existing table entry (no allocation).
    pub table_hits: u64,
    /// Calls that allocated a new entry.
    pub allocs: u64,
    /// Calls that bypassed the table because the string exceeded
    /// `INTERN_LENGTH_LIMIT` — counted as an alloc as well.
    pub long_bypass: u64,
    /// Total resize events the table performed.
    pub resizes: u64,
    /// Total probe steps walked across all `intern` calls. Divided by
    /// `calls - cache_hits` gives the average probe length, the canonical
    /// hash-table health metric.
    pub probe_steps: u64,
}

/// Byte length beyond which the interner bypasses its probe table: long
/// strings allocate a fresh [`StrId`] without a table entry (no dedup).
/// They almost never repeat in practice and hashing them costs more than the
/// alloc a dedup would save.
const INTERN_LENGTH_LIMIT: usize = 64;

/// Initial probe-table capacity, allocated lazily on the first short intern.
/// Power of two so probe-index is `hash & mask`.
const INITIAL_CAPACITY: usize = 256;

/// Stable handle to an interned string inside a [`StrInterner`].
///
/// A `u32` index into the interner's `spans`, resolvable via
/// [`StrInterner::resolve`]. Mapping rule `&'src str (interned) -> StrId(u32)`.
///
/// `Hash`/`Ord` are derived so a `StrId` can key the owned node store's
/// auxiliary maps and sort deterministically; both are zero-cost on a `u32`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StrId(pub u32);

/// Owned, lifetime-free string interner.
///
/// Deduplicates byte-equal strings and returns a stable [`StrId`], owning
/// every unique string's bytes in a single `String` (`buf`) plus a
/// `(start, len)` span per id (`spans`). Dedup is served by an
/// open-addressing probe table (`table`).
///
/// Derives `Clone` (the owned output may be cached/cloned by the #237
/// incremental cache). It does **not** derive `Copy` (owns heap storage) nor
/// `PartialEq`/`Eq`: the reused `stats: InternStats` field does not implement
/// `PartialEq`, so deriving it here would not compile, and structural
/// equality of an interner is not a meaningful operation. `Default` is the
/// trivially-empty interner (empty probe table; the first short intern sizes
/// it to `INITIAL_CAPACITY`).
#[derive(Debug, Clone, Default)]
pub struct StrInterner {
    /// Every unique string's bytes, concatenated in intern order.
    buf: String,
    /// `(start, len)` byte span into `buf` for each id; `spans[id.0 as usize]`
    /// locates `StrId(id)`. Indexed by stable id, not by probe position.
    spans: Vec<(u32, u32)>,
    /// Open-addressing probe table: `None` = empty slot, `Some(id)` = the
    /// [`StrId`] whose bytes hash to this slot. A probe resolves the
    /// candidate id against `buf` + `spans` to compare bytes. Empty until the
    /// first short intern lazily sizes it to `INITIAL_CAPACITY`.
    table: Vec<Option<StrId>>,
    /// `capacity - 1`; `capacity` is a power of two (or `0` before the table
    /// is first sized). Makes the slot index a single `hash & mask`.
    mask: usize,
    /// Number of occupied probe-table slots. Counts table-resident (short)
    /// strings only; long strings bypass the table, so this can be below
    /// `spans.len()`. Drives the load-factor resize.
    occupied: usize,
    /// Inline cache of the last interned id, short-circuiting so long
    /// identical runs count as `cache_hits`.
    last: Option<StrId>,
    /// Diagnostic counters feeding the dedup-ratio reporting.
    pub stats: InternStats,
}

impl StrInterner {
    /// Empty interner.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Intern `s`, returning a stable [`StrId`]. Byte-equal calls return the
    /// same id (dedup) and update the `InternStats` accounting.
    ///
    /// # Panics
    ///
    /// Panics if the interner's backing buffer would exceed `u32::MAX` bytes,
    /// a single interned string exceeds `u32::MAX` bytes, or the unique-string
    /// count exceeds `u32::MAX` — none reachable for any realistic document.
    pub fn intern(&mut self, s: &str) -> StrId {
        self.stats.calls += 1;

        // Inline cache: identical consecutive interns short-circuit on a
        // single resolve-and-compare.
        if let Some(id) = self.last
            && self.resolve(id) == s
        {
            self.stats.cache_hits += 1;
            return id;
        }

        let bytes = s.as_bytes();

        // Length-threshold bypass — long strings skip the probe table (no
        // dedup). They still allocate a `StrId` so payloads can reference
        // them.
        if bytes.len() > INTERN_LENGTH_LIMIT {
            self.stats.long_bypass += 1;
            self.stats.allocs += 1;
            let id = self.alloc(s);
            self.last = Some(id);
            return id;
        }

        // Lazily size the table on the first short intern, then keep the load
        // factor under 7/8 (power-of-two table makes this a multiply + compare,
        // no division).
        if self.table.is_empty() {
            self.table = vec![None; INITIAL_CAPACITY];
            self.mask = INITIAL_CAPACITY - 1;
        } else if self.occupied.saturating_mul(8) >= self.table.len().saturating_mul(7) {
            self.grow();
        }

        let hash = fx_hash(bytes);
        #[allow(
            clippy::cast_possible_truncation,
            reason = "low bits of u64 hash extracted as usize on purpose"
        )]
        let mut idx = (hash as usize) & self.mask;
        loop {
            self.stats.probe_steps += 1;
            match self.table[idx] {
                Some(existing) if self.resolve(existing) == s => {
                    self.stats.table_hits += 1;
                    self.last = Some(existing);
                    return existing;
                }
                None => {
                    let id = self.alloc(s);
                    self.table[idx] = Some(id);
                    self.occupied += 1;
                    self.stats.allocs += 1;
                    self.last = Some(id);
                    return id;
                }
                Some(_) => idx = (idx + 1) & self.mask,
            }
        }
    }

    /// Append `s`'s bytes to `buf`, record its span, and mint a fresh
    /// [`StrId`]. Does not touch the probe table — callers (the fresh-slot and
    /// long-bypass paths) own that bookkeeping.
    fn alloc(&mut self, s: &str) -> StrId {
        let start =
            u32::try_from(self.buf.len()).expect("owned interner buffer exceeds u32 byte range");
        let len = u32::try_from(s.len()).expect("interned string exceeds u32 byte length");
        let id = StrId(
            u32::try_from(self.spans.len())
                .expect("owned interner unique-string count exceeds u32"),
        );
        self.buf.push_str(s);
        self.spans.push((start, len));
        id
    }

    /// Doubles probe-table capacity and rebuilds it via fresh probing.
    /// `buf` / `spans` are untouched, so live [`StrId`]s stay valid.
    fn grow(&mut self) {
        let new_cap = self.table.len().saturating_mul(2);
        let new_mask = new_cap - 1;
        let mut new_table: Vec<Option<StrId>> = vec![None; new_cap];
        // Collect occupied ids up front so the re-probe below can resolve each
        // against `buf` without overlapping a borrow of `table`.
        let ids: Vec<StrId> = self.table.iter().flatten().copied().collect();
        for id in ids {
            let h = fx_hash(self.resolve(id).as_bytes());
            #[allow(
                clippy::cast_possible_truncation,
                reason = "low bits of u64 hash extracted as usize on purpose"
            )]
            let mut idx = (h as usize) & new_mask;
            while new_table[idx].is_some() {
                idx = (idx + 1) & new_mask;
            }
            new_table[idx] = Some(id);
        }
        self.table = new_table;
        self.mask = new_mask;
        self.stats.resizes += 1;
    }

    /// Resolve a [`StrId`] back to its interned bytes.
    ///
    /// # Panics
    ///
    /// Panics if `id` was not produced by this interner.
    #[must_use]
    pub fn resolve(&self, id: StrId) -> &str {
        let (start, len) = self.spans[id.0 as usize];
        &self.buf[start as usize..start as usize + len as usize]
    }

    /// Number of distinct strings held — the size of the dense [`StrId`] space
    /// (`StrId(0)..StrId(len)`). Counts every interned string, short and long,
    /// including table-bypassed long strings, which the owned tree must still
    /// address by id.
    #[must_use]
    pub fn len(&self) -> usize {
        self.spans.len()
    }

    /// Whether the interner holds no strings.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    /// Current probe-table capacity (`0` before the first short intern).
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.table.len()
    }

    /// Average probe length per non-cache-hit lookup. Returns `0.0` when no
    /// probed lookups have occurred. Meaningful because the probe table is a
    /// real open-addressing algorithm, so `probe_steps` carries real signal.
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

    #[test]
    fn distinct_interleaved_content_probes_the_table() {
        // Interleave two distinct readings so the inline cache never serves
        // them — every reuse must come from the probe table.
        let mut i = StrInterner::new();
        let a = i.intern("青");
        let b = i.intern("空");
        for _ in 0..50 {
            assert_eq!(i.intern("青"), a);
            assert_eq!(i.intern("空"), b);
        }
        assert_eq!(i.len(), 2);
        assert_eq!(i.stats.allocs, 2, "two fresh allocations only");
        assert!(
            i.stats.table_hits >= 100,
            "interleaved reuse hits the table"
        );
        assert!(
            i.stats.cache_hits == 0,
            "alternation defeats the inline cache"
        );
    }

    #[test]
    fn resolve_round_trips_utf8_bytes_exactly() {
        let mut i = StrInterner::new();
        let inputs = ["青梅", "おうめ", "明治の頃", "※［＃ほげ］", "🍣"];
        let ids: Vec<_> = inputs.iter().map(|s| i.intern(s)).collect();
        for (id, s) in ids.iter().zip(inputs) {
            assert_eq!(i.resolve(*id), s);
        }
        assert_eq!(i.len(), inputs.len());
    }

    #[test]
    fn long_strings_bypass_table_without_table_dedup() {
        let mut i = StrInterner::new();
        let long = "x".repeat(128); // beyond INTERN_LENGTH_LIMIT (64)

        // First long call bypasses the table; the second identical call hits
        // the inline cache (which compares full content), so they share an id.
        let s1 = i.intern(&long);
        let s2 = i.intern(&long);
        assert_eq!(s1, s2, "consecutive identical long interns share via cache");
        assert_eq!(i.stats.long_bypass, 1, "only the first long call bypasses");
        assert_eq!(i.stats.cache_hits, 1, "second long call hits the cache");
        assert_eq!(i.resolve(s1), long, "bypassed long string resolves exactly");
        // Long strings consume no probe-table slot.
        assert_eq!(i.capacity(), 0, "no short intern yet — table unsized");

        // A different long string re-primes the cache, so a later identical
        // long string can no longer short-circuit — and, with no table dedup,
        // re-allocates a *distinct* id whose bytes are still identical
        // (output-invariant despite the duplicate allocation).
        let other = "y".repeat(128);
        let _ = i.intern(&other);
        let s3 = i.intern(&long);
        assert_eq!(
            i.stats.long_bypass, 3,
            "non-consecutive long dup re-bypasses"
        );
        assert_ne!(s1, s3, "long strings are not table-deduped");
        assert_eq!(
            i.resolve(s3),
            i.resolve(s1),
            "distinct ids, identical bytes"
        );
    }

    #[test]
    fn many_unique_strings_trigger_resize() {
        let mut i = StrInterner::new();
        // 256-slot initial table; resize at 7/8 load. Insert 300 unique
        // strings — capacity must grow past the initial 256.
        for k in 0..300 {
            let s = format!("unique-string-{k}");
            i.intern(&s);
        }
        assert_eq!(i.len(), 300);
        assert!(i.capacity() >= 512, "table grew past initial capacity");
        assert!(i.stats.resizes >= 1, "at least one resize occurred");
    }

    #[test]
    fn average_probe_length_stays_low_at_typical_load() {
        let mut i = StrInterner::new();
        // 100 unique short strings in a 256-slot table (39% load).
        for k in 0..100 {
            let s = format!("k{k}");
            i.intern(&s);
        }
        assert!(
            i.avg_probe_length() < 2.0,
            "avg probe {} too high — hash function may be degenerate",
            i.avg_probe_length()
        );
    }

    #[test]
    fn empty_interner_has_no_strings_and_unsized_table() {
        let i = StrInterner::new();
        assert!(i.is_empty());
        assert_eq!(i.len(), 0);
        assert_eq!(
            i.capacity(),
            0,
            "table is sized lazily on first short intern"
        );
    }
}
