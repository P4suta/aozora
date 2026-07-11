//! Cross-surface parity gate — C ABI channel.
//!
//! One golden authority (`crates/aozora-conformance/fixtures/render`),
//! N thin walkers. This walker drives the crate's `extern "C"` surface
//! in-process over every render fixture and asserts each `aozora_*_json`
//! / `aozora_document_to_html` output is byte-identical to the golden the
//! in-process `render_gate` pins.
//!
//! Surface coverage: the C ABI deliberately exposes a **four-surface
//! subset** — `to_html`, `diagnostics_json`, `nodes_json`, `pairs_json`.
//! It carries no `serialize` or `container_pairs` export (an embedder that
//! needs the round-trip source or container coordinates uses the Python /
//! WASM / Extism-Go front doors, which do expose all six). The parity gate
//! pins exactly the surfaces the ABI ships; the six-surface coverage lives
//! in the sibling `test_fixture_parity.py` / `parity.mjs` / Go walkers.
//!
//! Framing: every C-ABI accessor hands back the raw `aozora::json` bytes
//! (or `to_html()` bytes) with no trailing newline, so the comparison is
//! byte-exact against the golden file.

#![allow(
    unsafe_code,
    reason = "exercising the crate's C ABI requires the same unsafe extern-C calls a real embedder makes"
)]

use core::ffi::c_int;
use core::ptr;

use aozora_conformance::{RenderFixture, fixtures_root};
use aozora_ffi::{
    AozoraBytes, AozoraDocument, AozoraStatus, aozora_bytes_free, aozora_document_diagnostics_json,
    aozora_document_free, aozora_document_new, aozora_document_nodes_json,
    aozora_document_pairs_json, aozora_document_to_html,
};

/// A JSON / HTML accessor in the C ABI: `(doc, out_bytes) -> status`.
type Accessor = unsafe extern "C" fn(*const AozoraDocument, *mut AozoraBytes) -> c_int;

fn load() -> Vec<RenderFixture> {
    let fixtures = RenderFixture::load_group(&fixtures_root(), "render");
    assert!(!fixtures.is_empty(), "no render fixtures found");
    fixtures
}

/// Parse `source` through the C ABI, invoke `accessor`, and return its
/// bytes as a `String` — mirroring the exact call/free dance an embedder
/// performs. Asserts every status code is `Ok`.
fn render(source: &str, accessor: Accessor) -> String {
    // SAFETY: the argument slice is a live `&str`; `out_doc` points to a
    // stack slot we own; on success we free the handle and its byte
    // buffer exactly once each. This is the documented C-ABI contract.
    unsafe {
        let mut doc: *mut AozoraDocument = ptr::null_mut();
        let status = aozora_document_new(source.as_ptr(), source.len(), &mut doc);
        assert_eq!(
            status,
            AozoraStatus::Ok as c_int,
            "aozora_document_new failed"
        );
        assert!(!doc.is_null(), "aozora_document_new yielded a null handle");

        let mut bytes = AozoraBytes {
            ptr: ptr::null_mut(),
            len: 0,
            cap: 0,
        };
        let status = accessor(doc, &mut bytes);
        assert_eq!(status, AozoraStatus::Ok as c_int, "accessor failed");

        let slice = core::slice::from_raw_parts(bytes.ptr, bytes.len);
        let out = String::from_utf8(slice.to_vec()).expect("C-ABI output is valid UTF-8");

        aozora_bytes_free(bytes);
        aozora_document_free(doc);
        out
    }
}

fn walk(surface: &str, accessor: Accessor, golden: impl Fn(&RenderFixture) -> Option<String>) {
    for fx in &load() {
        let want =
            golden(fx).unwrap_or_else(|| panic!("fixture {}: {surface} golden missing", fx.name));
        let actual = render(&fx.source, accessor);
        assert_eq!(actual, want, "FFI {surface} drift for fixture {}", fx.name);
    }
}

#[test]
fn fixture_parity_ffi_html_matches_golden() {
    walk("html", aozora_document_to_html, |fx| {
        fx.expected_html.clone()
    });
}

#[test]
fn fixture_parity_ffi_diagnostics_matches_golden() {
    walk("diagnostics", aozora_document_diagnostics_json, |fx| {
        fx.expected_diagnostics.clone()
    });
}

#[test]
fn fixture_parity_ffi_nodes_matches_golden() {
    walk("nodes", aozora_document_nodes_json, |fx| {
        fx.expected_nodes.clone()
    });
}

#[test]
fn fixture_parity_ffi_pairs_matches_golden() {
    walk("pairs", aozora_document_pairs_json, |fx| {
        fx.expected_pairs.clone()
    });
}
