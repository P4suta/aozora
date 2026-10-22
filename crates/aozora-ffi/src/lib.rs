//! Exposes C bindings through [`aozora_document_new`] and the document accessors.
//! Release builds use `panic = "abort"`, so a Rust panic terminates the host process.

use core::ffi::c_int;
use core::slice;

/// Wire schema version carried by every structured JSON result.
pub const AOZORA_SCHEMA_VERSION: u32 = aozora::json::SCHEMA_VERSION;

/// Operation status, where zero means success and negative values mean errors.
/// Positive values remain reserved for future warnings.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AozoraStatus {
    /// Operation succeeded.
    Ok = 0,
    /// An input pointer was null.
    NullInput = -1,
    /// The input contains invalid UTF-8.
    InvalidUtf8 = -2,
    /// Memory allocation failed.
    AllocFailed = -3,
    /// Output serialization failed.
    SerializeFailed = -4,
    /// The source byte length exceeds [`u32::MAX`].
    SourceTooLarge = -5,
}

/// An opaque parsed document handle.
/// Create it with [`aozora_document_new`] and release it once with [`aozora_document_free`].
/// A handle can move between threads, but callers must serialize access and destruction.
#[derive(Debug)]
pub struct AozoraDocument {
    inner: aozora::Document,
}

/// An owned byte buffer returned by the library.
/// Pass the unchanged `ptr`, `len`, and `cap` to [`aozora_bytes_free`] exactly once.
/// The payload has no null terminator.
#[repr(C)]
#[derive(Debug)]
pub struct AozoraBytes {
    /// Allocation pointer, non-null for buffers returned by the library.
    /// A null pointer acts as an empty sentinel for [`aozora_bytes_free`].
    pub ptr: *mut u8,
    /// Number of initialized, readable bytes, no greater than `cap`.
    pub len: usize,
    /// Allocation capacity in bytes, which must remain unchanged until release.
    pub cap: usize,
}

/// Returns the parser version as an owned UTF-8 buffer through `out_version`.
/// Returns [`AozoraStatus::NullInput`] for a null output pointer, or [`AozoraStatus::Ok`] on success.
/// Release the returned buffer with [`aozora_bytes_free`].
///
/// # Safety
/// A non-null `out_version` must reference a writable, aligned [`AozoraBytes`] slot.
#[unsafe(no_mangle)]
#[expect(
    unsafe_code,
    reason = "the C ABI requires exported symbols and raw pointer access"
)]
pub unsafe extern "C" fn aozora_version(out_version: *mut AozoraBytes) -> c_int {
    if out_version.is_null() {
        return AozoraStatus::NullInput as c_int;
    }
    let bytes = into_owned_bytes(env!("CARGO_PKG_VERSION").as_bytes().to_vec());
    // SAFETY: the caller provides a writable, aligned `out_version` slot.
    unsafe { out_version.write(bytes) };
    AozoraStatus::Ok as c_int
}

/// Returns the structured wire schema version without accessing caller memory.
#[unsafe(no_mangle)]
#[expect(
    unsafe_code,
    reason = "the C ABI requires exported symbols and raw pointer access"
)]
pub extern "C" fn aozora_schema_version() -> u32 {
    AOZORA_SCHEMA_VERSION
}

/// Parses a UTF-8 byte slice and writes an owned handle to `out_doc` on success.
/// Returns [`AozoraStatus::NullInput`] for null pointers, [`AozoraStatus::InvalidUtf8`] for invalid encoding, or [`AozoraStatus::SourceTooLarge`] for a length exceeding [`u32::MAX`].
/// A failure sets a non-null `out_doc` to null.
/// Release a successful handle with [`aozora_document_free`].
///
/// # Safety
/// For accepted source lengths, a non-null `src_ptr` must reference `src_len` readable bytes in one live allocation, with length at most `isize::MAX`.
/// A non-null `out_doc` must reference a writable, aligned pointer slot.
#[unsafe(no_mangle)]
#[expect(
    unsafe_code,
    reason = "the C ABI requires exported symbols and raw pointer access"
)]
pub unsafe extern "C" fn aozora_document_new(
    src_ptr: *const u8,
    src_len: usize,
    out_doc: *mut *mut AozoraDocument,
) -> c_int {
    if out_doc.is_null() {
        return AozoraStatus::NullInput as c_int;
    }
    // SAFETY: `out_doc` is non-null and the caller provides writable, aligned storage.
    unsafe { out_doc.write(core::ptr::null_mut()) };
    if src_ptr.is_null() {
        return AozoraStatus::NullInput as c_int;
    }
    if u32::try_from(src_len).is_err() {
        return AozoraStatus::SourceTooLarge as c_int;
    }
    // SAFETY: the caller provides one live allocation of `src_len` readable bytes within `isize::MAX` at the checked non-null pointer.
    let bytes = unsafe { slice::from_raw_parts(src_ptr, src_len) };
    let Ok(source_str) = core::str::from_utf8(bytes) else {
        return AozoraStatus::InvalidUtf8 as c_int;
    };
    let doc = Box::new(AozoraDocument {
        inner: aozora::parse(source_str.to_owned()).expect("source fits parser span limit"),
    });
    // SAFETY: the caller provides a writable, aligned `out_doc` slot.
    unsafe { out_doc.write(Box::into_raw(doc)) };
    AozoraStatus::Ok as c_int
}

/// Writes rendered HTML to `out_html`.
/// Returns [`AozoraStatus::NullInput`] for null pointers, or [`AozoraStatus::Ok`] on success.
/// Release the returned buffer with [`aozora_bytes_free`].
///
/// # Safety
/// `doc` must be null or an unfreed handle from [`aozora_document_new`].
/// Callers must serialize access and destruction.
/// A non-null `out_html` must reference a writable, aligned [`AozoraBytes`] slot.
#[unsafe(no_mangle)]
#[expect(
    unsafe_code,
    reason = "the C ABI requires exported symbols and raw pointer access"
)]
pub unsafe extern "C" fn aozora_document_to_html(
    doc: *const AozoraDocument,
    out_html: *mut AozoraBytes,
) -> c_int {
    if doc.is_null() || out_html.is_null() {
        return AozoraStatus::NullInput as c_int;
    }
    // SAFETY: the caller provides a live document handle and serializes calls.
    let doc_ref: &AozoraDocument = unsafe { &*doc };
    let html = doc_ref.inner.snapshot().to_html();
    let bytes = into_owned_bytes(html.into_bytes());
    // SAFETY: the caller provides a writable, aligned `out_html` slot.
    unsafe { out_html.write(bytes) };
    AozoraStatus::Ok as c_int
}

/// Writes diagnostics as a JSON envelope to `out_json`.
/// The wire format follows [`aozora::json::diagnostics`].
/// Returns [`AozoraStatus::NullInput`] for null pointers, or [`AozoraStatus::Ok`] on success.
/// Release the returned buffer with [`aozora_bytes_free`].
///
/// # Safety
/// `doc` must be null or an unfreed handle from [`aozora_document_new`].
/// Callers must serialize access and destruction.
/// A non-null `out_json` must reference a writable, aligned [`AozoraBytes`] slot.
#[unsafe(no_mangle)]
#[expect(
    unsafe_code,
    reason = "the C ABI requires exported symbols and raw pointer access"
)]
pub unsafe extern "C" fn aozora_document_diagnostics_json(
    doc: *const AozoraDocument,
    out_json: *mut AozoraBytes,
) -> c_int {
    if doc.is_null() || out_json.is_null() {
        return AozoraStatus::NullInput as c_int;
    }
    // SAFETY: the caller provides a live document handle and serializes calls.
    let doc_ref: &AozoraDocument = unsafe { &*doc };
    let tree = doc_ref.inner.snapshot();
    let json = aozora::json::diagnostics(tree.diagnostics());
    let owned = into_owned_bytes(json.into_bytes());
    // SAFETY: the caller provides a writable, aligned `out_json` slot.
    unsafe { out_json.write(owned) };
    AozoraStatus::Ok as c_int
}

/// Writes a plain-text diagnostic report to `out_text`.
/// A clean parse produces an empty buffer.
/// The report follows [`aozora::diagnostics_text`].
/// Returns [`AozoraStatus::NullInput`] for null pointers, or [`AozoraStatus::Ok`] on success.
/// Release the returned buffer with [`aozora_bytes_free`].
///
/// # Safety
/// `doc` must be null or an unfreed handle from [`aozora_document_new`].
/// Callers must serialize access and destruction.
/// A non-null `out_text` must reference a writable, aligned [`AozoraBytes`] slot.
#[unsafe(no_mangle)]
#[expect(
    unsafe_code,
    reason = "the C ABI requires exported symbols and raw pointer access"
)]
pub unsafe extern "C" fn aozora_document_diagnostics_text(
    doc: *const AozoraDocument,
    out_text: *mut AozoraBytes,
) -> c_int {
    if doc.is_null() || out_text.is_null() {
        return AozoraStatus::NullInput as c_int;
    }
    // SAFETY: the caller provides a live document handle and serializes calls.
    let doc_ref: &AozoraDocument = unsafe { &*doc };
    let tree = doc_ref.inner.snapshot();
    let text = aozora::diagnostics_text(doc_ref.inner.source(), tree.diagnostics());
    let owned = into_owned_bytes(text.into_bytes());
    // SAFETY: the caller provides a writable, aligned `out_text` slot.
    unsafe { out_text.write(owned) };
    AozoraStatus::Ok as c_int
}

/// Writes nodes as a JSON envelope to `out_json`.
/// Entries use original-source byte spans in source order, following [`aozora::json::nodes`].
/// Returns [`AozoraStatus::NullInput`] for null pointers, or [`AozoraStatus::Ok`] on success.
/// Release the returned buffer with [`aozora_bytes_free`].
///
/// # Safety
/// `doc` must be null or an unfreed handle from [`aozora_document_new`].
/// Callers must serialize access and destruction.
/// A non-null `out_json` must reference a writable, aligned [`AozoraBytes`] slot.
#[unsafe(no_mangle)]
#[expect(
    unsafe_code,
    reason = "the C ABI requires exported symbols and raw pointer access"
)]
pub unsafe extern "C" fn aozora_document_nodes_json(
    doc: *const AozoraDocument,
    out_json: *mut AozoraBytes,
) -> c_int {
    if doc.is_null() || out_json.is_null() {
        return AozoraStatus::NullInput as c_int;
    }
    // SAFETY: the caller provides a live document handle and serializes calls.
    let doc_ref: &AozoraDocument = unsafe { &*doc };
    let tree = doc_ref.inner.snapshot();
    let json = aozora::json::nodes(&tree);
    let owned = into_owned_bytes(json.into_bytes());
    // SAFETY: the caller provides a writable, aligned `out_json` slot.
    unsafe { out_json.write(owned) };
    AozoraStatus::Ok as c_int
}

/// Writes matched delimiters as a JSON envelope to `out_json`.
/// Entries use original-source byte spans, following [`aozora::json::pairs`].
/// Returns [`AozoraStatus::NullInput`] for null pointers, or [`AozoraStatus::Ok`] on success.
/// Release the returned buffer with [`aozora_bytes_free`].
///
/// # Safety
/// `doc` must be null or an unfreed handle from [`aozora_document_new`].
/// Callers must serialize access and destruction.
/// A non-null `out_json` must reference a writable, aligned [`AozoraBytes`] slot.
#[unsafe(no_mangle)]
#[expect(
    unsafe_code,
    reason = "the C ABI requires exported symbols and raw pointer access"
)]
pub unsafe extern "C" fn aozora_document_pairs_json(
    doc: *const AozoraDocument,
    out_json: *mut AozoraBytes,
) -> c_int {
    if doc.is_null() || out_json.is_null() {
        return AozoraStatus::NullInput as c_int;
    }
    // SAFETY: the caller provides a live document handle and serializes calls.
    let doc_ref: &AozoraDocument = unsafe { &*doc };
    let tree = doc_ref.inner.snapshot();
    let json = aozora::json::pairs(&tree);
    let owned = into_owned_bytes(json.into_bytes());
    // SAFETY: the caller provides a writable, aligned `out_json` slot.
    unsafe { out_json.write(owned) };
    AozoraStatus::Ok as c_int
}

/// Writes canonical Aozora source to `out_source`.
/// Returns [`AozoraStatus::NullInput`] for null pointers, or [`AozoraStatus::Ok`] on success.
/// Release the returned buffer with [`aozora_bytes_free`].
///
/// # Safety
/// `doc` must be null or an unfreed handle from [`aozora_document_new`].
/// Callers must serialize access and destruction.
/// A non-null `out_source` must reference a writable, aligned [`AozoraBytes`] slot.
#[unsafe(no_mangle)]
#[expect(
    unsafe_code,
    reason = "the C ABI requires exported symbols and raw pointer access"
)]
pub unsafe extern "C" fn aozora_document_to_source(
    doc: *const AozoraDocument,
    out_source: *mut AozoraBytes,
) -> c_int {
    if doc.is_null() || out_source.is_null() {
        return AozoraStatus::NullInput as c_int;
    }
    // SAFETY: the caller provides a live document handle and serializes calls.
    let doc_ref: &AozoraDocument = unsafe { &*doc };
    let source = doc_ref.inner.snapshot().to_source();
    let bytes = into_owned_bytes(source.into_bytes());
    // SAFETY: the caller provides a writable, aligned `out_source` slot.
    unsafe { out_source.write(bytes) };
    AozoraStatus::Ok as c_int
}

/// Writes matched container delimiters as a JSON envelope to `out_json`.
/// Entries use original-source byte spans, following [`aozora::json::container_pairs`].
/// Returns [`AozoraStatus::NullInput`] for null pointers, or [`AozoraStatus::Ok`] on success.
/// Release the returned buffer with [`aozora_bytes_free`].
///
/// # Safety
/// `doc` must be null or an unfreed handle from [`aozora_document_new`].
/// Callers must serialize access and destruction.
/// A non-null `out_json` must reference a writable, aligned [`AozoraBytes`] slot.
#[unsafe(no_mangle)]
#[expect(
    unsafe_code,
    reason = "the C ABI requires exported symbols and raw pointer access"
)]
pub unsafe extern "C" fn aozora_document_container_pairs_json(
    doc: *const AozoraDocument,
    out_json: *mut AozoraBytes,
) -> c_int {
    if doc.is_null() || out_json.is_null() {
        return AozoraStatus::NullInput as c_int;
    }
    // SAFETY: the caller provides a live document handle and serializes calls.
    let doc_ref: &AozoraDocument = unsafe { &*doc };
    let tree = doc_ref.inner.snapshot();
    let json = aozora::json::container_pairs(&tree);
    let owned = into_owned_bytes(json.into_bytes());
    // SAFETY: the caller provides a writable, aligned `out_json` slot.
    unsafe { out_json.write(owned) };
    AozoraStatus::Ok as c_int
}

/// Writes gaiji resolutions as a JSON envelope to `out_json`.
/// Entries follow [`aozora::json::gaiji`].
/// Returns [`AozoraStatus::NullInput`] for null pointers, or [`AozoraStatus::Ok`] on success.
/// Release the returned buffer with [`aozora_bytes_free`].
///
/// # Safety
/// `doc` must be null or an unfreed handle from [`aozora_document_new`].
/// Callers must serialize access and destruction.
/// A non-null `out_json` must reference a writable, aligned [`AozoraBytes`] slot.
#[unsafe(no_mangle)]
#[expect(
    unsafe_code,
    reason = "the C ABI requires exported symbols and raw pointer access"
)]
pub unsafe extern "C" fn aozora_document_gaiji_json(
    doc: *const AozoraDocument,
    out_json: *mut AozoraBytes,
) -> c_int {
    if doc.is_null() || out_json.is_null() {
        return AozoraStatus::NullInput as c_int;
    }
    // SAFETY: the caller provides a live document handle and serializes calls.
    let doc_ref: &AozoraDocument = unsafe { &*doc };
    let json = aozora::json::gaiji(&doc_ref.inner.snapshot());
    let owned = into_owned_bytes(json.into_bytes());
    // SAFETY: the caller provides a writable, aligned `out_json` slot.
    unsafe { out_json.write(owned) };
    AozoraStatus::Ok as c_int
}

/// Writes the owned source's UTF-8 byte length to `out_len`.
/// Returns [`AozoraStatus::NullInput`] for null pointers, or [`AozoraStatus::Ok`] on success.
///
/// # Safety
/// `doc` must be null or an unfreed handle from [`aozora_document_new`].
/// Callers must serialize access and destruction.
/// A non-null `out_len` must reference a writable, aligned `usize` slot.
#[unsafe(no_mangle)]
#[expect(
    unsafe_code,
    reason = "the C ABI requires exported symbols and raw pointer access"
)]
pub unsafe extern "C" fn aozora_document_source_byte_len(
    doc: *const AozoraDocument,
    out_len: *mut usize,
) -> c_int {
    if doc.is_null() || out_len.is_null() {
        return AozoraStatus::NullInput as c_int;
    }
    // SAFETY: the caller provides a live document handle and serializes calls.
    let doc_ref: &AozoraDocument = unsafe { &*doc };
    let len = doc_ref.inner.source().len();
    // SAFETY: the caller provides a writable, aligned `out_len` slot.
    unsafe { out_len.write(len) };
    AozoraStatus::Ok as c_int
}

/// Writes the canonical slug catalogue as a JSON envelope to `out_json`.
/// The wire format follows [`aozora::json::slugs`].
/// Returns [`AozoraStatus::NullInput`] for a null output pointer, or [`AozoraStatus::Ok`] on success.
/// Release the returned buffer with [`aozora_bytes_free`].
///
/// # Safety
/// A non-null `out_json` must reference a writable, aligned [`AozoraBytes`] slot.
#[unsafe(no_mangle)]
#[expect(
    unsafe_code,
    reason = "the C ABI requires exported symbols and raw pointer access"
)]
pub unsafe extern "C" fn aozora_slugs_json(out_json: *mut AozoraBytes) -> c_int {
    if out_json.is_null() {
        return AozoraStatus::NullInput as c_int;
    }
    let json = aozora::json::slugs();
    let owned = into_owned_bytes(json.into_bytes());
    // SAFETY: the caller provides a writable, aligned `out_json` slot.
    unsafe { out_json.write(owned) };
    AozoraStatus::Ok as c_int
}

/// Releases a document handle, doing nothing for null.
///
/// # Safety
/// A non-null `doc` must be an unfreed handle returned by [`aozora_document_new`].
/// No other call can access the handle during or after release.
#[unsafe(no_mangle)]
#[expect(
    unsafe_code,
    reason = "the C ABI requires exported symbols and raw pointer access"
)]
pub unsafe extern "C" fn aozora_document_free(doc: *mut AozoraDocument) {
    if !doc.is_null() {
        // SAFETY: the caller provides `doc` as an unfreed handle from `aozora_document_new`.
        drop(unsafe { Box::from_raw(doc) });
    }
}

/// Releases a byte buffer, doing nothing for a null allocation pointer.
///
/// # Safety
/// A non-null buffer must be an unchanged, unfreed value returned by this library.
/// No reference to its allocation can remain in use during or after release.
#[unsafe(no_mangle)]
#[expect(
    unsafe_code,
    reason = "the C ABI requires exported symbols and raw pointer access"
)]
pub unsafe extern "C" fn aozora_bytes_free(bytes: AozoraBytes) {
    if !bytes.ptr.is_null() {
        // SAFETY: the caller supplies the unchanged, unfreed allocation triple produced by `into_owned_bytes`.
        drop(unsafe { Vec::from_raw_parts(bytes.ptr, bytes.len, bytes.cap) });
    }
}

#[expect(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[expect(
        unsafe_code,
        reason = "exercises the C ABI ownership and pointer contract"
    )]
    fn version_and_schema_are_available_through_the_abi() {
        let mut version = AozoraBytes {
            ptr: core::ptr::null_mut(),
            len: 0,
            cap: 0,
        };
        let status = unsafe { aozora_version(&mut version) };
        assert_eq!(status, AozoraStatus::Ok as c_int);
        let value =
            unsafe { core::str::from_utf8(slice::from_raw_parts(version.ptr, version.len)) }
                .expect("version is UTF-8");
        assert_eq!(value, env!("CARGO_PKG_VERSION"));
        assert_eq!(aozora_schema_version(), AOZORA_SCHEMA_VERSION);
        unsafe { aozora_bytes_free(version) };
    }

    #[test]
    #[expect(
        unsafe_code,
        reason = "exercises the C ABI ownership and pointer contract"
    )]
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
        assert_eq!(
            json,
            format!(r#"{{"schemaVersion":{},"data":[]}}"#, AOZORA_SCHEMA_VERSION)
        );
        unsafe { aozora_bytes_free(diag) };

        unsafe { aozora_document_free(doc) };
    }

    #[test]
    #[expect(
        unsafe_code,
        reason = "exercises the C ABI ownership and pointer contract"
    )]
    fn nodes_json_is_empty_envelope_for_plain_input() {
        let src = b"plain";
        let mut doc: *mut AozoraDocument = core::ptr::null_mut();
        let status = unsafe { aozora_document_new(src.as_ptr(), src.len(), &mut doc) };
        assert_eq!(status, AozoraStatus::Ok as c_int);
        let mut nodes = AozoraBytes {
            ptr: core::ptr::null_mut(),
            len: 0,
            cap: 0,
        };
        let status = unsafe { aozora_document_nodes_json(doc, &mut nodes) };
        assert_eq!(status, AozoraStatus::Ok as c_int);
        let json = unsafe { core::str::from_utf8(slice::from_raw_parts(nodes.ptr, nodes.len)) }
            .expect("json is utf8");
        assert_eq!(
            json,
            format!(r#"{{"schemaVersion":{},"data":[]}}"#, AOZORA_SCHEMA_VERSION)
        );
        unsafe { aozora_bytes_free(nodes) };
        unsafe { aozora_document_free(doc) };
    }

    #[test]
    #[expect(
        unsafe_code,
        reason = "exercises the C ABI ownership and pointer contract"
    )]
    fn nodes_json_emits_ruby_kind_for_ruby_input() {
        let src = "｜青梅《おうめ》".as_bytes();
        let mut doc: *mut AozoraDocument = core::ptr::null_mut();
        let status = unsafe { aozora_document_new(src.as_ptr(), src.len(), &mut doc) };
        assert_eq!(status, AozoraStatus::Ok as c_int);
        let mut nodes = AozoraBytes {
            ptr: core::ptr::null_mut(),
            len: 0,
            cap: 0,
        };
        let status = unsafe { aozora_document_nodes_json(doc, &mut nodes) };
        assert_eq!(status, AozoraStatus::Ok as c_int);
        let json = unsafe { core::str::from_utf8(slice::from_raw_parts(nodes.ptr, nodes.len)) }
            .expect("json is utf8");
        assert!(json.contains(r#""kind":"ruby""#), "nodes json: {json}");
        unsafe { aozora_bytes_free(nodes) };
        unsafe { aozora_document_free(doc) };
    }

    #[test]
    #[expect(
        unsafe_code,
        reason = "exercises the C ABI ownership and pointer contract"
    )]
    fn pairs_json_emits_ruby_pair_for_ruby_input() {
        let src = "｜青梅《おうめ》".as_bytes();
        let mut doc: *mut AozoraDocument = core::ptr::null_mut();
        let status = unsafe { aozora_document_new(src.as_ptr(), src.len(), &mut doc) };
        assert_eq!(status, AozoraStatus::Ok as c_int);
        let mut pairs = AozoraBytes {
            ptr: core::ptr::null_mut(),
            len: 0,
            cap: 0,
        };
        let status = unsafe { aozora_document_pairs_json(doc, &mut pairs) };
        assert_eq!(status, AozoraStatus::Ok as c_int);
        let json = unsafe { core::str::from_utf8(slice::from_raw_parts(pairs.ptr, pairs.len)) }
            .expect("json is utf8");
        assert!(json.contains(r#""kind":"ruby""#), "pairs json: {json}");
        assert!(json.contains(r#""open":"#), "pairs json: {json}");
        assert!(json.contains(r#""close":"#), "pairs json: {json}");
        unsafe { aozora_bytes_free(pairs) };
        unsafe { aozora_document_free(doc) };
    }

    #[test]
    #[expect(
        unsafe_code,
        reason = "exercises the C ABI ownership and pointer contract"
    )]
    fn to_source_round_trips_ruby_input() {
        let src = "｜青梅《おうめ》";
        let mut doc: *mut AozoraDocument = core::ptr::null_mut();
        let status = unsafe { aozora_document_new(src.as_ptr(), src.len(), &mut doc) };
        assert_eq!(status, AozoraStatus::Ok as c_int);
        let mut out = AozoraBytes {
            ptr: core::ptr::null_mut(),
            len: 0,
            cap: 0,
        };
        let status = unsafe { aozora_document_to_source(doc, &mut out) };
        assert_eq!(status, AozoraStatus::Ok as c_int);
        let got = unsafe { core::str::from_utf8(slice::from_raw_parts(out.ptr, out.len)) }
            .expect("source is utf8");
        let want = aozora::parse(src.to_owned())
            .expect("source fits parser span limit")
            .snapshot()
            .to_source();
        assert_eq!(got, want, "to_source drift");
        unsafe { aozora_bytes_free(out) };
        unsafe { aozora_document_free(doc) };
    }

    #[test]
    #[expect(
        unsafe_code,
        reason = "exercises the C ABI ownership and pointer contract"
    )]
    fn container_pairs_json_matches_wire_authority() {
        let src = "plain";
        let mut doc: *mut AozoraDocument = core::ptr::null_mut();
        let status = unsafe { aozora_document_new(src.as_ptr(), src.len(), &mut doc) };
        assert_eq!(status, AozoraStatus::Ok as c_int);
        let mut out = AozoraBytes {
            ptr: core::ptr::null_mut(),
            len: 0,
            cap: 0,
        };
        let status = unsafe { aozora_document_container_pairs_json(doc, &mut out) };
        assert_eq!(status, AozoraStatus::Ok as c_int);
        let got = unsafe { core::str::from_utf8(slice::from_raw_parts(out.ptr, out.len)) }
            .expect("json is utf8");
        let want = aozora::json::container_pairs(
            &aozora::parse(src.to_owned())
                .expect("source fits parser span limit")
                .snapshot(),
        );
        assert_eq!(got, want, "container_pairs_json drift");
        unsafe { aozora_bytes_free(out) };
        unsafe { aozora_document_free(doc) };
    }

    #[test]
    #[expect(
        unsafe_code,
        reason = "exercises the C ABI ownership and pointer contract"
    )]
    fn gaiji_json_matches_wire_authority() {
        let src = "※［＃「魚＋更」、第4水準2-93-32］";
        let mut doc: *mut AozoraDocument = core::ptr::null_mut();
        let status = unsafe { aozora_document_new(src.as_ptr(), src.len(), &mut doc) };
        assert_eq!(status, AozoraStatus::Ok as c_int);
        let mut out = AozoraBytes {
            ptr: core::ptr::null_mut(),
            len: 0,
            cap: 0,
        };
        let status = unsafe { aozora_document_gaiji_json(doc, &mut out) };
        assert_eq!(status, AozoraStatus::Ok as c_int);
        let got = unsafe { core::str::from_utf8(slice::from_raw_parts(out.ptr, out.len)) }
            .expect("json is utf8");
        let expected = aozora::parse(src).expect("source is within parser limit");
        let want = aozora::json::gaiji(&expected.snapshot());
        assert_eq!(got, want, "gaiji_json drift");
        unsafe { aozora_bytes_free(out) };
        unsafe { aozora_document_free(doc) };
    }

    #[test]
    #[expect(
        unsafe_code,
        reason = "exercises the C ABI ownership and pointer contract"
    )]
    fn source_byte_len_reports_utf8_length() {
        let src = "｜青梅《おうめ》";
        let mut doc: *mut AozoraDocument = core::ptr::null_mut();
        let status = unsafe { aozora_document_new(src.as_ptr(), src.len(), &mut doc) };
        assert_eq!(status, AozoraStatus::Ok as c_int);
        let mut len: usize = 0;
        let status = unsafe { aozora_document_source_byte_len(doc, &mut len) };
        assert_eq!(status, AozoraStatus::Ok as c_int);
        assert_eq!(len, src.len());
        unsafe { aozora_document_free(doc) };
    }

    #[test]
    #[expect(
        unsafe_code,
        reason = "exercises the C ABI ownership and pointer contract"
    )]
    fn slugs_json_matches_wire_authority() {
        let mut out = AozoraBytes {
            ptr: core::ptr::null_mut(),
            len: 0,
            cap: 0,
        };
        let status = unsafe { aozora_slugs_json(&mut out) };
        assert_eq!(status, AozoraStatus::Ok as c_int);
        let got = unsafe { core::str::from_utf8(slice::from_raw_parts(out.ptr, out.len)) }
            .expect("json is utf8");
        assert_eq!(got, aozora::json::slugs(), "slugs_json drift");
        unsafe { aozora_bytes_free(out) };
    }

    #[test]
    #[expect(
        unsafe_code,
        reason = "exercises the C ABI ownership and pointer contract"
    )]
    fn null_input_returns_null_input_status() {
        let mut doc = core::ptr::NonNull::<AozoraDocument>::dangling().as_ptr();
        let status = unsafe { aozora_document_new(core::ptr::null(), 0, &mut doc) };
        assert_eq!(status, AozoraStatus::NullInput as c_int);
        assert!(doc.is_null());
    }

    #[test]
    #[expect(
        unsafe_code,
        reason = "exercises the C ABI ownership and pointer contract"
    )]
    fn invalid_utf8_returns_invalid_utf8_status() {
        let bad = [0xFFu8, 0xFE, 0xFD];
        let mut doc = core::ptr::NonNull::<AozoraDocument>::dangling().as_ptr();
        let status = unsafe { aozora_document_new(bad.as_ptr(), bad.len(), &mut doc) };
        assert_eq!(status, AozoraStatus::InvalidUtf8 as c_int);
        assert!(doc.is_null());
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    #[expect(
        unsafe_code,
        reason = "exercises the C ABI ownership and pointer contract"
    )]
    fn oversize_length_returns_source_too_large_status() {
        let dangling = core::ptr::NonNull::<u8>::dangling().as_ptr().cast_const();
        let oversize = u32::MAX as usize + 1;
        let mut doc = core::ptr::NonNull::<AozoraDocument>::dangling().as_ptr();
        let status = unsafe { aozora_document_new(dangling, oversize, &mut doc) };
        assert_eq!(status, AozoraStatus::SourceTooLarge as c_int);
        assert!(doc.is_null(), "oversize input must not yield a handle");
    }

    #[test]
    #[expect(
        unsafe_code,
        reason = "exercises the C ABI ownership and pointer contract"
    )]
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
    #[expect(
        unsafe_code,
        reason = "exercises the C ABI ownership and pointer contract"
    )]
    fn freeing_null_handle_is_safe_noop() {
        unsafe { aozora_document_free(core::ptr::null_mut()) };
    }

    #[test]
    fn status_discriminants_are_abi_stable() {
        assert_eq!(AozoraStatus::Ok as i32, 0);
        assert_eq!(AozoraStatus::NullInput as i32, -1);
        assert_eq!(AozoraStatus::InvalidUtf8 as i32, -2);
        assert_eq!(AozoraStatus::AllocFailed as i32, -3);
        assert_eq!(AozoraStatus::SerializeFailed as i32, -4);
        assert_eq!(AozoraStatus::SourceTooLarge as i32, -5);
    }
}
