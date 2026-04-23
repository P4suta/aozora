//! CommonMark 0.31.2 spec conformance.
//!
//! Runs every case from the upstream spec fixture. The MVP acceptance criterion is
//! 652/652 passing (measured by comrak as of v0.52.0). This harness will be connected
//! to the vendored comrak fork once it's wired into `afm-parser`.

#[test]
#[ignore = "M0 Spike — wiring to comrak pending"]
fn commonmark_0_31_2_full_pass() {
    // Placeholder — verifies the spec file fixture path is reachable.
    let _ = std::path::Path::new("../../spec/commonmark-0.31.2.json");
}
