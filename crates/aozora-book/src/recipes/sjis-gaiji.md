# Shift_JIS & gaiji

**Problem.** Aozora Bunko ships its corpus as Shift_JIS, and those
files contain 外字 (gaiji) references like
`※［＃「木＋吶のつくり」、第3水準1-85-54］`. You want to decode the
bytes and see how each gaiji reference resolved.

## Two concerns, two layers

- **Encoding** is *not* the parser's job — the parser is strictly
  UTF-8. Decode Shift_JIS first with
  [`aozora::encoding`](../arch/encoding.md), then hand the resulting
  `String` to `Document::new`.
- **Gaiji resolution** *is* the parser's job. As it classifies a
  `※［＃…］` reference it resolves the mencode against the bundled JIS
  X 0213 tables, attaching the result to the `Gaiji` node. You read it
  off the node; you do not call the resolver yourself.

## Solution (library)

```rust,no_run
# extern crate aozora;
use aozora::{Document, Node, NodeRef};
use aozora::encoding::decode_sjis;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Decode the Shift_JIS archive file to UTF-8 (strict — errors on
    // malformed bytes rather than substituting replacement chars).
    let bytes = std::fs::read("crime_and_punishment.txt")?;
    let utf8 = decode_sjis(&bytes)?;

    let doc = Document::new(utf8);
    let tree = doc.parse();
    let store = &tree.lex_output().store;

    for sn in tree.source_nodes() {
        if let NodeRef::Inline(Node::Gaiji(g)) = sn.node {
            // The gaiji payload's strings are StrId handles into the store;
            // `resolve(store)` derives the glyph against the JIS tables.
            let hint = store.resolve_str(g.hint);
            match g.resolve(store).and_then(|r| r.as_char()) {
                Some(ch) => println!("{hint} → {ch}"),
                None => println!("{hint} → (unresolved)"),
            }
        }
    }
    Ok(())
}
```

## Expected output

```text
木＋吶のつくり → 吶
```

`Gaiji` carries `hint` (the free-form source description, a
`StrId`), `canonical` (the typed mencode value, e.g. `第3水準1-85-54`),
and `standalone`. The resolved glyph is derived on demand via
`resolve(store)`, which returns a `Resolved` (`None` when no table
matched). `Resolved` is either a single `Char` — recovered via
`as_char()` above — or a `Multi` combining sequence for the handful of
plane-1 cells that need one; see the
[Gaiji node chapter](../nodes/gaiji.md).

## Picking the decoder

`aozora::encoding` offers more than one entry point:

- `decode_sjis(&[u8]) -> Result<String, _>` — force Shift_JIS. Use it
  when you *know* the input is the canonical archive encoding.
- `decode_auto(&[u8]) -> Result<Cow<str>, _>` — sniff: valid UTF-8 is
  returned borrowed (zero-copy), otherwise the bytes decode as
  Shift_JIS. Use it for a mixed corpus where some files are
  pre-converted UTF-8 mirrors.

Both are strict — neither substitutes replacement characters — so you
learn when you are looking at corrupted source rather than silently
absorbing it.

## Solution (CLI)

The `aozora` binary decodes Shift_JIS with `-E sjis` (alias
`--encoding sjis`); the default is UTF-8:

```sh
aozora render -E sjis crime.txt > crime.html
aozora check  -E sjis crime.txt          # diagnostics on the decoded text
aozora pandoc -E sjis crime.txt -t epub > crime.epub
```

## See also

- Runnable example: **`just example sjis`**
  (`crates/aozora/examples/sjis.rs`).
- [Gaiji node reference](../nodes/gaiji.md) — the `Gaiji` struct and
  the `Resolved` shapes.
- [Gaiji notation](../notation/gaiji.md) — the `※［＃…］` reference
  syntax.
- [Shift_JIS + 外字 resolver](../arch/encoding.md) — the decode +
  resolution architecture.
- [Library Quickstart → Shift_JIS input](../getting-started/library.md#shift_jis-input).
