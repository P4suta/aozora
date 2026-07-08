# Gaiji (外字 references)

Aozora Bunko predates ubiquitous Unicode support; many works still
ship as Shift_JIS source. Characters that don't fit in Shift_JIS —
JIS X 0213 plane-2 ideographs, accented Latin letters, ad-hoc
combining marks — appear in source as **gaiji references**:

```text
※［＃「魚＋師のつくり」、第3水準1-94-37］
※［＃「彳＋寺」、U+5F85、393-13］
※［＃濁点付き片仮名ヰ］
```

The leading `※` (U+203B, reference mark) opens the annotation; the
`［＃…］` body describes the character in three orthogonal ways:

1. A **descriptive name** in Japanese (`「魚＋師のつくり」` —
   "魚 plus the right-hand side of 師") for human readers.
2. A **JIS X 0213 plane / row / cell** triple
   (`第3水準1-94-37` — plane 1, row 94, cell 37).
3. A **Unicode codepoint** (`U+5F85`) when the character has one.

aozora resolves gaiji references through a compile-time
[PHF](https://docs.rs/phf/latest/phf/) lookup table built from the JIS X 0213
official mapping plus the Unicode UCS register, with the descriptive
name as a tertiary fallback.

## Why a compile-time table?

The gaiji table has ~14 000 entries. Loading it at runtime from a
JSON / TOML asset would:

- Add a startup cost on every `Document::new` (the parser is supposed
  to start reading bytes within microseconds).
- Force every binding (CLI, WASM, FFI, Python wheel) to ship the
  table as a separate asset, complicating distribution.
- Defeat dead-code elimination — the linker can't strip entries the
  consumer's input never references if they're loaded behind an
  opaque file read.

A `phf::Map` baked into the binary at compile time wins on every
axis: zero-allocation lookup, single-binary distribution, full
DCE and LTO visibility. The build cost is real (~40 s the first
time, ~0 s incremental) but happens once per workspace build, not
per-invocation.

`phf` over `static HashMap` (which would require runtime construction
in a `OnceLock`): `phf` produces a true compile-time perfect-hash
table — `O(1)` lookup with no first-call cost and no synchronisation
on the hot path.

## Resolution order

For a reference like `※［＃「魚＋師のつくり」、第3水準1-94-37］`:

1. **Unicode codepoint** if the source explicitly provided one
   (`U+XXXX`) — used directly.
2. **JIS X 0213 plane-row-cell** lookup (`第N水準P-R-C`) — most
   ideographs land here.
3. **Descriptive name** — the parser ships a curated mapping plus a
   single-character fallback (a description that is itself one glyph
   resolves to it). A reference that matches none of these resolves to
   nothing: the [`aozora::lex::unresolved_gaiji`](diagnostics.md#unresolved-gaiji)
   warning fires and the gaiji renders as its description text.

## AST shape

```rust,ignore
pub struct Gaiji<'src> {
    /// Free-form description from the source (e.g. "魚＋師のつくり").
    pub description: &'src str,
    /// Resolved Unicode value — a single scalar or a static combining
    /// sequence — or `None` when no path matched.
    pub ucs: Option<Resolved>,
    /// Raw mencode reference (e.g. "第3水準1-85-54", "U+XXXX").
    pub mencode: Option<&'src str>,
}
```

`Resolved` is `Char(char)` for the 99%+ single-scalar case or
`Multi(&'static str)` for the 25 JIS X 0213 plane-1 combining-sequence
cells. `ucs == None` is the unresolved case the
[`unresolved_gaiji`](diagnostics.md#unresolved-gaiji) warning flags.

## Render output

| `ucs` | HTML |
|---|---|
| `Some(_)` | `<span class="aozora-gaiji" data-codepoint="U+20B9B">𠮛</span>` — the resolved glyph as content, the scalar(s) as space-separated `U+XXXX` in `data-codepoint`. |
| `None` | `<span class="aozora-gaiji" data-description="魚＋師のつくり">魚＋師のつくり</span>` — the description as both attribute and content. |

## Accent decomposition

Aozora Bunko also encodes accented Latin letters (è, ñ, ä) using a
separate notation that *does not* go through `※［＃…］`:

```text
M&iexcl;cher    ← in some sources
me-zin       ← in others
```

The full table is at
<https://www.aozora.gr.jp/accent_separation.html> — 114 ASCII
digraphs / ligatures mapping to Unicode. aozora applies this
decomposition during the lexer's sanitize stage, so by the time
classification runs the source is pure Unicode. See
[Architecture → Lexer (sanitize → tokenize → pair → classify)](../arch/lexer.md)
for the stage ordering.

## See also

- [Architecture → Shift_JIS + 外字 resolver](../arch/encoding.md) —
  the encoding pipeline and the PHF table internals.
- [Diagnostics → `aozora::lex::unresolved_gaiji`](diagnostics.md#unresolved-gaiji)
  — unresolved gaiji reference.
