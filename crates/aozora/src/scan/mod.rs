#![expect(
    clippy::expect_used,
    reason = "the fixed trigger patterns are a valid static automaton"
)]

//! Finds notation delimiter offsets through [`scan_offsets`].
//! ADR-0035 requires a safe, portable matcher, provided by [`aho_corasick`].

#![forbid(unsafe_code)]

#[cfg(test)]
mod naive;
mod trait_def;

pub(crate) use trait_def::OffsetSink;

#[cfg(test)]
pub(crate) use naive::NaiveScanner;

fn automaton() -> &'static aho_corasick::AhoCorasick {
    use crate::spec::trigger::ALL_TRIGGER_TRIGRAMS;
    use aho_corasick::{AhoCorasick, MatchKind};
    use std::sync::OnceLock;

    static AUTOMATON: OnceLock<AhoCorasick> = OnceLock::new();
    AUTOMATON.get_or_init(|| {
        AhoCorasick::builder()
            .match_kind(MatchKind::Standard)
            .build(ALL_TRIGGER_TRIGRAMS)
            .expect("the 11 fixed trigger trigrams always compile")
    })
}

// mutants::skip: removing prewarming changes initialization timing without changing output.
#[cfg_attr(test, mutants::skip)]
pub(crate) fn prewarm() {
    automaton();
}

fn scan_into<S: OffsetSink>(source: &str, sink: &mut S) {
    for m in automaton().find_iter(source) {
        sink.push(u32::try_from(m.start()).unwrap_or(u32::MAX));
    }
}

#[must_use]
pub(crate) fn scan_offsets(source: &str) -> Vec<u32> {
    let mut sink = Vec::new();
    scan_into(source, &mut sink);
    sink
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_matches_naive_on_mixed_sample() {
        let s = "漢《かん》字、※［＃ここまで］「終わり」";
        let scanned = scan_offsets(s);
        let naive = NaiveScanner.scan_offsets(s);
        assert_eq!(scanned, naive);
        assert_eq!(scanned.len(), 8, "sample has 8 triggers");
    }

    #[test]
    fn skips_kana_sharing_the_e3_lead_byte() {
        assert_eq!(scan_offsets("あいうえおこんにちは"), Vec::<u32>::new());
    }
}
