# 0036. An index-owned AST replaces the arena

- Status: accepted
- Date: 2026-07-15
- Deciders: @P4suta
- Tags: architecture, ast, incremental

## Context

This records a decision that shipped in the #237 incremental-reparse work and
was never written down. It is being recorded now because **two accepted ADRs
still name it as a future prerequisite**:

> gated behind the v0.5.0 release (#99), **the `!Sync` arena rework**, and a
> real consumer (the LSP `ParseCache`'s cache-hit path)
> — ADR-0018:97-98
>
> blocked on **the `!Sync` arena rework**. Deferred to a later phase.
> — ADR-0018:119

The rework landed. Nothing records that it did, so the chain reads as though the
work is still pending. Accepted ADRs are never edited (`docs/ADR_INDEX.md`), so
a new one is the only way to close the reference.

Earlier revisions backed the AST with a `bumpalo` arena and a borrowed tree
whose every node held a `&'src str` into it. That tree was `Copy` and fast, but
bound to one lifetime and therefore **not `Send + Sync`**: it could not outlive
its `Document`, could not be cached, and could not cross a thread.

#237 needed exactly the opposite. A long-lived consumer — the LSP `ParseCache`,
an out-of-process segment cache — has to **own, cache, and move** a parse.

## Decision

Replace the arena's pointers with `u32` handles into owned `Vec`s. The classify
stage builds owned nodes directly into `aozora_syntax::ast::NodeStore` (a string
interner plus flat content/segment pools); there is no intermediate borrowed
tree to convert from. `LexOutput` is lifetime-free and `Send + Sync`.

`Tree<'a>`'s remaining lifetime tracks the borrowed *source*, nothing else.

## Consequences

- **A parse can be cached and moved across threads**, which is the whole point:
  #237's incremental re-parse and the LSP's cache-hit path both become
  expressible. ADR-0018's deferred work is unblocked, and this ADR closes that
  reference.
- **Traded speed for ownership, knowingly.** The borrowed arena tree was `Copy`
  and quick. It was replaced for what it could not do, not for what it did
  slowly — a direction that looks like a regression to anyone reading a
  benchmark alone. The node shape stays `Copy` and cache-friendly, so the cost
  is bounded.
- **`bumpalo` survives only in `aozora-scan`**, for its `OffsetSink` impl. The
  parser has no arena.
- Every binding (CLI / WASM / FFI / Python), the splice layer, and the CST
  projection sit on this shape. It is not cheaply reversible.

## Alternatives considered

**Keep the arena; make the cache re-parse.** Defeats the purpose — the cache
exists so the common edit does not re-parse.

**Keep the arena; add a separate owned type for consumers that need one.** Two
AST shapes, one conversion between them, and every downstream feature choosing a
side. The conversion cost lands on exactly the path (cache hit) the work exists
to make fast.

**Make the arena `Sync`.** The lifetime, not the thread-safety, is the binding
constraint: a tree tied to `'src` cannot outlive its `Document` no matter what
its auto-traits say.

## References

- ADR-0018 / ADR-0019 — the deferred splice work this unblocks; the three
  "`!Sync` arena rework" references this closes.
- #237 — incremental re-parse; the consumer that forced it.
- `crates/aozora-pipeline/src/lib.rs` — "there is no intermediate borrowed
  tree"; the account that survived in the code.
