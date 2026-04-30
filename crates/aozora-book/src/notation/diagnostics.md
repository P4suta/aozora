# Diagnostics catalogue

aozora is *non-fatal by design*: the parser always produces a tree,
even from malformed input, and reports problems through structured
diagnostics that callers can choose to treat as errors. This page
lists every diagnostic the lexer can emit.

Each diagnostic carries:

- A stable **code** (`E0001`, `W0001`, …). The number suffix is
  permanent across versions; codes are added but never renumbered.
- A **level**: `Error`, `Warning`, `Info`.
- A **span** (byte range in the source).
- A **message** in English.
- (optional) a **help** line suggesting a fix.

The CLI renders diagnostics through [`miette`](https://docs.rs/miette);
all bindings (Rust library, FFI JSON, WASM JSON, Python list) carry
the same structured data.

## E-codes (errors)

### E0001 — empty ruby reading

```text
｜青梅《》
```

The base text is given but the reading inside `《…》` is empty.
**Fix:** provide a reading or remove the `｜` marker.

### E0002 — nested ruby

```text
｜青梅《｜お《お》うめ》
```

The spec disallows ruby inside ruby; the inner `｜…《…》` is
ambiguous. **Fix:** restructure so the readings are siblings, not
nested.

### E0004 — mismatched bouten container closer

```text
［＃ここから傍点］…［＃ここで傍線終わり］
```

The opener was a bouten variant; the closer was a bousen variant.
**Fix:** match the closer to the opener family (`傍点終わり` for any
点 variant; `傍線終わり` for any 線 variant).

### E0005 — mismatched container closer

```text
［＃ここから2字下げ］…［＃ここで地付き終わり］
```

Different container kinds. The parser auto-closes the offending
opener at the closer's position. **Fix:** match opener and closer.

### E0009 — bracketed kaeriten with no pair

```text
有［＃二］朋自遠方来    （［＃一］ missing）
```

The bracketed kaeriten form requires a paired closer. **Fix:** add
the matching `［＃一］` (or remove the `［＃二］`).

## W-codes (warnings)

### W0001 — tcy target not found

```text
昭和27年生まれ［＃「999」は縦中横］
```

The quoted run does not appear in the look-back window (current line
+ previous line, max 64 KiB). The directive is dropped. **Fix:**
quote a run that actually appears in the source.

### W0003 — bouten target ambiguous

```text
平和平和［＃「平和」に傍点］
```

Two candidate runs in the look-back window. The parser applies the
bouten to the *most recent* match (right-most in vertical / left-to-
right reading); `W0003` flags the ambiguity for the author to
disambiguate.

### W0006 — unresolved gaiji reference

The gaiji reference resolved to neither a Unicode codepoint nor a
JIS X 0213 entry, and no descriptive-name fallback applied. The
character is rendered as descriptive text in `<span>` brackets.
**Fix:** check the JIS triple, add the codepoint manually, or extend
the descriptive-name table.

### W0007 — kaeriten outside 漢文 context

```text
こんにちは レ
```

A kaeriten character (`レ`, `一`, `上`, …) appeared in a context
that doesn't look like 漢文 (no preceding kanji run, surrounded by
kana). The parser still emits the kaeriten node but flags the
suspicious placement.

### W0008 — break inside single-line container

```text
　［＃地付き］right-flushed［＃改ページ］
```

The page break terminates the single-line container before its
implicit end-of-line closer. The container is dropped from the
output.

### W0010 — unrecognised container directive

The `［＃ここから…］` directive matched no known container kind.
The parser emits a `Container::Unknown` and copies the directive
verbatim into the canonical-serialise output.

## I-codes (info)

### I0001 — accent decomposition applied

```text
M[i!]cher  →  Micher
```

Reported once per source for each distinct ASCII digraph that the
sanitize phase decomposed. Off by default; enable with
`--diagnostics info` on the CLI.

## Why a stable code, not just a message?

Two reasons.

1. **Test stability.** The corpus sweep counts diagnostics by code
   to detect parser regressions. A test like "the corpus emits at
   most 12 W0006 warnings" is robust against message wording
   tweaks; a test that greps the message string breaks every
   localisation pass.
2. **Tool integration.** Editors / LSPs / CI lints filter diagnostics
   by code (e.g. "treat E* as error, ignore W0010 for legacy
   files"). String matching there is fragile in practice.

The cost is a small lookup table (`code → message`); the win is
that diagnostics survive refactors and translation.

## See also

- [Library Quickstart → Diagnostics](../getting-started/library.md)
- [CLI Quickstart → Diagnostics format](../getting-started/cli.md)
- [Architecture → Seven-phase lexer](../arch/lexer.md) — which
  phase emits which code.
