# Welcome

**aozora** is a pure-functional Rust parser for **青空文庫記法** (Aozora
Bunko notation) — the in-text annotation language used by
[青空文庫](https://www.aozora.gr.jp/), the long-running volunteer
digital library of Japanese literature in the public domain.

It handles ruby (`｜青梅《おうめ》`), bouten / bousen
(`［＃「X」に傍点］`), 縦中横, gaiji references
(`※［＃…、第3水準1-85-54］`), kunten / kaeriten, indent and align
containers (`［＃ここから2字下げ］… ［＃ここで字下げ終わり］`), and
page / section breaks — every notation that appears in a real Aozora
Bunko `.txt` source.

The repository is **CommonMark-free, Markdown-free**: aozora deals
*only* with the 青空文庫 notation. The renderer emits semantic HTML5;
the lexer reports structured diagnostics; the AST is a borrowed-arena
tree that can be walked in O(n) without copying source bytes. If you
want a Markdown dialect that *also* understands aozora notation, see
the sibling project [afm](https://github.com/P4suta/afm), which is
built on top of this parser.

## What this handbook is for

A practical tour and a deep reference, in one document.

- **Tour** — install the [CLI](getting-started/cli.md), drop the
  [library](getting-started/library.md) into a Rust project, or call
  it from [WASM](bindings/wasm.md), [C](bindings/c.md), or
  [Python](bindings/python.md).
- **Notation reference** — every annotation aozora recognises, with
  examples, output, edge cases, and the diagnostics that fire when
  authors get them subtly wrong.
- **Architecture** — what makes aozora *fast and small*: the
  [borrowed-arena AST](arch/arena.md), the
  [seven-phase lexer](arch/lexer.md), the
  [SIMD scanner backends](arch/scanner.md) (Teddy, structural
  bitmaps, Hoehrmann-style multi-pattern DFA),
  [Eytzinger-layout sorted-set lookup](arch/veb.md), and the
  [Shift_JIS + 外字 resolver](arch/encoding.md). Every choice is
  motivated against the alternative we *didn't* take.
- **Performance** — the release-profile decisions, PGO pipeline,
  [samply](perf/samply.md) workflow, criterion
  [benchmarks](perf/bench.md), and the parallel
  [corpus sweep](perf/corpus.md) that exercises the parser against
  every Aozora Bunko work.
- **Reference & contributing** — CLI, env vars, rustdoc API, and how
  the dev loop / TDD policy / release pipeline fit together.

## Project shape

aozora is a **single-author, green-field project** that takes the
opportunity to reach for the *good* algorithm and data structure for
each problem rather than the obvious naive one. That orientation
permeates every chapter — when you read about the scanner or the
arena or the gaiji table, you'll see *why this technique* spelled
out, not just *what the code does*.

## Status

`v0.2.x` working set. The CLI, Rust library, WASM, C ABI, and Python
binding all build and pass the integration smoke tests in CI. Public
crates.io publication is gated on the v1.0 API freeze; in the
meantime, depend on a tagged commit (see
[Install](getting-started/install.md)).

A live build of this site lives at
<https://p4suta.github.io/aozora/>; the rustdoc API reference is
layered underneath at <https://p4suta.github.io/aozora/api/aozora/>.
