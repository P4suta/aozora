//! Fuzz target — the `aozora-ffi` C-ABI surface must never panic/abort.
//!
//! Drives arbitrary bytes through the real exported `extern "C"`
//! entry points (`aozora_document_new` → `aozora_document_to_html` /
//! `aozora_document_diagnostics_json` / `aozora_document_nodes_json` /
//! `aozora_document_pairs_json` → the matching `*_free`) exactly as a
//! non-Rust host (Ruby / Node / Go / JVM) would. The crate is built
//! with `panic = "abort"` in release, so a panic anywhere on a parse /
//! render / JSON-serialize path reachable from these functions would
//! `abort()` the host process — a denial-of-service for every embedder.
//! This target makes libFuzzer surface such an input as a crash
//! artifact.
//!
//! Because the input is *raw bytes* (not pre-validated UTF-8), it also
//! exercises the FFI layer's own UTF-8 gate: non-UTF-8 slices must come
//! back as [`AozoraStatus::InvalidUtf8`] with a null handle, never a
//! crash. Valid-UTF-8 slices must round-trip through every accessor and
//! free cleanly.
//!
//! Invariants asserted (beyond "no abort"):
//!
//! 1. `aozora_document_new` returns either `Ok` (with a non-null
//!    handle) or `InvalidUtf8` (with a null handle) — never any other
//!    status, never `Ok` with a null handle.
//! 2. Every byte-buffer accessor returns `Ok` and a buffer whose
//!    `(ptr, len)` is internally consistent (non-null when `len > 0`).
//! 3. The HTML buffer is valid UTF-8 and carries no surviving PUA
//!    sentinel (U+E001..U+E004) — the same render contract the
//!    `render_html` target pins, but verified through the C ABI and
//!    only for inputs that did not themselves smuggle in a sentinel.
//! 4. Every buffer is released via `aozora_bytes_free` and the handle
//!    via `aozora_document_free` (ASan/LSan under cargo-fuzz then
//!    catches any leak or double-free).
//!
//! Run via `just fuzz-quick pipeline ffi_no_abort` (or
//! `fuzz-deep` / `fuzz-marathon`).

#![no_main]

use core::ffi::c_int;

use aozora_ffi::{
    AozoraBytes, AozoraDocument, AozoraStatus, aozora_bytes_free, aozora_document_diagnostics_json,
    aozora_document_free, aozora_document_new, aozora_document_pairs_json, aozora_document_to_html,
    aozora_document_nodes_json,
};
use libfuzzer_sys::fuzz_target;

/// PUA sentinel codepoints the renderer must consume — none may
/// survive into HTML emitted through the C ABI.
const PUA_SENTINELS: [char; 4] = ['\u{E001}', '\u{E002}', '\u{E003}', '\u{E004}'];

/// Empty `(ptr, len, cap)` triple to hand to an out-param accessor.
const fn empty_bytes() -> AozoraBytes {
    AozoraBytes {
        ptr: core::ptr::null_mut(),
        len: 0,
        cap: 0,
    }
}

/// Invoke one `(doc, *mut AozoraBytes) -> c_int` accessor, assert it
/// succeeded with a self-consistent buffer, optionally validate the
/// bytes, then free it. The closure sees the borrowed buffer contents
/// before they are released.
fn exercise_accessor(
    doc: *const AozoraDocument,
    accessor: unsafe extern "C" fn(*const AozoraDocument, *mut AozoraBytes) -> c_int,
    data: &[u8],
    label: &str,
    validate: impl FnOnce(&[u8]),
) {
    let mut buf = empty_bytes();
    // SAFETY: `doc` is a live handle from `aozora_document_new` (checked
    // non-null by the caller) and `&mut buf` is a writable AozoraBytes
    // slot, satisfying the accessor's documented contract.
    let status = unsafe { accessor(doc, &mut buf) };
    assert_eq!(
        status,
        AozoraStatus::Ok as c_int,
        "{label} returned non-Ok status {status} for src bytes = {data:?}",
    );
    // A zero-length buffer is allowed to carry a null ptr; a non-empty
    // one must not.
    if buf.len > 0 {
        assert!(
            !buf.ptr.is_null(),
            "{label} returned len {} with a null ptr; src bytes = {data:?}",
            buf.len,
        );
    }
    assert!(
        buf.cap >= buf.len,
        "{label} returned cap {} < len {}; src bytes = {data:?}",
        buf.cap,
        buf.len,
    );
    if !buf.ptr.is_null() {
        // SAFETY: `buf.ptr`/`buf.len` name the buffer the accessor just
        // populated; we only read it (and only for the duration of this
        // borrow, before `aozora_bytes_free` below).
        let slice = unsafe { core::slice::from_raw_parts(buf.ptr.cast_const(), buf.len) };
        validate(slice);
    } else {
        validate(&[]);
    }
    // SAFETY: `buf` was produced by the accessor (an aozora_* function),
    // so it is exactly the value `aozora_bytes_free` expects to invert.
    unsafe { aozora_bytes_free(buf) };
}

fuzz_target!(|data: &[u8]| {
    let mut doc: *mut AozoraDocument = core::ptr::null_mut();
    // SAFETY: `data.as_ptr()`/`data.len()` name a valid byte slice for
    // the duration of the call, and `&mut doc` is a writable handle slot.
    let status = unsafe { aozora_document_new(data.as_ptr(), data.len(), &mut doc) };

    // Invariant 1: only Ok (non-null handle) or InvalidUtf8 (null
    // handle) are legal outcomes for a (ptr, len) input.
    if status == AozoraStatus::InvalidUtf8 as c_int {
        assert!(
            doc.is_null(),
            "InvalidUtf8 must leave a null handle; src bytes = {data:?}",
        );
        return;
    }
    assert_eq!(
        status,
        AozoraStatus::Ok as c_int,
        "aozora_document_new returned unexpected status {status} for src bytes = {data:?}",
    );
    assert!(
        !doc.is_null(),
        "Ok status must yield a non-null handle; src bytes = {data:?}",
    );

    // The source is valid UTF-8 (else we'd have returned above). Decide
    // whether the no-PUA-in-HTML render contract applies: a source that
    // itself carries a reserved sentinel is allowed to pass it through
    // as plain text (it trips `Diagnostic::SourceContainsPua`), so the
    // invariant only targets *renderer*-planted sentinels.
    let src = core::str::from_utf8(data).expect("checked Ok above");
    let src_has_pua = src.chars().any(|c| PUA_SENTINELS.contains(&c));

    // Invariant 3: HTML is valid UTF-8 and sentinel-free.
    exercise_accessor(
        doc,
        aozora_document_to_html,
        data,
        "aozora_document_to_html",
        |html| {
            let Ok(text) = core::str::from_utf8(html) else {
                panic!("HTML buffer is not valid UTF-8; src bytes = {data:?}");
            };
            if !src_has_pua {
                for sentinel in PUA_SENTINELS {
                    assert!(
                        !text.contains(sentinel),
                        "PUA sentinel {sentinel:?} leaked into C-ABI HTML; src bytes = {data:?}",
                    );
                }
            }
        },
    );

    // The three JSON accessors: assert no-abort + buffer consistency.
    // Their bytes are not re-parsed here (the JSON-format round-trip is
    // covered elsewhere); reaching them at all proves the parse + JSON
    // serialize paths are panic-free through the C ABI.
    for (accessor, label) in [
        (
            aozora_document_diagnostics_json
                as unsafe extern "C" fn(*const AozoraDocument, *mut AozoraBytes) -> c_int,
            "aozora_document_diagnostics_json",
        ),
        (aozora_document_nodes_json, "aozora_document_nodes_json"),
        (aozora_document_pairs_json, "aozora_document_pairs_json"),
    ] {
        exercise_accessor(doc, accessor, data, label, |json| {
            assert!(
                core::str::from_utf8(json).is_ok(),
                "{label} buffer is not valid UTF-8; src bytes = {data:?}",
            );
        });
    }

    // Invariant 4: release the handle (LSan/ASan validates no leak).
    // SAFETY: `doc` is the live, not-yet-freed handle from
    // `aozora_document_new`; this is its single matching destructor.
    unsafe { aozora_document_free(doc) };
});
