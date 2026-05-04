# Concrete syntax tree (CST)

A [rowan][rowan]-backed lossless syntax tree lives under the `cst`
Cargo feature on the `aozora` crate. The CST is a **pure projection**
over the existing parse output — Phase 3 classification is unchanged,
the AST stays the perf-critical path, and the CST adds zero overhead
for consumers that don't enable the feature.

## Why a CST exists

The borrowed AST (`AozoraNode<'src>`) is great for renderers:
classified spans, typed payload, no whitespace noise. It is the wrong
shape for **source-faithful tooling**:

- A formatter rewriting `日本《にほん》` → `｜日本《にほん》` needs the
  exact whitespace and trivia between tokens.
- A LSP `textDocument/foldingRange` provider needs the open / close
  positions of every nestable region, including ones the renderer
  ignores.
- A refactor that renames a kanji-range `［＃「青空」に傍点］` to
  `［＃「あおぞら」に傍点］` must preserve every bracket character
  the user wrote, not just the parsed `target`.

A CST whose leaves concatenate to the parser's input gives those
tools what they need without any custom plumbing.

## Lossless invariant

The contract is sharp:

> Concatenating every leaf token's text yields the **sanitized**
> source bytes the parser actually saw.

"Sanitized" matters: Phase 0 normalises CRLF→LF, strips a leading
BOM, isolates long decorative rule lines with a leading blank line,
and rewrites `〔…〕` accent spans through accent decomposition. These
transformations happen *before* classification, so `source_nodes`
coordinates address sanitized bytes. The CST tracks that coordinate
system; an editor that wants to map back to the user's raw bytes
runs the same Phase 0 transformation and inverts where needed.

The proptest in `tests/property_lossless.rs` runs the invariant
across the full Aozora-shaped input distribution
(`aozora_fragment` / `pathological_aozora` /
`unicode_adversarial` from `aozora-proptest`). A regression here
breaks every editor surface that walks the CST.

## Architecture

The crate stays decoupled by design:

- `aozora-cst` depends on `aozora-pipeline` + `aozora-spec` directly,
  **not** on the `aozora` meta crate. Going through `aozora` would
  create a cycle (the meta crate's `cst` feature re-exports
  `aozora-cst`).
- `build_cst(sanitized_source, source_nodes) -> SyntaxNode` takes
  the lower-level bits explicitly so consumers writing custom
  pipelines can reach in.
- `aozora::cst::from_tree(&tree) -> SyntaxNode` is the ergonomic
  entry point; it runs Phase 0 sanitize internally and forwards.
- The Phase 3 classifier sees no changes — adding / removing CST
  consumers cannot perturb AST perf.

## SyntaxKind granularity

The CST is intentionally coarser than a token-stream
re-construction:

| `SyntaxKind`      | Role                                                   |
| ----------------- | ------------------------------------------------------ |
| `Document`        | Tree root                                              |
| `Container`       | Paired-container region (`［＃ここから...］...［＃ここで...終わり］`) |
| `Construct`       | Single classified Aozora construct                     |
| `ContainerOpen` / `ContainerClose` | Container boundary tokens             |
| `ConstructText`   | Source slice of a `Construct`                          |
| `Plain`           | Plain text run between classifications                 |

Finer per-token granularity (individual punctuation, kana runs, …)
can land later once a concrete consumer needs it. The lossless
property holds at any granularity, so widening the leaf set is
non-breaking for downstream tooling that walks `preorder_with_tokens`.

## Why rowan, not Phase 3 integration

The bumpalo-arena AST stays the hot path; the CST sits on top as an
editor-grade convenience layer rather than coupling lossless-tree
concerns into the perf-critical classifier. rowan (over cstree)
gives the lossless tree a maintained home — rust-analyzer's tree
infrastructure with 86 reverse deps — and the bumpalo / Arc
dual-allocator overhead is the price for keeping the AST untouched.

## Cross-references

- [Architecture → Borrowed-arena AST](arena.md) — the underlying
  perf-critical tree.
- [Architecture → Four-phase lexer](lexer.md) — where Phase 0
  sanitize and Phase 3 classify do their work.
- [`Document::edit`](https://docs.rs/aozora/latest/aozora/struct.Document.html#method.edit)
  — the incremental-parse counterpart that reuses the same CST.

[rowan]: https://docs.rs/rowan
