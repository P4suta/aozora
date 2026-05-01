# AST query DSL

A tree-sitter-flavoured pattern DSL selects nodes / tokens from the
[concrete syntax tree](arch/cst.md). Editor surfaces (LSP
`textDocument/documentHighlight`, "find all ruby annotations",
refactoring filters, syntax-aware search) compose against the DSL
instead of re-implementing tree walks.

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

## Grammar

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

## Execution model

The DSL compiles once into a `Vec<Pattern>`; the engine then tests
every pattern at every preorder step (`O(nodes × patterns)`). The
small capture-only surface keeps the implementation tight while the
predicate / field-access / alternation extensions wait for a
concrete consumer ask.

## Not yet supported

- Predicates (`#eq?`, `#match?`) — the tree-sitter query language
  exposes per-capture filters. The DSL ships without them; consumers
  filter the resulting [`Capture`] vec in Rust.
- Field accessors (`(Container body: (Construct))`) — the CST has
  no named fields yet.
- Quantifiers (`(...)?`, `(...)*`, `(...)+`).
- Alternation `[...]` between patterns.

These extensions are forward-compatible with the existing API
shape (`compile` → `captures`); a future release can land them
without breaking existing queries.

## Cross-references

- [Architecture → Concrete syntax tree](arch/cst.md) — the CST the
  DSL queries.
- [Node reference](nodes/index.md) — `NodeKind` / `SyntaxKind`
  documentation.
