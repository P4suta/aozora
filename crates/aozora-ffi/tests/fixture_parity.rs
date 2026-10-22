//! Compares C binding outputs with the shared render fixture goldens.

use core::ffi::c_int;
use core::ptr;

use aozora_conformance::{RenderFixture, fixtures_root};
use aozora_ffi::{
    AozoraBytes, AozoraDocument, AozoraStatus, aozora_bytes_free,
    aozora_document_container_pairs_json, aozora_document_diagnostics_json, aozora_document_free,
    aozora_document_new, aozora_document_nodes_json, aozora_document_pairs_json,
    aozora_document_to_html, aozora_document_to_source,
};

type Accessor = unsafe extern "C" fn(*const AozoraDocument, *mut AozoraBytes) -> c_int;

fn load() -> Vec<RenderFixture> {
    let fixtures = RenderFixture::load_group(&fixtures_root(), "render");
    assert!(!fixtures.is_empty(), "no render fixtures found");
    fixtures
}

#[expect(
    unsafe_code,
    reason = "exercises the C ABI call and free contract against golden fixtures"
)]
fn render(source: &str, accessor: Accessor) -> String {
    // SAFETY: the source is live, output slots are writable, and this block frees successful handles and buffers exactly once.
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

#[test]
fn fixture_parity_ffi_serialize_matches_golden() {
    walk("serialize", aozora_document_to_source, |fx| {
        fx.expected_serialize.clone()
    });
}

#[test]
fn fixture_parity_ffi_container_pairs_matches_golden() {
    walk(
        "container_pairs",
        aozora_document_container_pairs_json,
        |fx| fx.expected_container_pairs.clone(),
    );
}
