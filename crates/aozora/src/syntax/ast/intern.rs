#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "interning validates pool bounds and collision invariants"
)]

//! Interns syntax tree strings and records allocation statistics.

use core::mem;

const FX_PRIME: u64 = 0x517c_c1b7_2722_0a95;
const FX_ROTATE_BITS: u32 = 5;
const LOAD_FACTOR_NUMERATOR: usize = 7;
const LOAD_FACTOR_DENOMINATOR: usize = 8;

#[inline]
pub(crate) fn fx_hash(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0;
    for &b in bytes {
        h = h.rotate_left(FX_ROTATE_BITS) ^ u64::from(b);
        h = h.wrapping_mul(FX_PRIME);
    }
    h
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct InternStats {
    pub(crate) calls: u64,
    pub(crate) cache_hits: u64,
    pub(crate) table_hits: u64,
    pub(crate) allocs: u64,
    pub(crate) long_bypass: u64,
    pub(crate) resizes: u64,
    pub(crate) probe_steps: u64,
}

const INTERN_LENGTH_LIMIT: usize = 64;

const INITIAL_CAPACITY: usize = 256;

/// Index into one string interner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct StrId(pub(crate) u32);

#[derive(Debug, Clone, Default)]
pub(crate) struct StrInterner {
    buf: String,
    spans: Vec<(u32, u32)>,
    table: Vec<Option<StrId>>,
    mask: usize,
    occupied: usize,
    last: Option<StrId>,
    pub(crate) stats: InternStats,
}

impl StrInterner {
    #[cfg(test)]
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Strings beyond `INTERN_LENGTH_LIMIT` may receive separate IDs.
    pub(crate) fn intern(&mut self, s: &str) -> StrId {
        self.stats.calls += 1;

        if let Some(id) = self.last
            && self.resolve(id) == s
        {
            self.stats.cache_hits += 1;
            return id;
        }

        let bytes = s.as_bytes();

        if bytes.len() > INTERN_LENGTH_LIMIT {
            self.stats.long_bypass += 1;
            self.stats.allocs += 1;
            let id = self.alloc(s);
            self.last = Some(id);
            return id;
        }

        if self.table.is_empty() {
            self.table = vec![None; INITIAL_CAPACITY];
            self.mask = INITIAL_CAPACITY
                .checked_sub(1)
                .expect("initial interner capacity is nonzero");
        } else if self
            .occupied
            .saturating_mul(LOAD_FACTOR_DENOMINATOR)
            .cmp(&self.table.len().saturating_mul(LOAD_FACTOR_NUMERATOR))
            .is_ge()
        {
            self.grow();
        }

        let hash = fx_hash(bytes);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "low bits of u64 hash extracted as usize on purpose"
        )]
        let mut idx = (hash as usize) & self.mask;
        for _ in 0..self.table.len() {
            self.stats.probe_steps = self
                .stats
                .probe_steps
                .checked_add(1)
                .expect("interner probe count fits u64");
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
                Some(_) => idx = idx.wrapping_add(1) & self.mask,
            }
        }
        panic!("interner probe table has no empty slot");
    }

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

    fn grow(&mut self) {
        let new_cap = self.table.len().saturating_mul(2);
        let new_mask = new_cap
            .checked_sub(1)
            .expect("grown interner capacity is nonzero");
        let mut new_table: Vec<Option<StrId>> = vec![None; new_cap];
        let old_table = mem::take(&mut self.table);
        for id in old_table.into_iter().flatten() {
            let h = fx_hash(self.resolve(id).as_bytes());
            #[expect(
                clippy::cast_possible_truncation,
                reason = "low bits of u64 hash extracted as usize on purpose"
            )]
            let mut idx = (h as usize) & new_mask;
            for _ in 0..new_table.len() {
                if new_table[idx].is_none() {
                    break;
                }
                idx = idx.wrapping_add(1) & new_mask;
            }
            assert!(
                new_table[idx].is_none(),
                "grown interner table must have an empty slot"
            );
            new_table[idx] = Some(id);
        }
        self.table = new_table;
        self.mask = new_mask;
        self.stats.resizes += 1;
    }

    #[must_use]
    pub(crate) fn resolve(&self, id: StrId) -> &str {
        let (start, len) = self.spans[id.0 as usize];
        &self.buf[start as usize..start as usize + len as usize]
    }

    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.spans.len()
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn capacity(&self) -> usize {
        self.table.len()
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn avg_probe_length(&self) -> f64 {
        let probed = self.stats.calls.saturating_sub(self.stats.cache_hits);
        if probed == 0 {
            0.0
        } else {
            #[expect(
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

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
    }

    #[test]
    fn intern_dedups_and_resolves_round_trip() {
        let mut i = StrInterner::new();

        let a1 = i.intern("あ");
        let a2 = i.intern("あ");
        assert_eq!(a1, a2, "byte-equal intern must return the same id");

        assert_eq!(i.resolve(a1), "あ", "resolve must round-trip the bytes");

        let b = i.intern("い");
        assert_ne!(a1, b, "distinct content must yield distinct ids");
        assert_eq!(i.resolve(b), "い", "resolve must round-trip the bytes");

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
        let long = "x".repeat(128);

        let s1 = i.intern(&long);
        let s2 = i.intern(&long);
        assert_eq!(s1, s2, "consecutive identical long interns share via cache");
        assert_eq!(i.stats.long_bypass, 1, "only the first long call bypasses");
        assert_eq!(i.stats.cache_hits, 1, "second long call hits the cache");
        assert_eq!(i.resolve(s1), long, "bypassed long string resolves exactly");
        assert_eq!(i.capacity(), 0, "no short intern yet — table unsized");

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
        for k in 0..300 {
            let s = format!("unique-string-{k}");
            i.intern(&s);
        }
        assert_eq!(i.len(), 300);
        assert!(i.capacity() >= 512, "table grew past initial capacity");
        assert!(i.stats.resizes >= 1, "at least one resize occurred");
    }

    #[test]
    fn resize_begins_at_the_load_factor_boundary() {
        let mut i = StrInterner::new();
        for k in 0..224 {
            i.intern(&format!("load-boundary-{k}"));
        }
        assert_eq!(i.capacity(), INITIAL_CAPACITY);
        i.intern("load-boundary-trigger");
        assert_eq!(i.capacity(), INITIAL_CAPACITY * 2);
    }

    #[test]
    fn resize_preserves_ids_values_and_stats() {
        let mut i = StrInterner::new();
        let entries: Vec<_> = (0..224)
            .map(|index| {
                let value = format!("resize-entry-{index}");
                let id = i.intern(&value);
                (value, id)
            })
            .collect();
        let before = i.stats;

        let trigger = i.intern("resize-entry-trigger");

        assert_eq!(i.stats.calls, before.calls + 1);
        assert_eq!(i.stats.allocs, before.allocs + 1);
        assert_eq!(i.stats.resizes, before.resizes + 1);
        assert_eq!(i.resolve(trigger), "resize-entry-trigger");
        for (value, id) in entries {
            assert_eq!(i.resolve(id), value);
            assert_eq!(i.intern(&value), id);
        }
    }

    #[test]
    fn average_probe_length_stays_low_at_typical_load() {
        let mut i = StrInterner::new();
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
    fn length_limit_boundary_is_inclusive_of_the_table() {
        let mut i = StrInterner::new();
        let at_limit = "a".repeat(64);
        let breaker = "b".repeat(4);
        let id1 = i.intern(&at_limit);
        let _ = i.intern(&breaker);
        let id2 = i.intern(&at_limit);
        assert_eq!(id1, id2, "a 64-byte string is table-deduped, not bypassed");
        assert_eq!(i.stats.long_bypass, 0, "64 bytes is within the table limit");
    }

    #[test]
    fn stats_count_long_bypass_allocations_and_probe_steps() {
        let mut i = StrInterner::new();
        let long = "x".repeat(100);
        i.intern(&long);
        assert_eq!(i.stats.long_bypass, 1);
        assert_eq!(
            i.stats.allocs, 1,
            "a bypassed long string is still an alloc"
        );
        i.intern("短");
        assert!(
            i.stats.probe_steps >= 1,
            "a short intern records at least one probe step"
        );
    }

    #[test]
    fn is_empty_is_false_after_interning() {
        let mut i = StrInterner::new();
        i.intern("あ");
        assert!(!i.is_empty(), "an interner holding a string is not empty");
    }

    #[test]
    fn avg_probe_length_is_defined_pointwise() {
        let empty = StrInterner::new();
        assert!(
            approx(empty.avg_probe_length(), 0.0),
            "no probed lookups → 0.0, got {}",
            empty.avg_probe_length()
        );
        let mut one = StrInterner::new();
        one.intern("a");
        assert!(
            approx(one.avg_probe_length(), 1.0),
            "one lookup, one probe step → 1.0, got {}",
            one.avg_probe_length()
        );
    }

    #[test]
    fn dedup_survives_a_resize() {
        let mut i = StrInterner::new();
        let strings: Vec<String> = (0..300).map(|k| format!("entry-{k}")).collect();
        let ids: Vec<StrId> = strings.iter().map(|s| i.intern(s)).collect();
        assert!(
            i.stats.resizes >= 1,
            "300 unique strings must trigger a resize"
        );
        let unique_before = i.len();
        for (s, id) in strings.iter().zip(&ids).rev() {
            assert_eq!(i.intern(s), *id, "dedup must survive the resize for {s:?}");
        }
        assert_eq!(
            i.len(),
            unique_before,
            "re-interning after a resize must allocate nothing new"
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
