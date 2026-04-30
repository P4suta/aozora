# Summary

[Welcome](welcome.md)

---

# Getting Started

- [Install](getting-started/install.md)
- [CLI Quickstart](getting-started/cli.md)
- [Library Quickstart](getting-started/library.md)

# 青空文庫記法 Reference

- [Notation overview](notation/overview.md)
- [Ruby (｜青梅《おうめ》)](notation/ruby.md)
- [Bouten / bousen (傍点・傍線)](notation/bouten.md)
- [縦中横 (tate-chū-yoko)](notation/tcy.md)
- [Gaiji (外字 references)](notation/gaiji.md)
- [Kunten / kaeriten (訓点・返り点)](notation/kunten.md)
- [Indent & align containers (字下げ)](notation/indent.md)
- [Page & section breaks (改ページ・改丁)](notation/breaks.md)
- [Diagnostics catalogue](notation/diagnostics.md)

# Architecture

- [Pipeline overview](arch/pipeline.md)
- [Borrowed-arena AST](arch/arena.md)
- [Seven-phase lexer](arch/lexer.md)
- [SIMD scanner backends](arch/scanner.md)
- [Eytzinger sorted-set lookup](arch/veb.md)
- [Shift_JIS + 外字 resolver](arch/encoding.md)
- [HTML renderer & canonical serialiser](arch/renderer.md)
- [Crate map](arch/crates.md)

# Bindings

- [Rust library](bindings/rust.md)
- [WASM (wasm-pack)](bindings/wasm.md)
- [C ABI](bindings/c.md)
- [Python (PyO3 / maturin)](bindings/python.md)

# Performance

- [Release profile & PGO](perf/profile.md)
- [Profiling with samply](perf/samply.md)
- [Benchmarks (criterion)](perf/bench.md)
- [Corpus sweeps](perf/corpus.md)

# Reference

- [CLI reference](ref/cli.md)
- [API reference (rustdoc)](ref/api.md)
- [Environment variables](ref/env.md)

---

# Contributing

- [Development loop](contrib/dev.md)
- [Testing strategy](contrib/testing.md)
- [Release process](contrib/release.md)
