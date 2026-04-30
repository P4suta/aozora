# Library Quickstart

The minimal Rust use of aozora is six lines:

```rust
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

`Document` owns two things: a [`bumpalo::Bump`](https://docs.rs/bumpalo)
arena and the source `Box<str>`. `AozoraTree<'a>` borrows from both:

```rust
let doc  = aozora::Document::new(source);   // Document: 'static
let tree = doc.parse();                     // AozoraTree<'_> bound to &doc
let html = tree.to_html();                  // walks the borrow

// dropping doc releases every node in a single Bump::reset()
drop(doc);
```

That is: **hand the `Document` around, not the `tree`**. If you need
to keep a parse result alive across function boundaries, the function
takes ownership of (or borrows) the `Document`, and re-derives the
`tree` on the inside. This is unusual for Rust libraries — most parse
APIs hand back an owned tree — but it's what makes aozora's
zero-copy AST safe. See [Architecture → Borrowed-arena AST](../arch/arena.md)
for why this trade is worth it.

## Shift_JIS input

Aozora Bunko ships its corpus as Shift_JIS. Decode through
`aozora-encoding` first:

```rust
use aozora::Document;
use aozora_encoding::sjis;

let bytes = std::fs::read("src.sjis.txt")?;
let utf8  = sjis::decode_to_string(&bytes)?;   // returns Cow<'_, str>
let doc   = Document::new(utf8.into_owned());
let tree  = doc.parse();
```

`sjis::decode_to_string` handles BOM stripping, JIS X 0213 codepoints,
and the Aozora-specific 外字 references that survive the decode pass
as private-use sentinels (resolved later in the parser).

## Diagnostics

```rust
use aozora::Diagnostic;

let diags: &[Diagnostic] = tree.diagnostics();
for d in diags {
    eprintln!("[{}] {} @ {}..{}", d.code, d.message, d.span.start, d.span.end);
}
```

Each `Diagnostic` carries a stable error code, a span, and a level.
Diagnostics are *non-fatal* by design: the parser always produces a
tree, even from malformed input. Callers that want strict behaviour
treat any diagnostic as an error themselves. See the
[Diagnostics catalogue](../notation/diagnostics.md) for the code list.

## Walking the AST

`AozoraTree` exposes a flat node iterator and a typed enum:

```rust
use aozora::AozoraNode;

for node in tree.nodes() {
    match node {
        AozoraNode::Plain(s)    => print!("{s}"),
        AozoraNode::Ruby(r)     => print!("[ruby:{}={}]", r.target(), r.reading()),
        AozoraNode::Bouten(b)   => print!("[bouten {}]", b.kind().slug()),
        AozoraNode::Tcy(t)      => print!("[tcy:{}]", t.text()),
        AozoraNode::Gaiji(g)    => print!("[gaiji {}]", g.codepoint()),
        AozoraNode::Container(c)=> { /* recurse into c.children() */ }
        // …
    }
}
```

For richer traversal patterns (visitor, fold, structural diff), the
nodes implement `Copy` (they're effectively `(tag, &str, &Bump-slice)`
triples), so you can keep references around freely as long as the
`Document` lives.

## Round-trip and canonicalisation

Every parse should round-trip:

```rust
let parsed = doc.parse();
let canonical: String = parsed.serialize();
assert_eq!(canonical, doc.source());     // for *canonical* input
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
