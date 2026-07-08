# aozora-ffi

C ABI driver for [**aozora**](https://github.com/P4suta/aozora) — a
pure-functional Rust parser for **青空文庫記法** (Aozora Bunko notation):
ruby (`｜青梅《おうめ》`), bouten (`［＃「X」に傍点］`), 縦中横, 外字
references, kaeriten, indent / align containers, page breaks, and more.

The API is **opaque-handle**: every parse produces a `*mut AozoraDocument`,
accessed through a small set of `aozora_*` functions and freed by a single
matching destructor. Structured data is returned as JSON strings (the
cross-driver wire envelope), because the AST is `#[non_exhaustive]` upstream
and every modern host language already ships a JSON reader.

## Install

This crate is not published to crates.io; build the native artefacts from
the workspace:

```sh
cargo build --release -p aozora-ffi
```

That produces, under `target/release/`:

- the shared library `libaozora_ffi.{so,dylib}` / `aozora_ffi.dll`
  (the `cdylib`) and the static archive `libaozora_ffi.a` (the `staticlib`),
- the C header `aozora.h`, generated from the `extern "C"` surface by
  `build.rs` (cbindgen).

Link against either the `cdylib` or the `staticlib` and `#include "aozora.h"`.
Designed for embedding in non-Rust hosts (Ruby / Node / Go / JVM via
libffi / FFI / cgo / JNA).

## Quickstart

```c
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include "aozora.h"

int main(void) {
    const char *src = "｜青梅《おうめ》の街";  /* UTF-8 */

    AozoraDocument *doc = NULL;
    /* 0 = AozoraStatus::Ok; negative = null input / invalid UTF-8 / too large */
    if (aozora_document_new((const uint8_t *)src, strlen(src), &doc) != 0) {
        return 1;
    }

    AozoraBytes html = {0};  /* zeroed ptr => aozora_bytes_free is a no-op */
    if (aozora_document_to_html(doc, &html) == 0) {
        fwrite(html.ptr, 1, html.len, stdout);   /* semantic HTML5 with <ruby> */
        aozora_bytes_free(html);
    }

    AozoraBytes nodes = {0};
    if (aozora_document_nodes_json(doc, &nodes) == 0) {
        /* {"schemaVersion":2,"data":[{ "kind":"ruby", "span":{…} }, …]} */
        fwrite(nodes.ptr, 1, nodes.len, stdout);
        aozora_bytes_free(nodes);
    }

    aozora_document_free(doc);
    return 0;
}
```

Every buffer returned as an `AozoraBytes` (`{ ptr, len, cap }`) MUST be
round-tripped through `aozora_bytes_free`, and every handle through
`aozora_document_free`. The other accessors follow the same shape:
`aozora_document_diagnostics_json`, `aozora_document_diagnostics_text`, and
`aozora_document_pairs_json`.

## UTF-8 source

Inputs are UTF-8 byte slices; `aozora_document_new` returns
`AozoraStatus::InvalidUtf8` for anything else. Real 青空文庫 archive files are
`Shift_JIS`, so decode them to UTF-8 host-side before handing the bytes in.
Sources longer than `u32::MAX` (~4 GiB) are rejected up front with
`AozoraStatus::SourceTooLarge` — the parser core uses `u32` span offsets.

## Performance

The parser's process-global lazy tables are built on the first parse; each
`aozora_*` accessor re-parses the owned source into a fresh, lifetime-free
tree and frees it when the call returns. Emit every output you need from a
single handle rather than reconstructing one per accessor. Linking the
`staticlib` lets the release profile's fat LTO inline across the ABI boundary.

## Panic / abort contract

The workspace release profile is compiled with `panic = "abort"`, so a Rust
panic does **not** unwind across the C ABI — it aborts the whole host process.
There is no `catch_unwind` net. Every `aozora_*` function validates its
pointer / length / UTF-8 preconditions up front and reports problems through
the `AozoraStatus` return code; treat a non-zero status as the only supported
error channel and pre-validate untrusted input.

## Threading

An `AozoraDocument` handle is single-owner: embedders may move it across
threads, but must not call into one handle concurrently from multiple threads
without external synchronisation. Construct a fresh handle per thread.

## Links

- **Source & issues:** <https://github.com/P4suta/aozora>
- **C ABI reference:** <https://p4suta.github.io/aozora/api/aozora_ffi/index.html>
- **Handbook:** <https://p4suta.github.io/aozora/>
- **Changelog:** <https://github.com/P4suta/aozora/blob/main/CHANGELOG.md>

## License

`Apache-2.0 OR MIT`, at your option.
