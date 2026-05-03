//! Round-trip + uniqueness invariants for [`Sentinel`].
//!
//! The PUA sentinel scheme is the single point through which every
//! phase of the lex pipeline talks about Aozora-vs-plain bytes — a
//! `from_char` / `as_char` mismatch silently misroutes every entry in
//! every registry. The properties here gate that surface against:
//!
//! 1. `from_char ∘ as_char = Some` for every `Sentinel::ALL` variant
//!    (the pipeline relies on every emitted sentinel being recognised
//!    on the way back).
//! 2. `as_char` is injective over `Sentinel::ALL` (no two variants
//!    encode to the same codepoint, so a registry hit is unambiguous).
//! 3. `from_char(c) = None` for every char outside the four reserved
//!    sentinel codepoints (a stray PUA byte in source data must not
//!    masquerade as a registry sentinel).
//!
//! A 4-variant enum is technically exhaustive without proptest, but
//! the negative property #3 is naturally a property — generate any
//! `char`, exclude the four sentinels, assert `from_char` returns
//! `None`. That keeps the gate honest if a future variant lands.

use aozora_proptest::config::default_config;
use aozora_spec::Sentinel;
use proptest::prelude::*;

#[test]
fn round_trip_holds_for_all_variants() {
    for kind in Sentinel::ALL {
        let c = kind.as_char();
        assert_eq!(
            Sentinel::from_char(c),
            Some(kind),
            "{kind:?} round-trip lost the variant"
        );
    }
}

#[test]
fn as_char_is_injective_over_all_variants() {
    let mut chars: Vec<char> = Sentinel::ALL.iter().map(|s| s.as_char()).collect();
    chars.sort_unstable();
    chars.dedup();
    assert_eq!(
        chars.len(),
        Sentinel::ALL.len(),
        "Sentinel::ALL has at least two variants encoding to the same codepoint"
    );
}

proptest! {
    #![proptest_config(default_config())]

    /// `from_char` must reject every codepoint that is not one of the
    /// four reserved sentinels. A stray PUA byte in source data ends
    /// up in the normalized buffer with no registry entry; treating
    /// it as a sentinel would make every renderer dispatch on a phantom
    /// position.
    #[test]
    fn from_char_rejects_non_sentinel_codepoints(c in any::<char>()) {
        let reserved: [char; 4] = [
            Sentinel::Inline.as_char(),
            Sentinel::BlockLeaf.as_char(),
            Sentinel::BlockOpen.as_char(),
            Sentinel::BlockClose.as_char(),
        ];
        let from = Sentinel::from_char(c);
        if reserved.contains(&c) {
            prop_assert!(from.is_some(), "{c:?} is reserved but from_char returned None");
        } else {
            prop_assert!(from.is_none(), "{c:?} is not reserved but from_char returned {from:?}");
        }
    }

    /// `from_char ∘ as_char` is the identity on `Sentinel::ALL`. Drives
    /// the same property as `round_trip_holds_for_all_variants` but as
    /// a proptest so a future widening of the variant set does not need
    /// a hand-written test addition.
    #[test]
    fn from_char_after_as_char_is_identity(idx in 0usize..Sentinel::ALL.len()) {
        let kind = Sentinel::ALL[idx];
        prop_assert_eq!(Sentinel::from_char(kind.as_char()), Some(kind));
    }
}
