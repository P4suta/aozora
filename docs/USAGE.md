# Usage guide

How to invoke aozora from each supported entry point: the CLI, the
Rust library, the WASM driver, the C ABI driver, and the Python
binding.

For architectural background see [`ARCHITECTURE.md`](./ARCHITECTURE.md).
For the contributor flow see [`../CONTRIBUTING.md`](../CONTRIBUTING.md).

## CLI

The `aozora` binary ships in every GitHub Release as a tar.gz / zip
for linux x86_64, macos arm64, and windows x86_64. Or build from
source:

```sh
cargo install --git https://github.com/P4suta/aozora --tag v0.2.5 --locked aozora-cli
```

Three subcommands cover the public functionality:

```sh
aozora check FILE.txt           # lex + report diagnostics on stderr
aozora fmt   FILE.txt           # round-trip parse ∘ serialize, print to stdout
aozora render FILE.txt          # render to HTML on stdout
```

Common flags:

- `-` (or no path argument) — read from stdin instead of a file.
- `-E sjis` / `--encoding sjis` — decode Shift_JIS source (default
  is UTF-8). Aozora Bunko's distributed `.txt` files are Shift_JIS.
- `aozora check --strict` — exit non-zero on any diagnostic.
- `aozora fmt --check` — exit non-zero if the formatted output differs
  from the input.
- `aozora fmt --write` — overwrite the input file with the canonical
  form (ignored when reading from stdin).

Examples:

```sh
# Lex an Aozora Bunko file and print diagnostics
aozora check -E sjis crime_and_punishment.txt

# Render to HTML
aozora render -E sjis crime_and_punishment.txt > out.html

# Pipe from stdin
cat src.txt | aozora render -

# CI gate: format must round-trip
aozora fmt --check src.txt
```

Exit codes: `0` on success, `1` on diagnostics with `--strict` or on a
formatting mismatch with `--check`, `2` on a usage error.

## Rust library

In `Cargo.toml`:

```toml
[dependencies]
aozora = { git = "https://github.com/P4suta/aozora.git", tag = "v0.2.5" }
```

Minimal use:

```rust
use aozora::{Document, Diagnostic};

fn main() {
    let source = std::fs::read_to_string("src.txt").unwrap();
    let doc = Document::new(source);
    let tree = doc.parse();

    let html: String = tree.to_html();
    let canonical: String = tree.serialize();
    let diagnostics: &[Diagnostic] = tree.diagnostics();

    println!("{html}");
    eprintln!("{} diagnostic(s)", diagnostics.len());
}
```

`Document` owns a [`bumpalo`](https://docs.rs/bumpalo) arena; `tree`
borrows from it. The borrow lives as long as the `Document`, so
hand the `Document` around (not the `tree`) when you need to keep
the parse result alive across function boundaries. Dropping the
`Document` releases every node in a single `Bump::reset` step.

For Shift_JIS input, decode through `aozora-encoding` first:

```rust
use aozora::Document;
use aozora_encoding::sjis;

let bytes = std::fs::read("src.sjis.txt")?;
let utf8 = sjis::decode_to_string(&bytes)?;
let doc = Document::new(utf8);
let tree = doc.parse();
```

Walk the AST when you need structured access:

```rust
use aozora::AozoraNode;

for node in tree.nodes() {
    if let AozoraNode::Ruby(r) = node {
        println!("ruby: target={:?} reading={:?}", r.target(), r.reading());
    }
}
```

Full API reference: <https://p4suta.github.io/aozora/>.

## WASM

The `aozora-wasm` crate compiles to `wasm32-unknown-unknown` and
exposes a `Document` class via `wasm-bindgen`. Build with:

```sh
rustup target add wasm32-unknown-unknown   # one-time
wasm-pack build --target web --release crates/aozora-wasm
```

Note that `crates/aozora-wasm/Cargo.toml` sets `wasm-opt = false`
because `wasm-pack`'s bundled binaryen lags behind the bulk-memory
opcodes Rust 1.95+ emits. Run a current `wasm-opt` over the artifact
yourself:

```sh
wasm-opt -O3 --enable-bulk-memory --enable-mutable-globals \
    crates/aozora-wasm/pkg/aozora_wasm_bg.wasm \
    -o crates/aozora-wasm/pkg/aozora_wasm_bg.wasm
```

The post-`wasm-opt` artifact has a 500 KiB size budget.

JS / TypeScript usage:

```js
import init, { Document } from "./pkg/aozora_wasm.js";

await init();
const doc = new Document("｜青梅《おうめ》");
const html = doc.to_html();
const canonical = doc.serialize();
const diagnostics = JSON.parse(doc.diagnostics_json());
console.log(html);
doc.free();   // release the bumpalo arena explicitly
```

Methods on `Document`:

| Method | Returns | Notes |
|---|---|---|
| `new(source: string)` | `Document` | Copies the JS string into a Rust `Box<str>`. |
| `to_html()` | `string` | Renders to semantic HTML5. |
| `serialize()` | `string` | Re-emits canonical 青空文庫 source. |
| `diagnostics_json()` | `string` | JSON-encoded array of diagnostic objects. |
| `source_byte_len()` | `number` | Source byte length, useful for progress UI. |
| `free()` | — | Explicit drop; otherwise the JS GC eventually releases. |

## C ABI

The `aozora-ffi` crate compiles to a `cdylib` (`libaozora_ffi.so` /
`.dylib` / `aozora_ffi.dll`) plus a `staticlib`. The API is opaque
handle + JSON-encoded structured data:

```sh
cargo build --release -p aozora-ffi
# → target/release/libaozora_ffi.{so,dylib,a}
```

Quick smoke test (builds the cdylib, compiles `tests/c_smoke/smoke.c`
against it, runs it):

```sh
just smoke-ffi
```

Minimal C usage:

```c
#include <stdint.h>
#include <stdio.h>
#include <string.h>

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
extern void    aozora_bytes_free(AozoraBytes *bytes);
extern void    aozora_document_free(AozoraDocument *doc);

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

Status codes (returned by every `aozora_*` function):

| Code | Meaning |
|---|---|
| `0` | Ok |
| `-1` | Null input pointer |
| `-2` | Input was not valid UTF-8 |
| `-3` | Allocation failed |
| `-4` | Internal serialisation error |

Memory ownership: every pointer or `AozoraBytes` returned by an
`aozora_*` function must be released by the matching `_free` call.
Dropping a handle without calling free leaks; freeing then
dereferencing is undefined behaviour.

The build script generates `aozora.h` automatically. After
`cargo build --release -p aozora-ffi` the header lands at:

- `target/release/aozora.h` (host-side convenience copy)
- `$OUT_DIR/aozora.h` (cargo build-script standard location)

`#include "aozora.h"` in your C source and link against
`-laozora_ffi`.

## Python

The `aozora-py` crate is built and distributed via [`maturin`](https://www.maturin.rs/).
The PyO3 binding sits behind the `extension-module` feature so a
plain `cargo build --workspace` succeeds without Python development
headers installed:

```sh
pip install maturin
cd crates/aozora-py
maturin develop -F extension-module          # install in current venv
# or:
maturin build --release -F extension-module  # produce a wheel
```

Minimal Python usage:

```python
from aozora_py import Document

doc = Document("｜青梅《おうめ》")
print(doc.to_html())          # <ruby>青梅<rt>おうめ</rt></ruby>
print(doc.serialize())        # ｜青梅《おうめ》
print(doc.diagnostics())      # JSON string
```

The `Document` handle is `unsendable` (PyO3 marker) because the
underlying `bumpalo` arena uses interior `Cell` state. Concurrent
access from another Python thread raises a `RuntimeError`.

## Environment variables

| Variable | Used by | Purpose |
|---|---|---|
| `AOZORA_CORPUS_ROOT` | `aozora-corpus`, all sample-profiling, corpus sweep | Directory of 青空文庫 source files (UTF-8 or Shift_JIS). |
| `AOZORA_PROFILE_LIMIT` | `aozora-bench` probes | Cap the number of corpus documents per probe. |
| `AOZORA_PROFILE_REPEAT` | `samply-corpus` / `samply-render` | Number of parse / render passes per document after the one-time corpus load. Default 5. |
| `AOZORA_PROBE_DOC` | `pathological_probe` | Single corpus path to probe in tight per-call mode. |
| `RUSTC_WRAPPER` | dev container | `sccache` for warm builds. |
| `CARGO_INCREMENTAL` | dev container | Set to `0` for sccache compatibility. |
| `SCCACHE_CACHE_SIZE` | dev container | Defaults to `10G`. |
