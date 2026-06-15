# Diagnostics catalogue

aozora is *non-fatal by design*: the parser always produces a tree, even
from malformed input, and reports what it noticed through structured
diagnostics that callers choose how to treat. This page is the catalogue.

Each `Diagnostic` carries:

- a stable **code** — a dotted string such as
  `aozora::lex::unclosed_bracket`. The string is pinned by a test and
  never changes within a major release; new diagnostics add new codes.
- a **severity**: `Error` / `Warning` / `Note`.
- a **source axis**: `Source` (your input tripped it) or `Internal` (a
  library-bug sanity check — see [Internal](#internal)).
- a **span** — a byte range in the *sanitized* source (the Phase 0
  output: BOM stripped, CRLF→LF, 〔…〕 accents decomposed). For input with
  none of those, the sanitized bytes equal the original bytes.

## Rendering them

The `aozora check` CLI renders diagnostics three ways, chosen with
`--diagnostic-format`:

- **`human`** (the default on a terminal) — a graphical
  [`miette`](https://docs.rs/miette) report: the source line, a caret
  under the offending span, the label, the help text, and a link back to
  this page.
- **`json`** (the default when stderr is piped) — the `aozora::wire`
  diagnostics envelope, byte-identical to what the WASM / FFI / Python /
  Extism front doors emit. This is the machine / agent path.
- **`short`** — one grep-able line per diagnostic:
  `path:offset: severity[code]: message`.

Exit codes: `0` (diagnostics printed but tolerated), `1` (`--strict` with
at least one diagnostic), `2` (CLI usage error), `3` (an `Internal`
diagnostic fired — a library bug). See the [CLI reference](../ref/cli.md).

Library consumers get `tree.diagnostics() -> &[Diagnostic]` and reach the
parts through `code()`, `severity()`, `source()`, and `span()`. All
bindings carry the same structured data.

# Source diagnostics

These trace back to your input. The parser emits exactly these today; the
[Planned diagnostics](#planned-diagnostics) section below tracks the
authoring-error diagnostics still on the roadmap.

## Source contains PUA

`aozora::lex::source_contains_pua` · **Warning**

```text
…￯…        (a literal U+E001..=U+E004 codepoint in the source)
```

The source contains a codepoint in `U+E001..=U+E004`, which the lexer
reserves as inline / block placeholder sentinels. A source-side
occurrence collides with the lexer's own markers and would confuse the
placeholder registry. **Fix:** remove the private-use codepoint from the
source (these are not normal text characters and effectively never occur
in real 青空文庫 files).

## Unclosed bracket

`aozora::lex::unclosed_bracket` · **Error**

```text
［＃ここから2字下げ            （no matching ［＃ここで字下げ終わり］）
```

An Aozora open delimiter (ruby `｜`, annotation `［＃`, quote, …) reached
end-of-input with no matching close on the pairing stack. The label
points at the *opener*. The region degrades to plain text — no pair link
is emitted. **Fix:** add the missing close delimiter, or remove the
dangling opener.

## Unmatched close

`aozora::lex::unmatched_close` · **Error**

```text
青空］》            （a close with no matching open on the stack）
```

A close delimiter was seen with an empty pairing stack, or against a
stack top of a different `PairKind`. The label points at the stray close.
**Fix:** add the matching open delimiter, or remove the stray close.

## Accent decomposition applied

`aozora::lex::accent_decomposition_applied` · **Note**

```text
〔cafe'〕        (decomposed to 〔café〕)
```

A `〔…〕` accent digraph was rewritten to its Unicode-combined form during
Phase 0 sanitize (`cafe'` → `café`, `fune` + backtick → `funè`, …). This is
**intended behaviour**, not an error — it is surfaced as a `Note` so an
editor can show *what* changed. One note fires per `〔…〕` span that actually
contained a digraph; a `〔…〕` with no accent digraph is silent. The span is
in sanitized (post-decomposition) coordinates. The transform is loss-free:
the serializer reconstructs the original `〔…〕` source form. See
[ADR-0003](https://github.com/P4suta/aozora/blob/main/docs/adr/0003-accent-decomposition-preparse.md).
**No action required.**

## Unresolved gaiji

`aozora::lex::unresolved_gaiji` · **Warning**

```text
※［＃「架空の外字」、第3水準99-99-99］   (men-ku-ten out of range)
```

A 外字 (gaiji) reference — `※［＃…］` — resolved to **neither** a Unicode
scalar **nor** a JIS X 0213 cell: no `第N水準P-R-C` men-ku-ten or `U+XXXX`
reference matched, and the description is not itself a single resolvable
character. The construct still parses; the renderer falls back to the
description text (`<span class="aozora-gaiji" data-description="…">…</span>`)
rather than the intended glyph. The label points at the `※［＃…］` reference.
**Fix:** correct the men-ku-ten / `U+XXXX` reference, or accept the
description-only rendering. (Fires for top-level references; gaiji nested
inside a ruby / bouten reading is not yet flagged.)

## Mismatched container close

`aozora::lex::mismatched_container_close` · **Error**

```text
［＃ここから2字下げ］…［＃ここで地付き終わり］   (indent opened, align-end closed)
```

A paired container opened with one family (`indent` / `warichu` /
`keigakomi` / `align-end`) was closed by a closer of a *different* family.
The comparison is by family, so closing a `2字下げ` opener with a plain
`字下げ終わり` (both `indent`, differing only in amount) is **not** flagged —
only a genuine family mismatch is. The label points at the close marker.
The parser recovers by auto-closing the opener at the closer's position
(the container pair is still emitted, keyed by the open family). **Fix:**
match the closer to the opener — `ここから字下げ` ↔ `ここで字下げ終わり`,
`ここから地付き` ↔ `ここで地付き終わり`, etc.

## Empty ruby reading

`aozora::lex::empty_ruby_reading` · **Error**

```text
｜青梅《》        (base given, reading empty)
```

An explicit-base ruby supplied a base (a `｜` precedes the `《`) but an
empty `《》` reading. Because the `｜` marks the base unambiguously, this is
a genuine authoring slip rather than a literal `《》` run — so a bare
`青梅《》` with **no** `｜` is *not* flagged (the parser can't be sure a base
was intended and treats it as text). The construct degrades to plain text.
The label spans the whole `｜青梅《》`. **Fix:** supply a reading, or drop
the `｜…《》` markers to keep the base as plain text.

## Nested ruby

`aozora::lex::nested_ruby` · **Error**

```text
｜漢《か《ん》じ》      (the reading body opens another 《…》)
```

A ruby reading body itself opened another ruby. Ruby does not nest; the
label points at the inner `《`. The outer ruby is still parsed
best-effort. Note that an *adjacent* `《《…》》` is **not** nested ruby — the
tokenizer reads `《《` / `》》` as [double-bracket bouten](bouten.md), a
separate construct — so this fires only when the inner `《…》` closes
before the outer (text between the two closes, as in the catalogue shape
`｜…《…《…》…》`). **Fix:** close the outer reading before the inner `《`, or
remove the inner `《…》`.

## Unrecognised container directive

`aozora::lex::unrecognised_container_directive` · **Warning**

```text
［＃ここからナントカ］      (no such container kind)
```

A `［＃ここから…］` directive looked like a paired-container opener but
named no known container kind (`字下げ`, `地付き`, `地から N 字上げ`). The
bracket is kept as a plain `Annotation{Unknown}` (so output is preserved
and the "no bare `［＃`" guarantee holds) but is **not** treated as a
container — any matching `［＃ここで…終わり］` will not pair with it. The
label spans the directive. **Fix:** use a recognised opener, e.g.
`［＃ここから2字下げ］` or `［＃ここから地付き］`.

## TCY target not found

`aozora::lex::tcy_target_not_found` · **Warning**

```text
あ［＃「い」は縦中横］      (no 「い」 earlier in the line)
```

A 縦中横 forward reference (`［＃「X」は縦中横］`) named a target that does
not appear anywhere in the preceding text, so it has no run to rotate. The
directive degrades to an `Annotation{Unknown}`. The label spans the
directive. **Fix:** check the spelling of the quoted target, or place the
`［＃「X」は縦中横］` after the run it should style.

## Bouten target ambiguous

`aozora::lex::bouten_target_ambiguous` · **Warning**

```text
青空青空［＃「青空」に傍点］      (「青空」 occurs twice before the directive)
```

A forward-reference bouten (`［＃「X」に傍点］`) named a target that occurs
**more than once** in the preceding look-back window, so which run it
emphasises is ambiguous. The parser still applies it (to the match its
look-back rule selects) but the chosen run may not be the intended one.
The label spans the directive. **Fix:** reword so the quoted target is
unique before the directive. (Multi-target brackets like `［＃「A」「B」に傍点］`
name distinct runs and are never flagged.)

# Internal

`aozora::internal` · **Error** · source = `Internal`

Pipeline-internal sanity checks. **A correct build never emits these** —
their appearance means a bug in aozora itself, not a problem with your
input. The specific check is identified by an `InternalCheckCode`:

| Check code | Fires when |
|---|---|
| `aozora::lex::residual_annotation_marker` | an `［＃` digraph survived classification into the normalized text (a missing recogniser) |
| `aozora::lex::unregistered_sentinel` | a PUA sentinel sits at a normalized position not recorded in the placeholder registry |
| `aozora::lex::registry_out_of_order` | a placeholder-registry vector is not strictly ordered by position |
| `aozora::lex::registry_position_mismatch` | a registry entry references a position whose character is not the expected sentinel |

`aozora check` exits `3` when one fires. Please
[report it](https://github.com/P4suta/aozora/issues) with the source that
triggered it.

# Planned diagnostics

The parser currently emits only the codes above. The richer
*authoring-error* diagnostics below are **specified but not yet emitted** —
they are the roadmap for guiding authors toward fixes. Until each lands,
the construct still parses on a best-effort basis (the relevant
[error-recovery](../arch/error-recovery.md) behaviour applies) but without
a dedicated diagnostic.

| Planned code | Severity | Triggers on |
|---|---|---|
| `mismatched_bouten_container` | Error | 傍点 opener closed by a 傍線 closer (or vice-versa) |
| `bracketed_kaeriten_no_pair` | Error | a bracketed kaeriten (`［＃二］`) with no paired `［＃一］` |
| `kaeriten_outside_kanbun` | Warning | a kaeriten char in a non-漢文 context |
| `break_in_single_line_container` | Warning | a page break terminating a single-line container early |

## Why a stable string code, not just a message?

1. **Test stability.** The corpus sweep and conformance gate count
   diagnostics by code; a test like "this corpus emits at most N
   `unresolved_gaiji` warnings" survives message-wording tweaks and
   localisation. A test that greps the message string does not.
2. **Tool integration.** Editors / LSPs / CI lints filter by code
   (e.g. "treat every `Error`-severity code as fatal, ignore
   `unrecognised_container_directive` for legacy files"). String matching
   on prose is fragile.

## See also

- [Architecture → Error recovery](../arch/error-recovery.md) — what the
  parser *does* after each diagnostic fires (preserved output, dropped
  tokens, where the bytes go).
- [CLI reference](../ref/cli.md) — `aozora check --diagnostic-format` and
  the exit-code contract.
- [Library Quickstart → Diagnostics](../getting-started/library.md)
- [Bindings → Diagnostics as JSON](../recipes/diagnostics-json.md)
