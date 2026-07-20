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
 * Opaque handle to a parsed Aozora document. Allocate via
 * [`aozora_document_new`]; free via [`aozora_document_free`].
 *
 * Wraps an [`aozora::Document`] and its immutable parsed snapshot. The
 * C ABI treats it as a single-owner handle — embedders may move it
 * across threads but must not call into one handle concurrently from
 * multiple threads without external synchronisation.
 */
typedef struct AozoraDocument AozoraDocument;

/**
 * `(ptr, len, cap)` triple representing an owned `Vec<u8>` returned
 * to the caller. The caller MUST round-trip it through
 * [`aozora_bytes_free`] to release the memory.
 *
 * Layout matches `Vec<u8>::from_raw_parts(ptr, len, cap)` so the
 * destructor can reconstruct the vec for drop.
 */
typedef struct {
  /**
   * Pointer to the start of the owned allocation. Null only on the
   * caller-zeroed sentinel value (which [`aozora_bytes_free`] treats
   * as a no-op); on any value returned by an `aozora_*` function it
   * is non-null and valid for `cap` bytes.
   */
  uint8_t *ptr;
  /**
   * Number of initialised, readable bytes — the length of the
   * returned payload (HTML or JSON). May be less than `cap`.
   */
  uintptr_t len;
  /**
   * Total capacity of the allocation, in bytes. Must be passed back
   * unchanged to [`aozora_bytes_free`]: it reconstructs the `Vec`
   * with `(ptr, len, cap)`, so a wrong `cap` corrupts the allocator.
   */
  uintptr_t cap;
} AozoraBytes;

#ifdef __cplusplus
extern "C" {
#endif // __cplusplus

/**
 * Construct a [`Document`](AozoraDocument) from a UTF-8 byte slice.
 *
 * On success, writes the document handle to `*out_doc` and returns
 * [`AozoraStatus::Ok`]. On failure, writes `null` to `*out_doc` and
 * returns the matching error status.
 *
 * Inputs whose byte length exceeds [`u32::MAX`] are rejected with
 * [`AozoraStatus::SourceTooLarge`] *before* the parser core runs (the
 * core's span offsets are `u32` and it asserts that bound; under
 * `panic = "abort"` hitting the assert would abort the whole host
 * process). Guarding at construction means no oversize document can
 * exist, so the later `aozora_document_*` accessors never reach the
 * assert either. See the crate-level "Panic / abort contract" docs.
 *
 * # Safety
 *
 * - `src_ptr` must point to `src_len` valid UTF-8 bytes.
 * - `out_doc` must point to a writable `*mut AozoraDocument` slot.
 * - The caller must eventually call [`aozora_document_free`] on the
 *   returned handle.
 */
int aozora_document_new(const uint8_t *src_ptr, uintptr_t src_len, AozoraDocument **out_doc);

/**
 * Render the document to HTML, returning the result as an owned
 * byte buffer.
 *
 * On success, writes the bytes to `*out_html` and returns
 * [`AozoraStatus::Ok`]. The caller MUST call [`aozora_bytes_free`]
 * on the returned [`AozoraBytes`] to release the memory.
 *
 * # Safety
 *
 * - `doc` must be a non-null handle produced by
 *   [`aozora_document_new`] and not yet freed.
 * - `out_html` must point to a writable [`AozoraBytes`] slot.
 */
int aozora_document_to_html(const AozoraDocument *doc, AozoraBytes *out_html);

/**
 * Render the document's diagnostics as a JSON byte buffer.
 *
 * On success, writes the bytes to `*out_json` and returns
 * [`AozoraStatus::Ok`]. Empty document → the empty envelope
 * `{"schemaVersion":…,"data":[]}` (version is
 * [`aozora::json::SCHEMA_VERSION`]). The caller MUST call
 * [`aozora_bytes_free`] on the returned [`AozoraBytes`].
 *
 * Wire format is defined in [`aozora::json`] and shared bit-for-bit
 * with the WASM and PyO3 drivers.
 *
 * # Safety
 *
 * - `doc` must be a non-null handle produced by
 *   [`aozora_document_new`] and not yet freed.
 * - `out_json` must point to a writable [`AozoraBytes`] slot.
 */
int aozora_document_diagnostics_json(const AozoraDocument *doc, AozoraBytes *out_json);

/**
 * Render the document's diagnostics as a plain-text byte buffer.
 *
 * One block per diagnostic — `<severity> [<code>] @ <start>..<end>:
 * <message>` plus the offending source slice. This is the `miette`-free
 * portable rendering ([`aozora::diagnostics_text`]); a clean parse
 * yields an empty buffer. For the machine-readable view use
 * [`aozora_document_diagnostics_json`].
 *
 * On success, writes the bytes to `*out_text` and returns
 * [`AozoraStatus::Ok`]. The caller MUST call [`aozora_bytes_free`] on
 * the returned [`AozoraBytes`].
 *
 * # Safety
 *
 * - `doc` must be a non-null handle produced by
 *   [`aozora_document_new`] and not yet freed.
 * - `out_text` must point to a writable [`AozoraBytes`] slot.
 */
int aozora_document_diagnostics_text(const AozoraDocument *doc, AozoraBytes *out_text);

/**
 * Render the document's source-keyed Aozora nodes as a JSON byte
 * buffer.
 *
 * Each entry has the shape `{ kind, span: { start, end } }` in source
 * coordinates, sorted by `span.start`. Useful for editor integrations
 * (semantic tokens, document symbols, Lezer-Tree builders).
 *
 * On success, writes the bytes to `*out_json` and returns
 * [`AozoraStatus::Ok`]. Empty parse → the empty envelope
 * `{"schemaVersion":…,"data":[]}` (version is
 * [`aozora::json::SCHEMA_VERSION`]). The caller MUST call
 * [`aozora_bytes_free`] on the returned [`AozoraBytes`].
 *
 * Wire format is defined in [`aozora::json`] and shared bit-for-bit
 * with the WASM and PyO3 drivers.
 *
 * # Safety
 *
 * - `doc` must be a non-null handle produced by
 *   [`aozora_document_new`] and not yet freed.
 * - `out_json` must point to a writable [`AozoraBytes`] slot.
 */
int aozora_document_nodes_json(const AozoraDocument *doc, AozoraBytes *out_json);

/**
 * Render the document's matched open/close pair links as a JSON byte
 * buffer.
 *
 * Each entry has the shape
 * `{ kind, open: { start, end }, close: { start, end } }` in
 * sanitized-source coordinates. Useful for LSP requests like
 * `textDocument/linkedEditingRange` and
 * `textDocument/documentHighlight`.
 *
 * On success, writes the bytes to `*out_json` and returns
 * [`AozoraStatus::Ok`]. Empty parse → the empty envelope
 * `{"schemaVersion":…,"data":[]}` (version is
 * [`aozora::json::SCHEMA_VERSION`]). The caller MUST call
 * [`aozora_bytes_free`] on the returned [`AozoraBytes`].
 *
 * Wire format is defined in [`aozora::json`] and shared bit-for-bit
 * with the WASM and PyO3 drivers.
 *
 * # Safety
 *
 * - `doc` must be a non-null handle produced by
 *   [`aozora_document_new`] and not yet freed.
 * - `out_json` must point to a writable [`AozoraBytes`] slot.
 */
int aozora_document_pairs_json(const AozoraDocument *doc, AozoraBytes *out_json);

/**
 * Re-emit the document as Aozora source text (round-trip
 * serialization), returning the result as an owned byte buffer.
 *
 * Parses and walks the tree back to canonical source — the inverse of
 * [`aozora_document_to_html`]. This is the `to_source` surface shared
 * bit-for-bit with the WASM (`toSource`), PyO3 (`to_source`), and
 * Extism/Go drivers.
 *
 * On success, writes the bytes to `*out_source` and returns
 * [`AozoraStatus::Ok`]. The caller MUST call [`aozora_bytes_free`] on
 * the returned [`AozoraBytes`] to release the memory.
 *
 * # Safety
 *
 * - `doc` must be a non-null handle produced by
 *   [`aozora_document_new`] and not yet freed.
 * - `out_source` must point to a writable [`AozoraBytes`] slot.
 */
int aozora_document_to_source(const AozoraDocument *doc, AozoraBytes *out_source);

/**
 * Render the document's matched container open/close pairs as a JSON
 * byte buffer.
 *
 * Each entry has the shape
 * `{ kind, open: { start, end }, close: { start, end } }` in
 * sanitized-source coordinates, covering block-level enclosures
 * (indent / jisage / caption blocks) rather than the inline pairs of
 * [`aozora_document_pairs_json`].
 *
 * On success, writes the bytes to `*out_json` and returns
 * [`AozoraStatus::Ok`]. Empty parse → the empty envelope
 * `{"schemaVersion":…,"data":[]}` (version is
 * [`aozora::json::SCHEMA_VERSION`]). The caller MUST call
 * [`aozora_bytes_free`] on the returned [`AozoraBytes`].
 *
 * Wire format is defined in [`aozora::json`] and shared bit-for-bit
 * with the WASM, PyO3, and Extism/Go drivers.
 *
 * # Safety
 *
 * - `doc` must be a non-null handle produced by
 *   [`aozora_document_new`] and not yet freed.
 * - `out_json` must point to a writable [`AozoraBytes`] slot.
 */
int aozora_document_container_pairs_json(const AozoraDocument *doc, AozoraBytes *out_json);

/**
 * Render the document's resolved gaiji references (`※［＃…］`) as a
 * JSON byte buffer.
 *
 * Each entry records the source span of a gaiji directive alongside its
 * resolved canonical form (JIS X 0213 men-ku-ten, Unicode scalar, …).
 * The scan runs over the document's source string directly.
 *
 * On success, writes the bytes to `*out_json` and returns
 * [`AozoraStatus::Ok`]. A source with no gaiji → the empty envelope
 * `{"schemaVersion":…,"data":[]}` (version is
 * [`aozora::json::SCHEMA_VERSION`]). The caller MUST call
 * [`aozora_bytes_free`] on the returned [`AozoraBytes`].
 *
 * Wire format is defined in [`aozora::json`]. Its records also drive
 * the WASM `gaiji` values and the Python, Extism, and Go projections.
 *
 * # Safety
 *
 * - `doc` must be a non-null handle produced by
 *   [`aozora_document_new`] and not yet freed.
 * - `out_json` must point to a writable [`AozoraBytes`] slot.
 */
int aozora_document_gaiji_json(const AozoraDocument *doc, AozoraBytes *out_json);

/**
 * Write the document's source byte length to `*out_len`.
 *
 * This is the length of the UTF-8 source buffer the handle owns — the
 * same quantity the WASM (`sourceByteLen`) and PyO3 (`source_byte_len`)
 * drivers return. Hosts use it to size buffers and to convert the `u32`
 * source coordinates in the JSON projections back into their own string
 * indices.
 *
 * On success, writes the length to `*out_len` and returns
 * [`AozoraStatus::Ok`].
 *
 * # Safety
 *
 * - `doc` must be a non-null handle produced by
 *   [`aozora_document_new`] and not yet freed.
 * - `out_len` must point to a writable `usize` slot.
 */
int aozora_document_source_byte_len(const AozoraDocument *doc, uintptr_t *out_len);

/**
 * Render the spec's canonical slug catalogue as a JSON byte buffer.
 *
 * This is document-independent — it projects the static slug table from
 * [`aozora::json::slugs`], the same authority behind the WASM
 * `slugs` and PyO3 `slugs_json` exports. Editor front ends use
 * it to drive directive completion.
 *
 * On success, writes the bytes to `*out_json` and returns
 * [`AozoraStatus::Ok`]. The caller MUST call [`aozora_bytes_free`] on
 * the returned [`AozoraBytes`].
 *
 * Wire format is defined in [`aozora::json`] and shared bit-for-bit
 * with PyO3 and Extism/Go.
 *
 * # Safety
 *
 * - `out_json` must point to a writable [`AozoraBytes`] slot.
 */
int aozora_slugs_json(AozoraBytes *out_json);

/**
 * Free a document handle returned by [`aozora_document_new`].
 *
 * # Safety
 *
 * - `doc` must be either null (then this is a no-op) or a handle
 *   returned by [`aozora_document_new`] that has not already been
 *   freed. Double-free is undefined behaviour.
 */
void aozora_document_free(AozoraDocument *doc);

/**
 * Free a byte buffer returned by an `aozora_*` function.
 *
 * # Safety
 *
 * - `bytes` must be a value previously returned by one of the
 *   `aozora_*` functions in this crate. Reusing or aliasing the
 *   inner pointer after this call is undefined behaviour.
 */
void aozora_bytes_free(AozoraBytes bytes);

#ifdef __cplusplus
}  // extern "C"
#endif  // __cplusplus

#endif  /* AOZORA_FFI_H */
