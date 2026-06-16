# aozora_py

Python bindings for [**aozora**](https://github.com/P4suta/aozora) — a
pure-functional Rust parser for **青空文庫記法** (Aozora Bunko notation):
ruby (`｜青梅《おうめ》`), bouten (`［＃「X」に傍点］`), 縦中横, 外字
references, kaeriten, indent / align containers, page breaks, and more.

The parser core is Rust (`#![forbid(unsafe_code)]`); this package is a
thin [PyO3](https://pyo3.rs/) / [maturin](https://www.maturin.rs/) wheel
around it — an in-process native module, no runtime process dependencies.

## Install

```sh
pip install aozora_py
```

Wheels ship as a single **`cp311-abi3`** build per platform (CPython
stable ABI), so one wheel covers CPython **3.11, 3.12, 3.13, 3.14** and
every future 3.x. The package is typed (`py.typed`, PEP 561).

## Quickstart

```python
import aozora_py

doc = aozora_py.Document("｜青梅《おうめ》の街")

doc.to_html()        # → semantic HTML5 with <ruby>…</ruby>
doc.serialize()      # → re-emit Aozora source text

# Inspection — parsed native objects (list[dict]):
doc.diagnostics()       # lexer diagnostics
doc.nodes()             # classified node spans (ruby / bouten / gaiji / …)
doc.pairs()             # matched open/close pair links
doc.container_pairs()   # indent / warichu / keigakomi / alignEnd containers

# …or the raw, byte-identical wire envelope strings shared with the
# WASM / FFI / Go drivers ({"schema_version": 1, "data": [...]}):
doc.diagnostics_json()
doc.nodes_json()

# One-shot parse + render:
aozora_py.parse_to_html("｜青梅《おうめ》")
```

## Shift_JIS source

Real 青空文庫 archive files are `Shift_JIS`. Hand bytes to
`Document.from_sjis` (it auto-detects `Shift_JIS` vs UTF-8), or decode
explicitly:

```python
raw = open("souseki.txt", "rb").read()      # Shift_JIS archive bytes
doc = aozora_py.Document.from_sjis(raw)
print(doc.to_html())

# Decode without parsing:
text = aozora_py.decode_sjis(raw)
```

## Performance

`aozora_py.prewarm()` forces one-time parser-table initialisation off the
first-parse critical path — call it once at startup in batch workloads
that parse many short documents.

## Threading

A `Document` is pinned to the thread that created it (the parser owns a
bump arena with interior mutability). Touching one from another thread
raises `RuntimeError` rather than sharing unsoundly — construct a fresh
`Document` per thread.

## Links

- **Source & issues:** <https://github.com/P4suta/aozora>
- **Handbook:** <https://p4suta.github.io/aozora/>
- **Changelog:** <https://github.com/P4suta/aozora/blob/main/CHANGELOG.md>

## License

`Apache-2.0 OR MIT`, at your option.
