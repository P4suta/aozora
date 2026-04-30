# C ABI

The `aozora-ffi` crate compiles to a `cdylib` + `staticlib`. The API
is opaque-handle + JSON-encoded structured data — the C side never
sees a Rust type, just opaque pointers and byte buffers.

## Build

```sh
cargo build --release -p aozora-ffi
# → target/release/libaozora_ffi.{so,dylib,a}
# → target/release/aozora.h          (cbindgen-generated)
```

The build script regenerates `aozora.h` automatically. After build,
the header lands at:

- `target/release/aozora.h` — host-side convenience copy
- `$OUT_DIR/aozora.h` — cargo build-script standard location

`#include "aozora.h"` and link with `-laozora_ffi`.

## Smoke test

```sh
just smoke-ffi
```

Builds the cdylib, compiles `crates/aozora-ffi/tests/c_smoke/smoke.c`
against it, runs it end-to-end. CI runs this on every PR — if the
ABI shape changes accidentally, the smoke test fails before the PR
merges.

## Minimal C usage

```c
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include "aozora.h"

int main(void) {
    const char *src = "｜青梅《おうめ》";
    AozoraDocument *doc = NULL;
    if (aozora_document_new((const uint8_t *)src, strlen(src), &doc) != 0)
        return 1;

    AozoraBytes html = {0};
    if (aozora_document_to_html(doc, &html) != 0) {
        aozora_document_free(doc);
        return 1;
    }
    fwrite(html.ptr, 1, html.len, stdout);

    aozora_bytes_free(&html);
    aozora_document_free(doc);
    return 0;
}
```

## API surface

```c
typedef struct AozoraDocument AozoraDocument;
typedef struct {
    uint8_t *ptr;
    size_t   len;
    size_t   cap;
} AozoraBytes;

extern int32_t aozora_document_new(const uint8_t *src, size_t src_len,
                                   AozoraDocument **out_doc);
extern int32_t aozora_document_to_html(const AozoraDocument *doc,
                                       AozoraBytes *out_html);
extern int32_t aozora_document_serialize(const AozoraDocument *doc,
                                         AozoraBytes *out_canonical);
extern int32_t aozora_document_diagnostics_json(const AozoraDocument *doc,
                                                AozoraBytes *out_json);
extern void    aozora_bytes_free(AozoraBytes *bytes);
extern void    aozora_document_free(AozoraDocument *doc);
```

## Status codes

| Code | Meaning |
|---|---|
| `0` | Ok |
| `-1` | Null input pointer |
| `-2` | Input was not valid UTF-8 |
| `-3` | Allocation failed |
| `-4` | Internal serialisation error |

## Memory ownership

Every pointer or `AozoraBytes` returned by an `aozora_*` function
must be released by the matching `_free` call:

| Returned by | Free with |
|---|---|
| `aozora_document_new` (`AozoraDocument *`) | `aozora_document_free` |
| `aozora_document_to_html` (`AozoraBytes`) | `aozora_bytes_free` |
| `aozora_document_serialize` (`AozoraBytes`) | `aozora_bytes_free` |
| `aozora_document_diagnostics_json` (`AozoraBytes`) | `aozora_bytes_free` |

Dropping a handle without `_free` leaks; freeing then dereferencing
is undefined behaviour. This is the standard ABI contract — any
`unsafe { Box::from_raw(...) }` mistake on the consumer side
trips both ASan and miri (both run in CI on the FFI test suite).

## Why JSON for diagnostics, not a C struct?

Three reasons.

1. **Variant types.** `Diagnostic` has optional fields (`help`,
   sometimes a multi-span). A flat C struct would either lose data
   or grow nullable pointers everywhere. JSON expresses optionality
   naturally.
2. **Schema stability.** Adding a new diagnostic field is a
   backward-compatible JSON change. Adding a field to a C struct
   breaks every consumer that compiled against the old size.
3. **Single emitter.** The same JSON shape is produced by
   `aozora-wasm` (consumed by JS) and `aozora-py` (consumed by
   Python). Aligning the C ABI on the same shape means downstream
   polyglot consumers don't translate between three different
   schemas.

The cost is one `serde_json::to_string` call per
`aozora_document_diagnostics_json` invocation — a one-shot O(N)
allocation that is a rounding error compared to the parse itself.

## Why opaque handle + bytes, not a flat C struct projection?

A `flat C struct` projection of `AozoraTree` would require:

- Naming every Rust enum variant in C (not supported cleanly via
  cbindgen for tagged unions).
- Translating the bumpalo arena into a malloc-backed block
  contiguous with the tree (which means copying the tree out).
- Pinning the AST shape across the C ABI — internal refactors
  (e.g. adding a new `AozoraNode` variant) would break ABI without
  warning.

The opaque-handle approach keeps the AST entirely Rust-side. C
consumers ask for HTML, canonical text, or JSON-encoded
diagnostics — three stable shapes that don't change with internal
refactors.

## Use from Go / Zig / Nim

Anything with a C FFI. The `aozora.h` header is plain C99 — no
inline functions, no macros that depend on a compiler-specific
extension, no `#pragma`. Tested in CI by the smoke test against
gcc, clang, and msvc.

## See also

- [Install → C ABI](../getting-started/install.md#c-abi)
- [Bindings → WASM](wasm.md) — same JSON diagnostics shape.
