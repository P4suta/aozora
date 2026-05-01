# AST query DSL

Phase N5 introduces a tree-sitter-flavoured pattern DSL that selects
nodes / tokens from the [concrete syntax tree](arch/cst.md). Editor
surfaces (LSP `textDocument/documentHighlight`, "find all ruby
annotations", refactoring filters, syntax-aware search) compose
against the DSL instead of re-implementing tree walks.

The DSL ships behind the `query` Cargo feature on the `aozora`
crate; that feature also enables `cst` since queries run against
`SyntaxNode`.

## Quickstart

```rust,ignore
use aozora::Document;
use aozora::query::compile;

let doc = Document::new("｜青梅《おうめ》と｜青空《あおぞら》");
let cst = aozora::cst::from_tree(&doc.parse());
let query = compile("(Construct @ruby)").expect("compile");
for capture in query.captures(&cst) {
    println!("{} -> {:?}", capture.name, capture.node);
}
```

## Grammar (0.4.0)

```text
query   := pattern ('\n' pattern)* '\n'?
pattern := '(' kind capture? ')'
         | '(' '_'  capture? ')'
kind    := SyntaxKind ident      // e.g. `Construct`, `Container`
capture := '@' ident
ident   := [A-Za-z_][A-Za-z0-9_-]*
```

- `(Construct)` — match every `Construct` node.
- `(Construct @ruby)` — capture each `Construct` under the name `ruby`.
- `(_)` — match any kind (node or token).
- `(_ @any)` — combined; tour every kind in preorder.
- Multiple patterns separated by newlines run as an OR — every
  matching node yields one [`Capture`](https://docs.rs/aozora-query/latest/aozora_query/struct.Capture.html)
  per pattern that hits.

## Smarter than naive

Routes considered and rejected:

1. **Walk the tree from scratch for every query** — quadratic in
   tree size × pattern count. The current implementation compiles
   the DSL once into a `Vec<Pattern>` and tests every pattern at
   every preorder step (`O(nodes × patterns)`).
2. **Defer the DSL until 0.5.0** — leaves consumers to hand-roll
   walkers. Phase N5's small surface (capture-only) is already
   useful enough to ship while the predicate / field-access /
   alternation extensions wait for a concrete consumer ask.

## What's not in 0.4.0

- Predicates (`#eq?`, `#match?`) — the tree-sitter query language
  exposes per-capture filters. We ship without them; consumers can
  filter the resulting [`Capture`] vec in Rust.
- Field accessors (`(Container body: (Construct))`) — the CST has
  no named fields yet.
- Quantifiers (`(...)?`, `(...)*`, `(...)+`).
- Alternation `[...]` between patterns.

These extensions are forward-compatible with the existing API
shape (`compile` → `captures`); a 0.5+ release lands them without
breaking existing queries.

## Cross-references

- [Architecture → Concrete syntax tree](arch/cst.md) — the CST the
  DSL queries.
- [Node reference](nodes/index.md) — `NodeKind` / `SyntaxKind`
  documentation.
