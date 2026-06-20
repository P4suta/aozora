# Round-trip & fmt --check

**Problem.** You want to confirm a file is already in canonical
Aozora form — or to canonicalise it — and to rely on parse ∘
serialize being lossless.

## The property

`Tree::serialize` re-emits Aozora source from the parsed tree.
The guarantee is a **fixed point**: parsing a *canonical* document and
serialising it returns the same bytes, and serialising again changes
nothing.

```rust
use aozora::Document;

fn main() {
    let source = "｜青梅《おうめ》";

    let once = Document::new(source).parse().serialize();
    let twice = Document::new(once.clone()).parse().serialize();

    assert_eq!(once, twice, "serialize is a fixed point");
    println!("{twice}");
}
```

## Expected output

```text
｜青梅《おうめ》
```

## Canonical vs. raw input

Real Aozora Bunko sources carry stylistic variation the lexer
normalises before tokenising — CRLF vs LF, NFC vs NFD around accents,
and the bare-vs-explicit ruby delimiter (`青梅《おうめ》` vs
`｜青梅《おうめ》`). For *raw* input, therefore:

```rust
// Not guaranteed for arbitrary raw input:
assert_eq!(Document::new(raw).parse().serialize(), raw);   // may differ

// Guaranteed: the SECOND pass is a fixed point.
let canonical = Document::new(raw).parse().serialize();
assert_eq!(Document::new(canonical.clone()).parse().serialize(), canonical);
```

The first `serialize()` *is* the canonical form (e.g. it always emits
the explicit `｜` ruby delimiter — see the
[Ruby node chapter](../nodes/ruby.md)); from there it is stable. This
fixed-point property is what the corpus sweep verifies across the full
~17 000-work catalogue.

## Solution (CLI)

`aozora fmt` is the round-trip at the shell. With `--check` it is a
read-only gate — exit `0` if the file is already canonical, `1` if it
would change:

```sh
aozora fmt --check src.txt        # CI gate: nonzero if not canonical
aozora fmt src.txt > out.txt      # write the canonical form to stdout
aozora fmt --write src.txt        # rewrite in place
cat src.txt | aozora fmt          # stdin → stdout
```

Exit codes: `0` on success (or no diff under `--check`), `1` on a
formatting mismatch under `--check`, `2` on a usage error. `aozora fmt
--check` is exactly what this project runs in CI to keep fixtures
canonical.

## See also

- Runnable example: **`just example round_trip`**
  (`crates/aozora/examples/round_trip.rs`).
- [Library Quickstart → Round-trip](../getting-started/library.md#round-trip-and-canonicalisation).
- [HTML renderer & canonical serialiser](../arch/renderer.md) — how
  the canonical form is defined.
- [CLI reference → `aozora fmt`](../ref/cli.md) — flags and exit
  codes.
