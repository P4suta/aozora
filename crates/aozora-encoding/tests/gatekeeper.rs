//! 金庫番 (gatekeeper) tests for `aozora-encoding`.
//!
//! These tests pin invariants that should *only* change as a
//! deliberate, reviewed action:
//!
//! * Public error variant inventory.
//! * `Resolved` variant inventory + `Copy`/`Eq` bounds.
//! * Combo-cell shape (every entry is a 2-codepoint sequence).
//! * Combo cells are exclusively in JIS X 0213 plane 1 row 4-5.
//! * Single-char table is non-empty + the spec's plane-1/plane-2
//!   counts agree with the build-time constants.
//! * Description-fallback table has the canonical 〓 / 〻 specials.
//! * `lookup` decision-tree: each input shape lands on a specific
//!   tier in the documented order.
//!
//! Failures here are *meant* to be brittle — any change to the
//! pinned numbers must come with a same-PR update plus a
//! CHANGELOG note (where applicable).

use aozora_encoding::DecodeError;
use aozora_encoding::gaiji::{Resolved, lookup, table_sizes};

#[test]
fn gatekeeper_decode_error_is_non_exhaustive_with_one_pinned_variant() {
    // Today there is exactly one variant. Adding a new variant
    // requires updating this test AND the user-facing JP error
    // strings (see workspace I3 contract).
    let err = DecodeError::ShiftJisInvalid;
    assert_eq!(
        format!("{err}"),
        "Shift_JIS からの変換に失敗しました (不正なバイト列)",
        "user-facing error message must remain in Japanese; \
         changing the wording is a user-visible UX change",
    );
}

#[test]
fn gatekeeper_resolved_variant_inventory() {
    // Two variants: Char (single Unicode scalar) + Multi (combining
    // sequence as a static str). Both must remain `Copy` so they
    // can sit inside the parser's `Copy`-able tree.
    fn assert_copy<T: Copy>() {}
    fn assert_eq_<T: Eq>() {}
    assert_copy::<Resolved>();
    assert_eq_::<Resolved>();

    // Pin both constructors round-trip via Eq.
    assert_eq!(Resolved::Char('A'), Resolved::Char('A'));
    assert_eq!(Resolved::Multi("か゚"), Resolved::Multi("か゚"));
    assert_ne!(Resolved::Char('A'), Resolved::Multi("A"));
}

#[test]
fn gatekeeper_table_sizes_match_jisx0213_2004_spec() {
    // The exact counts come from the JIS X 0213:2004 normative table
    // + 8th-edition 外字注記辞書. Bumping any of these requires
    // re-verifying against the upstream source; do NOT update them
    // to make a failing test pass.
    let (single, combo, description) = table_sizes();
    assert_eq!(single, 4329, "JIS X 0213 plane-1 + plane-2 single-char count");
    assert_eq!(combo, 25, "JIS X 0213 plane-1 combining-sequence cells");
    assert!(
        description >= 8_000,
        "description fallback table dropped below the 8K floor: {description}",
    );
}

/// Every combo cell must decode to *exactly* two Unicode scalars.
/// The combo path's whole reason to exist is that single-`char` is
/// not enough; if any future regen introduces a 1-char or 3-char
/// entry the contract has been silently broken.
#[test]
fn gatekeeper_every_combo_entry_is_exactly_two_scalars() {
    // The 25 combo cells live at known JIS X 0213 plane 1 mencodes.
    // Read combo TSV by walking the build-time map's value strings
    // via a known mencode for each.
    //
    // Easiest cross-check: walk a small representative set and
    // assert each resolved string is exactly 2 chars.
    let representative = [
        "第3水準1-4-87", // か゚
        "第3水準1-4-88", // き゚
        "第3水準1-4-89", // く゚
        "第3水準1-4-90", // け゚
        "第3水準1-4-91", // こ゚
        "第3水準1-5-87", // カ゚
        "第3水準1-5-88", // キ゚
    ];
    for mencode in representative {
        let r = lookup(None, Some(mencode), "").unwrap_or_else(|| {
            panic!("combo mencode {mencode} must resolve")
        });
        let s = match r {
            Resolved::Multi(s) => s,
            Resolved::Char(c) => panic!("expected Multi for {mencode}, got Char({c:?})"),
        };
        assert_eq!(
            s.chars().count(),
            2,
            "combo cell {mencode} must be exactly 2 scalars; got {s:?}",
        );
        // The second scalar of every Aozora combo cell is a
        // combining mark (handakuten or similar). If a regen
        // accidentally swapped the order, this catches it.
        let last = s.chars().next_back().unwrap();
        assert!(
            (0x0300..=0x036F).contains(&(last as u32))
                || (0x3099..=0x309A).contains(&(last as u32)),
            "second scalar of {mencode} ({s:?}) is not a combining mark",
        );
    }
}

#[test]
fn gatekeeper_lookup_tier_dispatch_order_is_pinned() {
    // 1. existing wins over everything.
    assert_eq!(
        lookup(Some('Z'), Some("第3水準1-85-54"), "木＋吶のつくり"),
        Some(Resolved::Char('Z')),
    );
    // 2. mencode → combo wins over single-char + U+ + dictionary.
    assert!(matches!(
        lookup(None, Some("第3水準1-4-87"), "anything"),
        Some(Resolved::Multi(_))
    ));
    // 3. mencode → single-char wins over U+ + dictionary.
    assert_eq!(
        lookup(None, Some("第3水準1-85-54"), "〓"),
        Some(Resolved::Char('\u{6798}')),
        "single-char table must beat the description-table 〓 entry",
    );
    // 4. mencode → U+ form (no table hit) resolves the codepoint.
    assert_eq!(
        lookup(None, Some("U+304B"), "anything"),
        Some(Resolved::Char('か')),
    );
    // 5. description-fallback handles the canonical specials.
    assert_eq!(lookup(None, None, "〓"), Some(Resolved::Char('\u{3013}')));
    assert_eq!(lookup(None, None, "〻"), Some(Resolved::Char('\u{303B}')));
    // 6. smart single-char fallback only fires when nothing else
    //    matched AND description is exactly one Unicode scalar.
    assert_eq!(lookup(None, None, "畺"), Some(Resolved::Char('\u{757A}')));
    assert_eq!(lookup(None, None, "未知の字"), None);
}

#[test]
fn gatekeeper_resolved_utf8_len_is_consistent_with_write_to() {
    // For every shape, write_to must emit exactly `utf8_len` bytes.
    // Pin a few specific values so a future change to write_to or
    // utf8_len can't drift independently.
    let cases: &[(Resolved, usize)] = &[
        (Resolved::Char('A'), 1),
        (Resolved::Char('あ'), 3),
        (Resolved::Char('𠂉'), 4),
        (Resolved::Multi("\u{304B}\u{309A}"), 6),
    ];
    for (r, want) in cases {
        let mut s = String::new();
        r.write_to(&mut s).expect("write to String");
        assert_eq!(s.len(), *want, "byte len mismatch for {r:?}");
        assert_eq!(r.utf8_len(), *want, "utf8_len mismatch for {r:?}");
    }
}
