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
[PHF](https://docs.rs/phf) lookup table built from the JIS X 0213
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
3. **Descriptive name** — the parser ships a curated mapping for the
   ~120 characters that have no JIS / Unicode codepoint at all.
   Misses fire diagnostic [`W0006`](diagnostics.md#W0006) and the
   gaiji is rendered as the descriptive text in `<span>` brackets.

## AST shape

```rust
pub struct Gaiji<'src> {
    pub description:    &'src str,        // 「魚＋師のつくり」
    pub jis:            Option<JisCode>,  // (plane, row, cell)
    pub unicode:        Option<char>,     // resolved codepoint
    pub resolution:     GaijiResolution,  // Direct | Lookup | Fallback
    pub span:           Span,
}

pub enum GaijiResolution {
    /// The source provided U+XXXX directly.
    Direct,
    /// Resolved via JIS table.
    Lookup,
    /// Could not resolve; rendered as descriptive text.
    Fallback,
}
```

## Render output

| Resolution | HTML |
|---|---|
| `Direct` / `Lookup` | the resolved codepoint inline, with a `data-aozora-gaiji-jis="1-94-37"` attribute for downstream analysis tools. |
| `Fallback` | `<span class="aozora-gaiji-fallback" title="魚＋師のつくり">[魚＋師のつくり]</span>` |

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
decomposition during the lexer's Phase 0 (sanitize), so by the time
classification runs the source is pure Unicode. See
[Architecture → Seven-phase lexer](../arch/lexer.md) for the phase
ordering.

## See also

- [Architecture → Shift_JIS + 外字 resolver](../arch/encoding.md) —
  the encoding pipeline and the PHF table internals.
- [Diagnostics → `W0006`](diagnostics.md#W0006) — unresolved gaiji
  reference.
