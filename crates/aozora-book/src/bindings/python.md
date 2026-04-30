# Python (PyO3 / maturin)

The `aozora-py` crate is a [PyO3](https://pyo3.rs/) binding shipped
via [`maturin`](https://www.maturin.rs/).

## Install

```sh
pip install maturin                         # one-time

cd crates/aozora-py
maturin develop -F extension-module         # install in current venv
# or
maturin build -F extension-module --release # produce a redistributable wheel
```

The `extension-module` feature gates the PyO3 import-side machinery
behind a flag, so a plain `cargo build --workspace` succeeds without
Python development headers installed. CI has both modes covered.

## Minimal Python usage

```python
from aozora_py import Document

doc = Document("｜青梅《おうめ》")
print(doc.to_html())          # <ruby>青梅<rt>おうめ</rt></ruby>
print(doc.serialize())        # ｜青梅《おうめ》
print(doc.diagnostics())      # JSON-encoded list of diagnostic dicts
```

## API surface

| Method | Returns | Notes |
|---|---|---|
| `Document(source: str)` | `Document` | The constructor copies `source` into a Rust `Box<str>`. |
| `to_html() -> str` | str | Renders to semantic HTML5 with `aozora-*` class hooks. |
| `serialize() -> str` | str | Re-emits canonical 青空文庫 source. |
| `diagnostics() -> str` | str | JSON-encoded list (same schema as the WASM and FFI bindings). |
| `source_byte_len() -> int` | int | Source byte length. |

The diagnostics JSON shape is shared across every binding — see
[Bindings → WASM](wasm.md#api-surface) for the schema.

## Thread safety: `unsendable`

The `Document` type is marked `unsendable` (PyO3 marker) because
the underlying bumpalo arena uses interior `Cell` state. Concurrent
access from another Python thread raises a `RuntimeError`:

```python
import threading
from aozora_py import Document

doc = Document(open("src.txt").read())
def worker(): doc.to_html()              # raises RuntimeError on second thread
threading.Thread(target=worker).start()  # boom
```

For *parallel* corpus processing, create a `Document` per thread.
The arena resets per-`Document`, so there's no contention point;
each thread allocates from its own arena.

### Why not `Send`?

PyO3 has a `Sendable` trait that enables cross-thread access for
binding types. We don't enable it because:

1. **Arena correctness.** `bumpalo::Bump` is `!Sync` — the per-page
   allocator state isn't atomic. Marking it `Sendable` from PyO3 would
   require a mutex around every allocation, which is the cost we
   designed the arena to avoid in the first place.
2. **GIL semantics.** Python threads share the GIL; "concurrent" in
   the Python sense is rarely actually parallel. The `unsendable`
   marker turns the misuse case into a loud `RuntimeError` instead of
   a silent data race.
3. **Multiprocessing path.** The right answer for parallel corpus
   work is `multiprocessing` (one `Document` per process — the
   arenas are independent by construction). The `unsendable` marker
   nudges users toward this.

## Why JSON-encoded diagnostics?

Same reason as the [WASM binding](wasm.md#why-a-hand-written-json-projection-over-serde-wasm-bindgen):

- The wire shape is stable across every binding.
- Avoids forcing a `pyclass` declaration on every diagnostic-related
  type.
- Downstream Python consumers `json.loads()` once and work with
  native dicts — no second translation.

The `diagnostics()` method returns a `str`, not a `list[dict]`, so
the `json.loads` is *visible* to the caller. Hiding it behind a
PyO3 `Vec<PyDict>` mapping would silently allocate one Python
object per diagnostic per call.

## Wheel distribution

aozora-py is not yet on PyPI — public release tracks the v1.0
freeze of the core library. Until then, build wheels locally:

```sh
maturin build -F extension-module --release  # → target/wheels/*.whl
pip install target/wheels/aozora_py-*.whl
```

Pre-1.0 distribution will likely use `cibuildwheel` to ship wheels
for every supported `(python, target)` combination — that's the
mainstream path for PyO3 projects in 2026.

## See also

- [Install → Python](../getting-started/install.md#python)
- [Bindings → C ABI](c.md) — same diagnostics JSON shape.
- [PyO3 user guide](https://pyo3.rs/) — the binding framework.
