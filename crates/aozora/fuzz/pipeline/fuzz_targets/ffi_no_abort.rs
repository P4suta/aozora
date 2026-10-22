//! Exercises the public `aozora_ffi` interface with arbitrary bytes.

#![no_main]

use core::ffi::c_int;

use aozora_ffi::{
    AozoraBytes, AozoraDocument, AozoraStatus, aozora_bytes_free, aozora_document_diagnostics_json,
    aozora_document_free, aozora_document_new, aozora_document_pairs_json, aozora_document_to_html,
    aozora_document_nodes_json,
};
use libfuzzer_sys::fuzz_target;

const PUA_SENTINELS: [char; 4] = ['\u{E001}', '\u{E002}', '\u{E003}', '\u{E004}'];

const fn empty_bytes() -> AozoraBytes {
    AozoraBytes {
        ptr: core::ptr::null_mut(),
        len: 0,
        cap: 0,
    }
}

fn exercise_accessor(
    doc: *const AozoraDocument,
    accessor: unsafe extern "C" fn(*const AozoraDocument, *mut AozoraBytes) -> c_int,
    data: &[u8],
    label: &str,
    validate: impl FnOnce(&[u8]),
) {
    let mut buf = empty_bytes();
    // SAFETY: the caller supplies a live `aozora_document_new` handle, and `buf` is a writable output slot.
    let status = unsafe { accessor(doc, &mut buf) };
    assert_eq!(
        status,
        AozoraStatus::Ok as c_int,
        "{label} returned non-Ok status {status} for src bytes = {data:?}",
    );
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
        // SAFETY: the accessor initialized the buffer, and this borrow ends before `aozora_bytes_free`.
        let slice = unsafe { core::slice::from_raw_parts(buf.ptr.cast_const(), buf.len) };
        validate(slice);
    } else {
        validate(&[]);
    }
    // SAFETY: `buf` comes from the accessor, and this call releases it once.
    unsafe { aozora_bytes_free(buf) };
}

fuzz_target!(|data: &[u8]| {
    let mut doc: *mut AozoraDocument = core::ptr::null_mut();
    // SAFETY: `data` remains a valid byte slice during the call, and `doc` is a writable handle slot.
    let status = unsafe { aozora_document_new(data.as_ptr(), data.len(), &mut doc) };

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

    let src = core::str::from_utf8(data).expect("checked Ok above");
    let src_has_pua = src.chars().any(|c| PUA_SENTINELS.contains(&c));

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

    // SAFETY: `doc` comes from `aozora_document_new`, and this call releases the live handle once.
    unsafe { aozora_document_free(doc) };
});
