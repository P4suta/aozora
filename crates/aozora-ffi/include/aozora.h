/* aozora-ffi — Aozora Bunko notation parser C ABI.
   Dual-licensed: Apache-2.0 OR MIT.

   PANIC / ABORT CONTRACT (read before embedding):
   This library is built with `panic = "abort"`. A Rust panic does NOT
   unwind across this ABI — it calls abort() and terminates the ENTIRE
   host process. There is no catch_unwind net, and none is possible.
   Every aozora_* function validates its inputs up front and reports
   problems through the int32_t status code (0 = ok, negative = error;
   see AozoraStatus). Inputs larger than UINT32_MAX bytes are rejected
   at aozora_document_new with SOURCE_TOO_LARGE (-5) before the parser
   core runs. Callers MUST pre-validate untrusted input and MUST NOT
   rely on catching a panic to recover. */

#ifndef AOZORA_FFI_H
#define AOZORA_FFI_H

#pragma once

#include <stdint.h>
#include <stddef.h>
#define AOZORA_SCHEMA_VERSION 3

/**
 * An opaque parsed document handle.
 * Create it with [`aozora_document_new`] and release it once with [`aozora_document_free`].
 * A handle can move between threads, but callers must serialize access and destruction.
 */
typedef struct AozoraDocument AozoraDocument;

/**
 * An owned byte buffer returned by the library.
 * Pass the unchanged `ptr`, `len`, and `cap` to [`aozora_bytes_free`] exactly once.
 * The payload has no null terminator.
 */
typedef struct {
  /**
   * Allocation pointer, non-null for buffers returned by the library.
   * A null pointer acts as an empty sentinel for [`aozora_bytes_free`].
   */
  uint8_t *ptr;
  /**
   * Number of initialized, readable bytes, no greater than `cap`.
   */
  uintptr_t len;
  /**
   * Allocation capacity in bytes, which must remain unchanged until release.
   */
  uintptr_t cap;
} AozoraBytes;

#ifdef __cplusplus
extern "C" {
#endif // __cplusplus

/**
 * Returns the parser version as an owned UTF-8 buffer through `out_version`.
 * Returns [`AozoraStatus::NullInput`] for a null output pointer, or [`AozoraStatus::Ok`] on success.
 * Release the returned buffer with [`aozora_bytes_free`].
 *
 * # Safety
 * A non-null `out_version` must reference a writable, aligned [`AozoraBytes`] slot.
 */
int aozora_version(AozoraBytes *out_version);

/**
 * Returns the structured wire schema version without accessing caller memory.
 */
uint32_t aozora_schema_version(void);

/**
 * Parses a UTF-8 byte slice and writes an owned handle to `out_doc` on success.
 * Returns [`AozoraStatus::NullInput`] for null pointers, [`AozoraStatus::InvalidUtf8`] for invalid encoding, or [`AozoraStatus::SourceTooLarge`] for a length exceeding [`u32::MAX`].
 * A failure sets a non-null `out_doc` to null.
 * Release a successful handle with [`aozora_document_free`].
 *
 * # Safety
 * For accepted source lengths, a non-null `src_ptr` must reference `src_len` readable bytes in one live allocation, with length at most `isize::MAX`.
 * A non-null `out_doc` must reference a writable, aligned pointer slot.
 */
int aozora_document_new(const uint8_t *src_ptr,
                        uintptr_t src_len,
                        AozoraDocument **out_doc);

/**
 * Writes rendered HTML to `out_html`.
 * Returns [`AozoraStatus::NullInput`] for null pointers, or [`AozoraStatus::Ok`] on success.
 * Release the returned buffer with [`aozora_bytes_free`].
 *
 * # Safety
 * `doc` must be null or an unfreed handle from [`aozora_document_new`].
 * Callers must serialize access and destruction.
 * A non-null `out_html` must reference a writable, aligned [`AozoraBytes`] slot.
 */
int aozora_document_to_html(const AozoraDocument *doc, AozoraBytes *out_html);

/**
 * Writes diagnostics as a JSON envelope to `out_json`.
 * The wire format follows [`aozora::json::diagnostics`].
 * Returns [`AozoraStatus::NullInput`] for null pointers, or [`AozoraStatus::Ok`] on success.
 * Release the returned buffer with [`aozora_bytes_free`].
 *
 * # Safety
 * `doc` must be null or an unfreed handle from [`aozora_document_new`].
 * Callers must serialize access and destruction.
 * A non-null `out_json` must reference a writable, aligned [`AozoraBytes`] slot.
 */
int aozora_document_diagnostics_json(const AozoraDocument *doc, AozoraBytes *out_json);

/**
 * Writes a plain-text diagnostic report to `out_text`.
 * A clean parse produces an empty buffer.
 * The report follows [`aozora::diagnostics_text`].
 * Returns [`AozoraStatus::NullInput`] for null pointers, or [`AozoraStatus::Ok`] on success.
 * Release the returned buffer with [`aozora_bytes_free`].
 *
 * # Safety
 * `doc` must be null or an unfreed handle from [`aozora_document_new`].
 * Callers must serialize access and destruction.
 * A non-null `out_text` must reference a writable, aligned [`AozoraBytes`] slot.
 */
int aozora_document_diagnostics_text(const AozoraDocument *doc, AozoraBytes *out_text);

/**
 * Writes nodes as a JSON envelope to `out_json`.
 * Entries use original-source byte spans in source order, following [`aozora::json::nodes`].
 * Returns [`AozoraStatus::NullInput`] for null pointers, or [`AozoraStatus::Ok`] on success.
 * Release the returned buffer with [`aozora_bytes_free`].
 *
 * # Safety
 * `doc` must be null or an unfreed handle from [`aozora_document_new`].
 * Callers must serialize access and destruction.
 * A non-null `out_json` must reference a writable, aligned [`AozoraBytes`] slot.
 */
int aozora_document_nodes_json(const AozoraDocument *doc, AozoraBytes *out_json);

/**
 * Writes matched delimiters as a JSON envelope to `out_json`.
 * Entries use original-source byte spans, following [`aozora::json::pairs`].
 * Returns [`AozoraStatus::NullInput`] for null pointers, or [`AozoraStatus::Ok`] on success.
 * Release the returned buffer with [`aozora_bytes_free`].
 *
 * # Safety
 * `doc` must be null or an unfreed handle from [`aozora_document_new`].
 * Callers must serialize access and destruction.
 * A non-null `out_json` must reference a writable, aligned [`AozoraBytes`] slot.
 */
int aozora_document_pairs_json(const AozoraDocument *doc, AozoraBytes *out_json);

/**
 * Writes canonical Aozora source to `out_source`.
 * Returns [`AozoraStatus::NullInput`] for null pointers, or [`AozoraStatus::Ok`] on success.
 * Release the returned buffer with [`aozora_bytes_free`].
 *
 * # Safety
 * `doc` must be null or an unfreed handle from [`aozora_document_new`].
 * Callers must serialize access and destruction.
 * A non-null `out_source` must reference a writable, aligned [`AozoraBytes`] slot.
 */
int aozora_document_to_source(const AozoraDocument *doc, AozoraBytes *out_source);

/**
 * Writes matched container delimiters as a JSON envelope to `out_json`.
 * Entries use original-source byte spans, following [`aozora::json::container_pairs`].
 * Returns [`AozoraStatus::NullInput`] for null pointers, or [`AozoraStatus::Ok`] on success.
 * Release the returned buffer with [`aozora_bytes_free`].
 *
 * # Safety
 * `doc` must be null or an unfreed handle from [`aozora_document_new`].
 * Callers must serialize access and destruction.
 * A non-null `out_json` must reference a writable, aligned [`AozoraBytes`] slot.
 */
int aozora_document_container_pairs_json(const AozoraDocument *doc, AozoraBytes *out_json);

/**
 * Writes gaiji resolutions as a JSON envelope to `out_json`.
 * Entries follow [`aozora::json::gaiji`].
 * Returns [`AozoraStatus::NullInput`] for null pointers, or [`AozoraStatus::Ok`] on success.
 * Release the returned buffer with [`aozora_bytes_free`].
 *
 * # Safety
 * `doc` must be null or an unfreed handle from [`aozora_document_new`].
 * Callers must serialize access and destruction.
 * A non-null `out_json` must reference a writable, aligned [`AozoraBytes`] slot.
 */
int aozora_document_gaiji_json(const AozoraDocument *doc, AozoraBytes *out_json);

/**
 * Writes the owned source's UTF-8 byte length to `out_len`.
 * Returns [`AozoraStatus::NullInput`] for null pointers, or [`AozoraStatus::Ok`] on success.
 *
 * # Safety
 * `doc` must be null or an unfreed handle from [`aozora_document_new`].
 * Callers must serialize access and destruction.
 * A non-null `out_len` must reference a writable, aligned `usize` slot.
 */
int aozora_document_source_byte_len(const AozoraDocument *doc, uintptr_t *out_len);

/**
 * Writes the canonical slug catalogue as a JSON envelope to `out_json`.
 * The wire format follows [`aozora::json::slugs`].
 * Returns [`AozoraStatus::NullInput`] for a null output pointer, or [`AozoraStatus::Ok`] on success.
 * Release the returned buffer with [`aozora_bytes_free`].
 *
 * # Safety
 * A non-null `out_json` must reference a writable, aligned [`AozoraBytes`] slot.
 */
int aozora_slugs_json(AozoraBytes *out_json);

/**
 * Releases a document handle, doing nothing for null.
 *
 * # Safety
 * A non-null `doc` must be an unfreed handle returned by [`aozora_document_new`].
 * No other call can access the handle during or after release.
 */
void aozora_document_free(AozoraDocument *doc);

/**
 * Releases a byte buffer, doing nothing for a null allocation pointer.
 *
 * # Safety
 * A non-null buffer must be an unchanged, unfreed value returned by this library.
 * No reference to its allocation can remain in use during or after release.
 */
void aozora_bytes_free(AozoraBytes bytes);

#ifdef __cplusplus
}  // extern "C"
#endif  // __cplusplus

#endif  /* AOZORA_FFI_H */
