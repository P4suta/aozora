# Summary

[Welcome](welcome.md)

---

# Getting Started

- [Install](getting-started/install.md)
- [CLI Quickstart](getting-started/cli.md)
- [Library Quickstart](getting-started/library.md)

# 青空文庫記法 Reference

- [Node reference](nodes/index.md)
  - [Ruby](nodes/ruby.md)
  - [Bouten](nodes/bouten.md)
  - [CombineUpright](nodes/tate-chu-yoko.md)
  - [Gaiji](nodes/gaiji.md)
  - [Indent](nodes/indent.md)
  - [AlignEnd](nodes/align-end.md)
  - [Warichu](nodes/warichu.md)
  - [Framed](nodes/keigakomi.md)
  - [PageBreak](nodes/page-break.md)
  - [SectionBreak](nodes/section-break.md)
  - [Heading](nodes/aozora-heading.md)
  - [HeadingHint](nodes/heading-hint.md)
  - [Illustration](nodes/sashie.md)
  - [Kaeriten](nodes/kaeriten.md)
  - [Directive](nodes/annotation.md)
  - [AngleQuote](nodes/angle-quote.md)
  - [Container](nodes/container.md)
  - [ContainerOpen](nodes/container-open.md)
  - [ContainerClose](nodes/container-close.md)
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
- [Concrete syntax tree](arch/cst.md)
- [Error recovery](arch/error-recovery.md)
- [tree-sitter reference grammar](arch/grammar-tree-sitter.md)
- [Crate map](arch/crates.md)

# Bindings

- [Choosing a binding](bindings/choosing.md)
- [Rust library](bindings/rust.md)
- [WASM (wasm-pack / npm)](bindings/wasm.md)
- [Python (PyO3 / maturin)](bindings/python.md)
- [Go (wazero host SDK)](bindings/go.md)
- [C ABI](bindings/c.md)
- [Extism host SDKs (Java / PHP / Ruby / …)](bindings/extism.md)
- [Pandoc AST projection](bindings/pandoc.md)

# Recipes

- [Recipes overview](recipes/index.md)
  - [Extract ruby readings](recipes/extract-ruby.md)
  - [Diagnostics as JSON](recipes/diagnostics-json.md)
  - [Walk the AST](recipes/walk-ast.md)
  - [Shift_JIS & 外字 input](recipes/sjis-gaiji.md)
  - [EPUB via Pandoc](recipes/epub-pandoc.md)
  - [Byte-exact round-trip](recipes/round-trip.md)
  - [Polyglot host integration](recipes/polyglot.md)
  - [Batch many files](recipes/batch.md)

# Performance

- [Release profile & PGO](perf/profile.md)
- [Profiling with samply](perf/samply.md)
- [Benchmarks (criterion)](perf/bench.md)
- [Corpus sweeps](perf/corpus.md)
- [Phase D results — single-table registry](perf/phase-d-results.md)

# Reference

- [CLI reference](ref/cli.md)
- [API reference (rustdoc)](ref/api.md)
- [Environment variables](ref/env.md)
- [Conformance suite](conformance.md)
- [Query DSL](query.md)
- [Wire format](wire/overview.md)

---

# Contributing

- [Your first PR](contrib/first-pr.md)
- [Development loop](contrib/dev.md)
- [Testing strategy](contrib/testing.md)
- [Troubleshooting & gate recovery](contrib/troubleshooting.md)
- [Release process](contrib/release.md)
