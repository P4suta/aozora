# Notations in host literal contexts

**Problem.** You embed `aozora` inside a larger grammar — a CommonMark/GFM host
like [aozora-flavored-markdown](https://github.com/P4suta/aozora-flavored-markdown).
aozora is host-blind: it lexes the whole text and collapses each notation into a
private-use sentinel *before* the host parser runs. When the host then routes a
sentinel into one of its **literal** contexts — an inline code span `` `…` `` or
a link / image destination `[…](…)` — the notation must appear as its
**original source**, not be interpreted. A host that resolves only the sentinels
it finds in "normal" text will leak the raw sentinel into those literal fields
and desync its registry cursor, corrupting every later notation.

## Solution

Resolve *every* sentinel the host emits — including the ones it placed in a
literal field — and recover the author's original bytes from
`Tree::source_nodes` + `Span::slice`. `source_nodes()` is source-ordered and
parallel to the registry, so each `SourceNode::source_span` slices straight back
to the text the author typed.

```rust
# extern crate aozora;
use aozora::Document;

fn main() {
    // Two notations; imagine the host parsed the second inside a code span,
    // where it must be shown verbatim rather than resolved.
    let source = "冒頭｜青梅《おうめ》。［＃改ページ］次の章。";
    let doc = Document::new(source);
    let tree = doc.parse();

    for sn in tree.source_nodes() {
        // The exact bytes the author typed for this notation. A host that put
        // the sentinel into a literal field emits THIS, unchanged, instead of
        // the resolved / interpreted form.
        let original = sn.source_span.slice(source);
        println!("{original}");
    }
}
```

## Expected output

```text
｜青梅《おうめ》
［＃改ページ］
```

## Why this works

`aozora` keeps two parallel views of the parse:

- the **registry** (`Tree::node_at` / `lex_output().registry`), keyed by
  *normalized* PUA-rewritten coordinates — what a renderer walks; and
- **`source_nodes()`**, the same constructs keyed by *source* coordinates, each
  carrying a `source_span` into the original document.

A host integrates by walking its own parse, and for each sentinel it encounters
(in normal text *or* a literal field) looking up the matching node. In a literal
context it does not render the resolved node — it emits
`source_span.slice(source)`, the verbatim original. Because `source_nodes()` is
sorted by `source_span.start` and runs parallel to the registry, a host can zip
the two by position; the source slice is always available without re-parsing.

The contract is documented on
[`Tree::source_nodes`](https://docs.rs/aozora/latest/aozora/struct.Tree.html#method.source_nodes):
a host with literal contexts must resolve every sentinel and can always recover
the original via `source_span` + `Span::slice`.

## See also

- [Walk the AST](walk-ast.md) — the same `source_nodes()` walk, by node kind.
- [Byte-exact round-trip](round-trip.md) — `Tree::to_source` /
  `to_source_verbatim` for whole-document recovery.
- [Owned AST & NodeStore](../arch/arena.md) — how the owned AST relates
  to the `Document`'s source lifetime.
