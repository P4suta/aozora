//! C ABI driver for the aozora parser.
//!
//! Designed for embedding in non-Rust hosts (Ruby / Node / Go / JVM
//! via libffi / FFI / cgo / JNA). The API is **opaque-handle**: every
//! parse produces a `*mut AozoraDocument`, accessed through a small
//! set of `aozora_*` functions, freed by a single matching destructor.
//! Structured data (registry / diagnostics) is exposed as JSON
//! strings rather than C structs because (a) the AST shape is
//! `#[non_exhaustive]` upstream and any C struct mirror would lock
//! callers into a specific revision, and (b) every modern target
//! language already has a JSON reader.
//!
//! ## ABI stability
//!
//! - Function names are `aozora_*` (no namespace tricks).
//! - All inputs are `*const u8 + len` byte slices.
//! - All outputs are `*mut AozoraDocument` opaque handles or
//!   `aozora_string_t`-shaped `(ptr, len, cap)` triples that the
//!   caller hands back to `aozora_string_free`.
//! - All return codes are `int32_t`: `0` = success, negative =
//!   error category, see [`AozoraStatus`].
//!
//! ## Memory ownership
//!
//! Every pointer returned by an `aozora_*` function MUST be released
//! by the matching `aozora_*_free` call. Dropping a handle without
//! calling free leaks the underlying allocation; freeing a handle
//! and then dereferencing it is undefined behaviour (the standard
//! C-API contract).
//!
//! ## Safety
//!
//! This crate must use `unsafe` to honour the C ABI; it is the only
//! crate in the workspace where `unsafe_code = "forbid"` is locally
//! relaxed. Each `unsafe` block carries a `// SAFETY:` justification.

#![allow(
    unsafe_code,
    reason = "C ABI surface inherently requires unsafe blocks (extern \"C\", raw pointers)"
)]

use core::ffi::c_int;
use core::slice;
use std::ffi::CString;

/// Status code returned by every `aozora_*` function.
///
/// `0` is success; negative values are error categories. Positive
/// values are reserved for future warning channels (e.g., "parse
/// completed with diagnostics").
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AozoraStatus {
    /// Operation succeeded.
    Ok = 0,
    /// One of the input pointers was null.
    NullInput = -1,
    /// The input byte slice was not valid UTF-8.
    InvalidUtf8 = -2,
    /// Allocation failed (out of memory).
    AllocFailed = -3,
    /// Internal serialisation error (JSON output construction failed).
    SerializeFailed = -4,
}

/// Opaque handle to a parsed Aozora document. Allocate via
/// [`aozora_document_new`]; free via [`aozora_document_free`].
///
/// The handle owns the parsed source string and the parse output;
/// it is `Send` (callers may move it across threads) but not `Sync`
/// (concurrent access from multiple threads is undefined).
#[derive(Debug)]
pub struct AozoraDocument {
    /// Owned source string. The parse output borrows from it for the
    /// document's lifetime. Currently the legacy aozora_parser path
    /// re-parses by ownership, so this field is a forward-looking
    /// anchor for the borrowed-AST migration that arrives with
    /// Move 2's fused engine.
    #[allow(dead_code, reason = "anchor for borrowed-AST migration; see comment")]
    source: String,
    /// Parse output. Owned; freed when the handle drops.
    parse_result: aozora::ParseResult,
}

/// `(ptr, len, cap)` triple representing an owned `Vec<u8>` returned
/// to the caller. The caller MUST round-trip it through
/// [`aozora_bytes_free`] to release the memory.
///
/// Layout matches `Vec<u8>::from_raw_parts(ptr, len, cap)` so the
/// destructor can reconstruct the vec for drop.
#[repr(C)]
#[derive(Debug)]
pub struct AozoraBytes {
    pub ptr: *mut u8,
    pub len: usize,
    pub cap: usize,
}

/// Construct a [`Document`](AozoraDocument) from a UTF-8 byte slice.
///
/// On success, writes the document handle to `*out_doc` and returns
/// [`AozoraStatus::Ok`]. On failure, writes `null` to `*out_doc` and
/// returns the matching error status.
///
/// # Safety
///
/// - `src_ptr` must point to `src_len` valid UTF-8 bytes.
/// - `out_doc` must point to a writable `*mut AozoraDocument` slot.
/// - The caller must eventually call [`aozora_document_free`] on the
///   returned handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aozora_document_new(
    src_ptr: *const u8,
    src_len: usize,
    out_doc: *mut *mut AozoraDocument,
) -> c_int {
    if src_ptr.is_null() || out_doc.is_null() {
        return AozoraStatus::NullInput as c_int;
    }
    // SAFETY: caller guarantees src_ptr + src_len name a valid byte slice.
    let bytes = unsafe { slice::from_raw_parts(src_ptr, src_len) };
    let Ok(source_str) = core::str::from_utf8(bytes) else {
        // SAFETY: caller guarantees out_doc is writable.
        unsafe { out_doc.write(core::ptr::null_mut()) };
        return AozoraStatus::InvalidUtf8 as c_int;
    };
    let source = source_str.to_owned();
    let parse_result = aozora::parse(&source);
    let doc = Box::new(AozoraDocument {
        source,
        parse_result,
    });
    // SAFETY: caller guarantees out_doc is writable.
    unsafe { out_doc.write(Box::into_raw(doc)) };
    AozoraStatus::Ok as c_int
}

/// Render the document to HTML, returning the result as an owned
/// byte buffer.
///
/// On success, writes the bytes to `*out_html` and returns
/// [`AozoraStatus::Ok`]. The caller MUST call [`aozora_bytes_free`]
/// on the returned [`AozoraBytes`] to release the memory.
///
/// # Safety
///
/// - `doc` must be a non-null handle produced by
///   [`aozora_document_new`] and not yet freed.
/// - `out_html` must point to a writable [`AozoraBytes`] slot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aozora_document_to_html(
    doc: *const AozoraDocument,
    out_html: *mut AozoraBytes,
) -> c_int {
    if doc.is_null() || out_html.is_null() {
        return AozoraStatus::NullInput as c_int;
    }
    // SAFETY: caller guarantees doc is a valid handle.
    let doc_ref: &AozoraDocument = unsafe { &*doc };
    let html = aozora::html::render_from_artifacts(&doc_ref.parse_result.artifacts);
    let bytes = into_owned_bytes(html.into_bytes());
    // SAFETY: caller guarantees out_html is writable.
    unsafe { out_html.write(bytes) };
    AozoraStatus::Ok as c_int
}

/// Render the document's diagnostics as a JSON byte buffer.
///
/// On success, writes the bytes to `*out_json` and returns
/// [`AozoraStatus::Ok`]. Empty document → `"[]"`. The caller MUST
/// call [`aozora_bytes_free`] on the returned [`AozoraBytes`].
///
/// # Safety
///
/// - `doc` must be a non-null handle produced by
///   [`aozora_document_new`] and not yet freed.
/// - `out_json` must point to a writable [`AozoraBytes`] slot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aozora_document_diagnostics_json(
    doc: *const AozoraDocument,
    out_json: *mut AozoraBytes,
) -> c_int {
    if doc.is_null() || out_json.is_null() {
        return AozoraStatus::NullInput as c_int;
    }
    // SAFETY: caller guarantees doc is a valid handle.
    let doc_ref: &AozoraDocument = unsafe { &*doc };
    // The diagnostics include miette::SourceSpan fields that don't
    // implement Serialize; we project to a plain shape first.
    let diags: Vec<DiagnosticView> = doc_ref
        .parse_result
        .diagnostics
        .iter()
        .map(DiagnosticView::from)
        .collect();
    match serde_json::to_vec(&diags) {
        Ok(bytes) => {
            let owned = into_owned_bytes(bytes);
            // SAFETY: caller guarantees out_json is writable.
            unsafe { out_json.write(owned) };
            AozoraStatus::Ok as c_int
        }
        Err(_) => AozoraStatus::SerializeFailed as c_int,
    }
}

/// Free a document handle returned by [`aozora_document_new`].
///
/// # Safety
///
/// - `doc` must be either null (then this is a no-op) or a handle
///   returned by [`aozora_document_new`] that has not already been
///   freed. Double-free is undefined behaviour.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aozora_document_free(doc: *mut AozoraDocument) {
    if !doc.is_null() {
        // SAFETY: caller guarantees doc is a valid handle from
        // aozora_document_new and is not yet freed.
        drop(unsafe { Box::from_raw(doc) });
    }
}

/// Free a byte buffer returned by an `aozora_*` function.
///
/// # Safety
///
/// - `bytes` must be a value previously returned by one of the
///   `aozora_*` functions in this crate. Reusing or aliasing the
///   inner pointer after this call is undefined behaviour.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aozora_bytes_free(bytes: AozoraBytes) {
    if !bytes.ptr.is_null() {
        // SAFETY: bytes was produced by `into_owned_bytes`, which
        // invokes `Vec::into_raw_parts`-equivalent. Reconstructing
        // the Vec with the same triple is the inverse operation.
        drop(unsafe { Vec::from_raw_parts(bytes.ptr, bytes.len, bytes.cap) });
    }
}

/// Convert an owned `Vec<u8>` into the C-ABI [`AozoraBytes`] triple,
/// transferring ownership to the caller.
///
/// Uses `core::mem::forget` (workspace lints normally disallow it)
/// because that is precisely the FFI ownership-transfer dance: the
/// caller takes responsibility for releasing the buffer via
/// [`aozora_bytes_free`], which inverts the `forget` by calling
/// `Vec::from_raw_parts`.
#[allow(
    clippy::disallowed_methods,
    reason = "transferring ownership across the C ABI; aozora_bytes_free is the inverse"
)]
fn into_owned_bytes(mut v: Vec<u8>) -> AozoraBytes {
    let ptr = v.as_mut_ptr();
    let len = v.len();
    let cap = v.capacity();
    core::mem::forget(v);
    AozoraBytes { ptr, len, cap }
}

/// Plain-shape projection of [`aozora::Diagnostic`] for JSON
/// serialisation. The upstream type holds non-Serialize miette
/// fields; this view picks only the offsets, the codepoint (where
/// applicable), and the variant tag.
#[derive(Debug, serde::Serialize)]
struct DiagnosticView {
    kind: &'static str,
    span_start: u32,
    span_end: u32,
    codepoint: Option<char>,
}

impl From<&aozora::Diagnostic> for DiagnosticView {
    fn from(d: &aozora::Diagnostic) -> Self {
        // Each variant projects its (kind, span, optional codepoint)
        // triplet. The catch-all `_` arm keeps the impl total against
        // the upstream `#[non_exhaustive]` contract.
        match d {
            aozora::Diagnostic::SourceContainsPua { codepoint, span, .. } => Self {
                kind: "source_contains_pua",
                span_start: span.start,
                span_end: span.end,
                codepoint: Some(*codepoint),
            },
            aozora::Diagnostic::UnclosedBracket { span, .. } => Self {
                kind: "unclosed_bracket",
                span_start: span.start,
                span_end: span.end,
                codepoint: None,
            },
            aozora::Diagnostic::UnmatchedClose { span, .. } => Self {
                kind: "unmatched_close",
                span_start: span.start,
                span_end: span.end,
                codepoint: None,
            },
            aozora::Diagnostic::ResidualAnnotationMarker { span, .. } => Self {
                kind: "residual_annotation_marker",
                span_start: span.start,
                span_end: span.end,
                codepoint: None,
            },
            aozora::Diagnostic::UnregisteredSentinel { codepoint, span, .. } => Self {
                kind: "unregistered_sentinel",
                span_start: span.start,
                span_end: span.end,
                codepoint: Some(*codepoint),
            },
            aozora::Diagnostic::RegistryOutOfOrder { span, .. } => Self {
                kind: "registry_out_of_order",
                span_start: span.start,
                span_end: span.end,
                codepoint: None,
            },
            aozora::Diagnostic::RegistryPositionMismatch { expected, span, .. } => Self {
                kind: "registry_position_mismatch",
                span_start: span.start,
                span_end: span.end,
                codepoint: Some(*expected),
            },
            // `#[non_exhaustive]` upstream — future variants land here
            // until added explicitly above.
            _ => Self {
                kind: "unknown",
                span_start: 0,
                span_end: 0,
                codepoint: None,
            },
        }
    }
}

// ----------------------------------------------------------------------
// Suppress unused-import warning for serde_json under cfg test paths.
// ----------------------------------------------------------------------

// Anchor a CString reference to keep the dep present in case we add
// an `aozora_version_string()` accessor in the next commit (it's a
// natural fit for cstr-bridged metadata).
#[doc(hidden)]
pub fn _link_cstring() -> Option<CString> {
    CString::new("aozora").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// End-to-end smoke: parse, render to HTML, free. Exercises the
    /// happy path of every public entry point.
    #[test]
    fn end_to_end_roundtrip_through_c_abi() {
        let src = b"Hello, world.";
        let mut doc: *mut AozoraDocument = core::ptr::null_mut();
        let status = unsafe { aozora_document_new(src.as_ptr(), src.len(), &mut doc) };
        assert_eq!(status, AozoraStatus::Ok as c_int);
        assert!(!doc.is_null());

        let mut html = AozoraBytes {
            ptr: core::ptr::null_mut(),
            len: 0,
            cap: 0,
        };
        let status = unsafe { aozora_document_to_html(doc, &mut html) };
        assert_eq!(status, AozoraStatus::Ok as c_int);
        let html_str = unsafe { core::str::from_utf8(slice::from_raw_parts(html.ptr, html.len)) }
            .expect("html is utf8");
        assert!(html_str.contains("Hello"));
        unsafe { aozora_bytes_free(html) };

        let mut diag = AozoraBytes {
            ptr: core::ptr::null_mut(),
            len: 0,
            cap: 0,
        };
        let status = unsafe { aozora_document_diagnostics_json(doc, &mut diag) };
        assert_eq!(status, AozoraStatus::Ok as c_int);
        let json = unsafe { core::str::from_utf8(slice::from_raw_parts(diag.ptr, diag.len)) }
            .expect("json is utf8");
        assert_eq!(json, "[]"); // no diagnostics for plain text
        unsafe { aozora_bytes_free(diag) };

        unsafe { aozora_document_free(doc) };
    }

    #[test]
    fn null_input_returns_null_input_status() {
        let mut doc: *mut AozoraDocument = core::ptr::null_mut();
        let status = unsafe { aozora_document_new(core::ptr::null(), 0, &mut doc) };
        assert_eq!(status, AozoraStatus::NullInput as c_int);
        assert!(doc.is_null());
    }

    #[test]
    fn invalid_utf8_returns_invalid_utf8_status() {
        let bad = [0xFFu8, 0xFE, 0xFD];
        let mut doc: *mut AozoraDocument = core::ptr::null_mut();
        let status = unsafe { aozora_document_new(bad.as_ptr(), bad.len(), &mut doc) };
        assert_eq!(status, AozoraStatus::InvalidUtf8 as c_int);
        assert!(doc.is_null());
    }

    #[test]
    fn diagnostics_emit_for_pua_collision() {
        let src = "abc\u{E001}def".as_bytes();
        let mut doc: *mut AozoraDocument = core::ptr::null_mut();
        let status = unsafe { aozora_document_new(src.as_ptr(), src.len(), &mut doc) };
        assert_eq!(status, AozoraStatus::Ok as c_int);
        let mut diag = AozoraBytes {
            ptr: core::ptr::null_mut(),
            len: 0,
            cap: 0,
        };
        let status = unsafe { aozora_document_diagnostics_json(doc, &mut diag) };
        assert_eq!(status, AozoraStatus::Ok as c_int);
        let json = unsafe { core::str::from_utf8(slice::from_raw_parts(diag.ptr, diag.len)) }
            .expect("json is utf8");
        assert!(json.contains("source_contains_pua"), "diag json: {json}");
        unsafe { aozora_bytes_free(diag) };
        unsafe { aozora_document_free(doc) };
    }

    #[test]
    fn freeing_null_handle_is_safe_noop() {
        unsafe { aozora_document_free(core::ptr::null_mut()) };
    }

    #[test]
    fn link_cstring_returns_some() {
        // Anchor for the std::ffi::CString import.
        assert!(_link_cstring().is_some());
    }
}
