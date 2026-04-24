//! The pinned set of CSS class tokens the afm HTML renderer emits.
//!
//! This list is the single source of truth for:
//!
//! * The `css_class_contract` integration test, which ensures every
//!   class has a rule in both the horizontal and vertical themes
//!   shipped by `afm-book`.
//! * The `check_css_class_contract` predicate in
//!   [`crate::test_support`], which enforces "no emitted class falls
//!   outside this list" as a structural invariant.
//!
//! Adding a new class to the renderer is a two-step edit: append the
//! token to [`AFM_CLASSES`] (keeping the list sorted), then add a
//! matching selector to both theme stylesheets. The hygiene tests in
//! `css_class_contract.rs` catch both halves of any drift.

/// Base + modifier class tokens the afm renderer emits on its output
/// elements.
///
/// Kept in strict alphabetical order so PR diffs minimise and the
/// `pinned_classes_are_sorted_and_unique` hygiene test stays green.
/// The semantic groups (`afm-bouten-*`, `afm-container-*`,
/// `afm-section-break-*`, …) are visible via shared prefixes; no
/// hand-grouping needed.
pub const AFM_CLASSES: &[&str] = &[
    "afm-align-end",
    "afm-annotation",
    "afm-bouten",
    "afm-bouten-circle",
    "afm-bouten-cross",
    "afm-bouten-double-circle",
    "afm-bouten-double-under-line",
    "afm-bouten-goma",
    "afm-bouten-janome",
    "afm-bouten-left",
    "afm-bouten-other",
    "afm-bouten-right",
    "afm-bouten-under-line",
    "afm-bouten-wavy-line",
    "afm-bouten-white-circle",
    "afm-bouten-white-sesame",
    "afm-bouten-white-triangle",
    "afm-container",
    "afm-container-align-end",
    "afm-container-indent",
    "afm-container-keigakomi",
    "afm-container-warichu",
    "afm-double-ruby",
    "afm-gaiji",
    "afm-indent",
    "afm-kaeriten",
    "afm-page-break",
    "afm-section-break",
    "afm-section-break-choho",
    "afm-section-break-dan",
    "afm-section-break-spread",
    "afm-tcy",
    "afm-warichu",
];
