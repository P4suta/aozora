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
//!
//! ## Panic / abort contract for embedders
//!
//! The workspace release profile is compiled with `panic = "abort"`.
//! A Rust panic therefore does **not** unwind across the C ABI — it
//! terminates the **entire host process** via `abort()`. There is no
//! `catch_unwind` net here, and none is possible under `panic =
//! "abort"`. Concretely:
//!
//! - Every `aozora_*` function validates its pointer / length / UTF-8
//!   preconditions up front and reports problems through the
//!   [`AozoraStatus`] return code. On those paths it never panics.
//! - In particular, inputs whose byte length exceeds [`u32::MAX`]
//!   (~4 GiB) are rejected at [`aozora_document_new`] with
//!   [`AozoraStatus::SourceTooLarge`] *before* the parser core runs,
//!   because the core asserts `len <= u32::MAX` (its span offsets are
//!   `u32`) and would otherwise abort the host process.
//! - Embedders MUST pre-validate untrusted input and MUST NOT rely on
//!   catching a Rust panic to recover: by the time a panic reaches the
//!   ABI the process is already being torn down. Treat a non-zero
//!   status as the only supported error channel.

#![allow(
    unsafe_code,
    reason = "C ABI surface inherently requires unsafe blocks (extern \"C\", raw pointers)"
)]

use core::ffi::c_int;
use core::slice;

/// Wire schema version carried by every structured JSON result.
pub const AOZORA_SCHEMA_VERSION: u32 = aozora::json::SCHEMA_VERSION;

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
    /// The input byte length exceeded [`u32::MAX`] (~4 GiB). The parser
    /// core uses `u32` span offsets and asserts this bound; rejecting
    /// here keeps the assert (and, under `panic = "abort"`, the whole
    /// host process) from firing. See the crate-level "Panic / abort
    /// contract" docs.
    SourceTooLarge = -5,
}

/// Opaque handle to a parsed Aozora document. Allocate via
/// [`aozora_document_new`]; free via [`aozora_document_free`].
///
/// Wraps an [`aozora::Document`] and its immutable parsed snapshot. The
/// C ABI treats it as a single-owner handle — embedders may move it
/// across threads but must not call into one handle concurrently from
/// multiple threads without external synchronisation.
#[derive(Debug)]
pub struct AozoraDocument {
    inner: aozora::Document,
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
    /// Pointer to the start of the owned allocation. Null only on the
    /// caller-zeroed sentinel value (which [`aozora_bytes_free`] treats
    /// as a no-op); on any value returned by an `aozora_*` function it
    /// is non-null and valid for `cap` bytes.
    pub ptr: *mut u8,
    /// Number of initialised, readable bytes — the length of the
    /// returned payload (HTML or JSON). May be less than `cap`.
    pub len: usize,
    /// Total capacity of the allocation, in bytes. Must be passed back
    /// unchanged to [`aozora_bytes_free`]: it reconstructs the `Vec`
    /// with `(ptr, len, cap)`, so a wrong `cap` corrupts the allocator.
    pub cap: usize,
}

/// Construct a [`Document`](AozoraDocument) from a UTF-8 byte slice.
///
/// On success, writes the document handle to `*out_doc` and returns
/// [`AozoraStatus::Ok`]. On failure, writes `null` to `*out_doc` and
/// returns the matching error status.
///
/// Inputs whose byte length exceeds [`u32::MAX`] are rejected with
/// [`AozoraStatus::SourceTooLarge`] *before* the parser core runs (the
/// core's span offsets are `u32` and it asserts that bound; under
/// `panic = "abort"` hitting the assert would abort the whole host
/// process). Guarding at construction means no oversize document can
/// exist, so the later `aozora_document_*` accessors never reach the
/// assert either. See the crate-level "Panic / abort contract" docs.
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
    if out_doc.is_null() {
        return AozoraStatus::NullInput as c_int;
    }
    // SAFETY: out_doc is non-null (checked above) and the caller
    // guarantees it is writable.
    unsafe { out_doc.write(core::ptr::null_mut()) };
    if src_ptr.is_null() {
        return AozoraStatus::NullInput as c_int;
    }
    // Reject oversize input before touching the parser core: its span
    // offsets are `u32` and it asserts `len <= u32::MAX`, which under
    // `panic = "abort"` would abort the host process. We can fail fast
    // on `src_len` alone — it is the source byte length and the parser
    // never grows the buffer past it.
    if u32::try_from(src_len).is_err() {
        return AozoraStatus::SourceTooLarge as c_int;
    }
    // SAFETY: caller guarantees src_ptr + src_len name a valid byte
    // slice. src_ptr is non-null (checked above) and src_len <= u32::MAX
    // (checked above) < isize::MAX, so the slice-length precondition of
    // `from_raw_parts` is satisfied.
    let bytes = unsafe { slice::from_raw_parts(src_ptr, src_len) };
    let Ok(source_str) = core::str::from_utf8(bytes) else {
        return AozoraStatus::InvalidUtf8 as c_int;
    };
    let doc = Box::new(AozoraDocument {
        inner: aozora::parse(source_str.to_owned()).expect("source fits parser span limit"),
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
    let html = doc_ref.inner.snapshot().to_html();
    let bytes = into_owned_bytes(html.into_bytes());
    // SAFETY: caller guarantees out_html is writable.
    unsafe { out_html.write(bytes) };
    AozoraStatus::Ok as c_int
}

/// Render the document's diagnostics as a JSON byte buffer.
///
/// On success, writes the bytes to `*out_json` and returns
/// [`AozoraStatus::Ok`]. Empty document → the empty envelope
/// `{"schemaVersion":…,"data":[]}` (version is
/// [`aozora::json::SCHEMA_VERSION`]). The caller MUST call
/// [`aozora_bytes_free`] on the returned [`AozoraBytes`].
///
/// Wire format is defined in [`aozora::json`] and shared bit-for-bit
/// with the WASM and PyO3 drivers.
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
    let tree = doc_ref.inner.snapshot();
    let json = aozora::json::diagnostics(tree.diagnostics());
    let owned = into_owned_bytes(json.into_bytes());
    // SAFETY: caller guarantees out_json is writable.
    unsafe { out_json.write(owned) };
    AozoraStatus::Ok as c_int
}

/// Render the document's diagnostics as a plain-text byte buffer.
///
/// One block per diagnostic — `<severity> [<code>] @ <start>..<end>:
/// <message>` plus the offending source slice. This is the `miette`-free
/// portable rendering ([`aozora::diagnostics_text`]); a clean parse
/// yields an empty buffer. For the machine-readable view use
/// [`aozora_document_diagnostics_json`].
///
/// On success, writes the bytes to `*out_text` and returns
/// [`AozoraStatus::Ok`]. The caller MUST call [`aozora_bytes_free`] on
/// the returned [`AozoraBytes`].
///
/// # Safety
///
/// - `doc` must be a non-null handle produced by
///   [`aozora_document_new`] and not yet freed.
/// - `out_text` must point to a writable [`AozoraBytes`] slot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aozora_document_diagnostics_text(
    doc: *const AozoraDocument,
    out_text: *mut AozoraBytes,
) -> c_int {
    if doc.is_null() || out_text.is_null() {
        return AozoraStatus::NullInput as c_int;
    }
    // SAFETY: caller guarantees doc is a valid handle.
    let doc_ref: &AozoraDocument = unsafe { &*doc };
    let tree = doc_ref.inner.snapshot();
    let text = aozora::diagnostics_text(doc_ref.inner.source(), tree.diagnostics());
    let owned = into_owned_bytes(text.into_bytes());
    // SAFETY: caller guarantees out_text is writable.
    unsafe { out_text.write(owned) };
    AozoraStatus::Ok as c_int
}

/// Render the document's source-keyed Aozora nodes as a JSON byte
/// buffer.
///
/// Each entry has the shape `{ kind, span: { start, end } }` in source
/// coordinates, sorted by `span.start`. Useful for editor integrations
/// (semantic tokens, document symbols, Lezer-Tree builders).
///
/// On success, writes the bytes to `*out_json` and returns
/// [`AozoraStatus::Ok`]. Empty parse → the empty envelope
/// `{"schemaVersion":…,"data":[]}` (version is
/// [`aozora::json::SCHEMA_VERSION`]). The caller MUST call
/// [`aozora_bytes_free`] on the returned [`AozoraBytes`].
///
/// Wire format is defined in [`aozora::json`] and shared bit-for-bit
/// with the WASM and PyO3 drivers.
///
/// # Safety
///
/// - `doc` must be a non-null handle produced by
///   [`aozora_document_new`] and not yet freed.
/// - `out_json` must point to a writable [`AozoraBytes`] slot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aozora_document_nodes_json(
    doc: *const AozoraDocument,
    out_json: *mut AozoraBytes,
) -> c_int {
    if doc.is_null() || out_json.is_null() {
        return AozoraStatus::NullInput as c_int;
    }
    // SAFETY: caller guarantees doc is a valid handle.
    let doc_ref: &AozoraDocument = unsafe { &*doc };
    let tree = doc_ref.inner.snapshot();
    let json = aozora::json::nodes(&tree);
    let owned = into_owned_bytes(json.into_bytes());
    // SAFETY: caller guarantees out_json is writable.
    unsafe { out_json.write(owned) };
    AozoraStatus::Ok as c_int
}

/// Render the document's matched open/close pair links as a JSON byte
/// buffer.
///
/// Each entry has the shape
/// `{ kind, open: { start, end }, close: { start, end } }` in
/// sanitized-source coordinates. Useful for LSP requests like
/// `textDocument/linkedEditingRange` and
/// `textDocument/documentHighlight`.
///
/// On success, writes the bytes to `*out_json` and returns
/// [`AozoraStatus::Ok`]. Empty parse → the empty envelope
/// `{"schemaVersion":…,"data":[]}` (version is
/// [`aozora::json::SCHEMA_VERSION`]). The caller MUST call
/// [`aozora_bytes_free`] on the returned [`AozoraBytes`].
///
/// Wire format is defined in [`aozora::json`] and shared bit-for-bit
/// with the WASM and PyO3 drivers.
///
/// # Safety
///
/// - `doc` must be a non-null handle produced by
///   [`aozora_document_new`] and not yet freed.
/// - `out_json` must point to a writable [`AozoraBytes`] slot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aozora_document_pairs_json(
    doc: *const AozoraDocument,
    out_json: *mut AozoraBytes,
) -> c_int {
    if doc.is_null() || out_json.is_null() {
        return AozoraStatus::NullInput as c_int;
    }
    // SAFETY: caller guarantees doc is a valid handle.
    let doc_ref: &AozoraDocument = unsafe { &*doc };
    let tree = doc_ref.inner.snapshot();
    let json = aozora::json::pairs(&tree);
    let owned = into_owned_bytes(json.into_bytes());
    // SAFETY: caller guarantees out_json is writable.
    unsafe { out_json.write(owned) };
    AozoraStatus::Ok as c_int
}

/// Re-emit the document as Aozora source text (round-trip
/// serialization), returning the result as an owned byte buffer.
///
/// Parses and walks the tree back to canonical source — the inverse of
/// [`aozora_document_to_html`]. This is the `to_source` surface shared
/// bit-for-bit with the WASM (`toSource`), PyO3 (`to_source`), and
/// Extism/Go drivers.
///
/// On success, writes the bytes to `*out_source` and returns
/// [`AozoraStatus::Ok`]. The caller MUST call [`aozora_bytes_free`] on
/// the returned [`AozoraBytes`] to release the memory.
///
/// # Safety
///
/// - `doc` must be a non-null handle produced by
///   [`aozora_document_new`] and not yet freed.
/// - `out_source` must point to a writable [`AozoraBytes`] slot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aozora_document_to_source(
    doc: *const AozoraDocument,
    out_source: *mut AozoraBytes,
) -> c_int {
    if doc.is_null() || out_source.is_null() {
        return AozoraStatus::NullInput as c_int;
    }
    // SAFETY: caller guarantees doc is a valid handle.
    let doc_ref: &AozoraDocument = unsafe { &*doc };
    let source = doc_ref.inner.snapshot().to_source();
    let bytes = into_owned_bytes(source.into_bytes());
    // SAFETY: caller guarantees out_source is writable.
    unsafe { out_source.write(bytes) };
    AozoraStatus::Ok as c_int
}

/// Render the document's matched container open/close pairs as a JSON
/// byte buffer.
///
/// Each entry has the shape
/// `{ kind, open: { start, end }, close: { start, end } }` in
/// sanitized-source coordinates, covering block-level enclosures
/// (indent / jisage / caption blocks) rather than the inline pairs of
/// [`aozora_document_pairs_json`].
///
/// On success, writes the bytes to `*out_json` and returns
/// [`AozoraStatus::Ok`]. Empty parse → the empty envelope
/// `{"schemaVersion":…,"data":[]}` (version is
/// [`aozora::json::SCHEMA_VERSION`]). The caller MUST call
/// [`aozora_bytes_free`] on the returned [`AozoraBytes`].
///
/// Wire format is defined in [`aozora::json`] and shared bit-for-bit
/// with the WASM, PyO3, and Extism/Go drivers.
///
/// # Safety
///
/// - `doc` must be a non-null handle produced by
///   [`aozora_document_new`] and not yet freed.
/// - `out_json` must point to a writable [`AozoraBytes`] slot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aozora_document_container_pairs_json(
    doc: *const AozoraDocument,
    out_json: *mut AozoraBytes,
) -> c_int {
    if doc.is_null() || out_json.is_null() {
        return AozoraStatus::NullInput as c_int;
    }
    // SAFETY: caller guarantees doc is a valid handle.
    let doc_ref: &AozoraDocument = unsafe { &*doc };
    let tree = doc_ref.inner.snapshot();
    let json = aozora::json::container_pairs(&tree);
    let owned = into_owned_bytes(json.into_bytes());
    // SAFETY: caller guarantees out_json is writable.
    unsafe { out_json.write(owned) };
    AozoraStatus::Ok as c_int
}

/// Render the document's resolved gaiji references (`※［＃…］`) as a
/// JSON byte buffer.
///
/// Each entry records the source span of a gaiji directive alongside its
/// resolved canonical form (JIS X 0213 men-ku-ten, Unicode scalar, …).
/// The scan runs over the document's source string directly.
///
/// On success, writes the bytes to `*out_json` and returns
/// [`AozoraStatus::Ok`]. A source with no gaiji → the empty envelope
/// `{"schemaVersion":…,"data":[]}` (version is
/// [`aozora::json::SCHEMA_VERSION`]). The caller MUST call
/// [`aozora_bytes_free`] on the returned [`AozoraBytes`].
///
/// Wire format is defined in [`aozora::json`]. Its records also drive
/// the WASM `gaiji` values and the Python, Extism, and Go projections.
///
/// # Safety
///
/// - `doc` must be a non-null handle produced by
///   [`aozora_document_new`] and not yet freed.
/// - `out_json` must point to a writable [`AozoraBytes`] slot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aozora_document_gaiji_json(
    doc: *const AozoraDocument,
    out_json: *mut AozoraBytes,
) -> c_int {
    if doc.is_null() || out_json.is_null() {
        return AozoraStatus::NullInput as c_int;
    }
    // SAFETY: caller guarantees doc is a valid handle.
    let doc_ref: &AozoraDocument = unsafe { &*doc };
    let json = aozora::json::gaiji(&doc_ref.inner.snapshot());
    let owned = into_owned_bytes(json.into_bytes());
    // SAFETY: caller guarantees out_json is writable.
    unsafe { out_json.write(owned) };
    AozoraStatus::Ok as c_int
}

/// Write the document's source byte length to `*out_len`.
///
/// This is the length of the UTF-8 source buffer the handle owns — the
/// same quantity the WASM (`sourceByteLen`) and PyO3 (`source_byte_len`)
/// drivers return. Hosts use it to size buffers and to convert the `u32`
/// source coordinates in the JSON projections back into their own string
/// indices.
///
/// On success, writes the length to `*out_len` and returns
/// [`AozoraStatus::Ok`].
///
/// # Safety
///
/// - `doc` must be a non-null handle produced by
///   [`aozora_document_new`] and not yet freed.
/// - `out_len` must point to a writable `usize` slot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aozora_document_source_byte_len(
    doc: *const AozoraDocument,
    out_len: *mut usize,
) -> c_int {
    if doc.is_null() || out_len.is_null() {
        return AozoraStatus::NullInput as c_int;
    }
    // SAFETY: caller guarantees doc is a valid handle.
    let doc_ref: &AozoraDocument = unsafe { &*doc };
    let len = doc_ref.inner.source().len();
    // SAFETY: caller guarantees out_len is writable.
    unsafe { out_len.write(len) };
    AozoraStatus::Ok as c_int
}

/// Render the spec's canonical slug catalogue as a JSON byte buffer.
///
/// This is document-independent — it projects the static slug table from
/// [`aozora::json::slugs`], the same authority behind the WASM
/// `slugs` and PyO3 `slugs_json` exports. Editor front ends use
/// it to drive directive completion.
///
/// On success, writes the bytes to `*out_json` and returns
/// [`AozoraStatus::Ok`]. The caller MUST call [`aozora_bytes_free`] on
/// the returned [`AozoraBytes`].
///
/// Wire format is defined in [`aozora::json`] and shared bit-for-bit
/// with PyO3 and Extism/Go.
///
/// # Safety
///
/// - `out_json` must point to a writable [`AozoraBytes`] slot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aozora_slugs_json(out_json: *mut AozoraBytes) -> c_int {
    if out_json.is_null() {
        return AozoraStatus::NullInput as c_int;
    }
    let json = aozora::json::slugs();
    let owned = into_owned_bytes(json.into_bytes());
    // SAFETY: caller guarantees out_json is writable.
    unsafe { out_json.write(owned) };
    AozoraStatus::Ok as c_int
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
        assert_eq!(
            json,
            format!(r#"{{"schemaVersion":{},"data":[]}}"#, AOZORA_SCHEMA_VERSION)
        );
        unsafe { aozora_bytes_free(diag) };

        unsafe { aozora_document_free(doc) };
    }

    /// Smoke: nodes JSON for plain input is the empty envelope.
    #[test]
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

    /// Smoke: ruby span emits a `kind:"ruby"` entry in nodes JSON.
    #[test]
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

    /// Smoke: ruby pair emits a `kind:"ruby"` entry in pairs JSON.
    #[test]
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

    /// Smoke: `to_source` round-trips ruby back to canonical Aozora
    /// source through the C ABI.
    #[test]
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
        // Byte-identical to the library's own round-trip authority.
        let want = aozora::parse(src.to_owned())
            .expect("source fits parser span limit")
            .snapshot()
            .to_source();
        assert_eq!(got, want, "to_source drift");
        unsafe { aozora_bytes_free(out) };
        unsafe { aozora_document_free(doc) };
    }

    /// Smoke: container-pairs JSON is the empty envelope for inline-only
    /// input and shares the wire authority.
    #[test]
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

    /// Smoke: gaiji JSON resolves a `※［＃…］` reference and shares the
    /// wire authority.
    #[test]
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

    /// Smoke: source byte length reports the owned UTF-8 buffer length.
    #[test]
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

    /// Smoke: the document-independent slug catalogue matches the wire
    /// authority.
    #[test]
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
    fn null_input_returns_null_input_status() {
        let mut doc = core::ptr::NonNull::<AozoraDocument>::dangling().as_ptr();
        let status = unsafe { aozora_document_new(core::ptr::null(), 0, &mut doc) };
        assert_eq!(status, AozoraStatus::NullInput as c_int);
        assert!(doc.is_null());
    }

    #[test]
    fn invalid_utf8_returns_invalid_utf8_status() {
        let bad = [0xFFu8, 0xFE, 0xFD];
        let mut doc = core::ptr::NonNull::<AozoraDocument>::dangling().as_ptr();
        let status = unsafe { aozora_document_new(bad.as_ptr(), bad.len(), &mut doc) };
        assert_eq!(status, AozoraStatus::InvalidUtf8 as c_int);
        assert!(doc.is_null());
    }

    /// An oversize `src_len` (> `u32::MAX`) is rejected at construction
    /// with [`AozoraStatus::SourceTooLarge`] and never reaches the
    /// parser core's `u32`-span assert. The guard checks `src_len`
    /// before dereferencing `src_ptr`, so we can pass a non-null
    /// dangling pointer with a fabricated huge length: the function
    /// returns before any read. This keeps the test from allocating
    /// 4 GiB.
    #[test]
    #[cfg(target_pointer_width = "64")]
    fn oversize_length_returns_source_too_large_status() {
        // Non-null but never dereferenced (guard returns first).
        let dangling = core::ptr::NonNull::<u8>::dangling().as_ptr().cast_const();
        let oversize = u32::MAX as usize + 1;
        let mut doc = core::ptr::NonNull::<AozoraDocument>::dangling().as_ptr();
        let status = unsafe { aozora_document_new(dangling, oversize, &mut doc) };
        assert_eq!(status, AozoraStatus::SourceTooLarge as c_int);
        assert!(doc.is_null(), "oversize input must not yield a handle");
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

    /// The [`AozoraStatus`] discriminants are a C ABI stability contract:
    /// embedders hard-code these exact integers (`-1` = null input, `-5` =
    /// oversize, …), so their numeric values must never drift silently. The
    /// behavioural tests above all compare a returned code against
    /// `AozoraStatus::Variant as c_int`, which *cannot* catch a change to the
    /// discriminant itself — the function's return value and the expected
    /// value move together, so the equality still holds. Pin every variant to
    /// its literal wire value here so a drift (or an accidental sign flip)
    /// fails the suite.
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
