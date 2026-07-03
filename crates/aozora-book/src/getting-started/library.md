# Library Quickstart

The minimal Rust use of aozora is six lines:

```rust,no_run
# extern crate aozora;
use aozora::Document;

fn main() {
    let source = std::fs::read_to_string("src.txt").unwrap();
    let doc = Document::new(source);
    let tree = doc.parse();
    println!("{}", tree.to_html());
}
```

That's enough to get HTML out of any UTF-8 青空文庫 source. The rest
of this page covers the lifetime model, the diagnostic stream, and
the AST walk — three things you'll need once you do anything beyond
"render to HTML".

## The lifetime model

`Document` owns the source `Box<str>` (plus a `Copy` diagnostic
policy). `Document::parse` returns a `Tree<'a>` that owns all its AST
data (an `OwnedLexOutput`) and borrows only the source:

```rust
# extern crate aozora;
# let source = String::from("｜青空《あおぞら》文庫");
let doc  = aozora::Document::new(source);   // Document: 'static
let tree = doc.parse();                     // Tree<'_> bound to &doc
let html = tree.to_html();                  // walks the borrow
# let _ = (&tree, &html);

// the tree owns its AST (a flat NodeStore); dropping doc frees the source
drop(doc);
```

That is: **hand the `Document` around, not the `tree`**. If you need
to keep a parse result alive across function boundaries, the function
takes ownership of (or borrows) the `Document`, and re-derives the
`tree` on the inside. This is unusual for Rust libraries — most parse
APIs hand back an owned tree — but it's what keeps aozora's
source-borrowing `Tree` safe. See [Architecture → Owned AST & NodeStore](../arch/arena.md)
for why this trade is worth it (and the owned escape hatch when you need
to outlive the `Document`).

## Shift_JIS input

Aozora Bunko ships its corpus as Shift_JIS. Decode through the umbrella
`aozora::encoding` module first (consumers depend on `aozora` alone —
never on the internal `aozora-encoding` crate directly):

```rust,no_run
# extern crate aozora;
# fn main() -> Result<(), Box<dyn std::error::Error>> {
use aozora::Document;
use aozora::encoding::decode_sjis;

let bytes = std::fs::read("src.sjis.txt")?;
let utf8  = decode_sjis(&bytes)?;   // -> String; Err(DecodeError) on bad input
let doc   = Document::new(utf8);
let tree  = doc.parse();
# let _ = tree;
# Ok(()) }
```

`decode_sjis` handles BOM stripping, JIS X 0213 codepoints, and the
Aozora-specific 外字 references that survive the decode pass as
private-use sentinels (resolved later in the parser). It is *strict* —
malformed bytes return `Err(DecodeError)` rather than silently
substituting replacement characters. A runnable version is
`just example sjis`.

## Diagnostics

```rust
# extern crate aozora;
# let doc = aozora::Document::new("｜青空《あおぞら》文庫");
# let tree = doc.parse();
use aozora::Diagnostic;

let diags: &[Diagnostic] = tree.diagnostics();
for d in diags {
    let span = d.span();
    // `Diagnostic` is an enum — reach its parts through the accessors.
    // `Display` ({d}) renders the human message; there is no `.message`.
    eprintln!("[{:?}] {} @ {}..{}", d.severity(), d.code(), span.start, span.end);
}
```

Each `Diagnostic` carries a stable `code()`, a `span()`, and a
`severity()` (Error / Warning / Note). A runnable version is
`just example diagnostics`.
Diagnostics are *non-fatal* by design: the parser always produces a
tree, even from malformed input. Callers that want strict behaviour
treat any diagnostic as an error themselves. See the
[Diagnostics catalogue](../notation/diagnostics.md) for the code list.

## Walking the AST

`Tree::source_nodes()` returns a source-ordered side table — one
`SourceNode` per classified Aozora / container span (plain-text runs
between constructs round-trip verbatim and are not listed). It is the
surface editor tooling uses for semantic tokens and document symbols:

```rust
# extern crate aozora;
# let doc = aozora::Document::new("｜青空《あおぞら》文庫");
# let tree = doc.parse();
for entry in tree.source_nodes() {
    let span = entry.source_span;            // byte range into the source
    // `entry.node` is a `NodeRef`: Inline / BlockLeaf / BlockOpen /
    // BlockClose, each wrapping the owned AST node or container kind.
    println!("{}..{}  {:?}", span.start, span.end, entry.node);
}
```

Match on `entry.node` (`NodeRef`) to destructure a specific construct —
e.g. `NodeRef::Inline(Node::Ruby(r))` gives you the ruby base and
reading. A runnable version is `just example walk_ast`.

The owned nodes are cheap to copy — they're `Copy` tagged unions of
scalars and `u32` handles into the tree's `NodeStore`, not pointers —
so you can copy them out freely and resolve their text through the
store for as long as the `Tree` lives.

## Round-trip and canonicalisation

Every parse should round-trip:

```rust
# extern crate aozora;
# let doc = aozora::Document::new("青梅《おうめ》");
let parsed = doc.parse();
let canonical: String = parsed.to_source();
assert_eq!(canonical, doc.source());     // for *canonical* input (bare ruby)
```

Real Aozora Bunko sources contain stylistic variations (CRLF vs LF,
NFC vs NFD around accents, half-width vs full-width punctuation) that
the lexer normalises before tokenising. For those the assertion above
holds *after* `aozora fmt` has been applied once.

The pure round-trip property is what `aozora fmt --check` exercises in
CI, and what the corpus sweep verifies across the full Aozora Bunko
catalogue (~17 000 works).

## Where to next

- [Notation reference](../notation/overview.md) for what each node
  type represents.
- [Architecture → Pipeline overview](../arch/pipeline.md) for what
  happens between `Document::new` and `Document::parse`.
- [API reference](../ref/api.md) for the rustdoc-generated surface.
