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
- a **span** — a byte range in the *sanitized* source (the sanitize-stage
  output: BOM stripped, CRLF→LF, 〔…〕 accents decomposed). For input with
  none of those, the sanitized bytes equal the original bytes.

## Rendering them

The `aozora check` CLI renders diagnostics three ways, chosen with
`--format`:

- **`human`** (the default on a terminal) — a graphical
  [`miette`](https://docs.rs/miette/latest/miette/) report: the source line, a caret
  under the offending span, the label, the help text, and a link back to
  this page.
- **`json`** (the default when stderr is piped) — the `aozora::json`
  diagnostics envelope, byte-identical to what the WASM / FFI / Python /
  Extism front doors emit. This is the machine / agent path.
- **`short`** — one grep-able line per diagnostic:
  `path:offset: severity[code]: message`.

Exit codes: `0` (diagnostics printed but tolerated), `1` (`--strict` with
at least one diagnostic), `2` (CLI usage error), `3` (an `Internal`
diagnostic fired — a library bug). See the [CLI reference](../ref/cli.md).

Looking up a code you saw in `check` output? `aozora explain <code>`
prints its severity, help, and a link back to this page — pass the full
`aozora::lex::unclosed_bracket` or just the short `unclosed_bracket`:

```sh
aozora explain aozora::lex::unclosed_bracket
aozora explain unclosed_bracket              # short form
```

Library consumers get `tree.diagnostics() -> &[Diagnostic]` and reach the
parts through `code()`, `severity()`, `source()`, and `span()`; the same
catalogue is available programmatically as `Diagnostic::explain(code) ->
Option<DiagnosticInfo>`. All bindings carry the same structured data.

# Source diagnostics

These trace back to your input. The parser emits exactly these — the
authoring-error catalogue is [complete](#planned-diagnostics) (no diagnostic
is specified-but-unimplemented).

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
the sanitize stage (`cafe'` → `café`, `fune` + backtick → `funè`, …). This is
**intended behaviour**, not an error — it is surfaced as a `Note` so an
editor can show *what* changed. One note fires per `〔…〕` span that actually
contained a digraph; a `〔…〕` with no accent digraph is silent. The span is
in sanitized (post-decomposition) coordinates. The transform is loss-free:
`to_source` reconstructs the original `〔…〕` source form. See
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
description-only rendering. (Fires for both top-level `※［＃…］` references
and gaiji nested inside a ruby / bouten reading.)

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
bracket is kept as a plain `Directive{Unknown}` (so output is preserved
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
directive degrades to an `Directive{Unknown}`. The label spans the
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

## Forward referent not stylable

`aozora::lex::forward_referent_not_stylable` · **Warning**

```text
我《われ》は［＃「我」に傍点］          (target 「我」 is a ruby base)
```

An inline-style forward reference (`［＃「X」は太字／斜体］`, `［＃「X」に傍点］`,
`は縦中横`, `は「□」囲み`, …) named a target `X` that **is** present in the
preceding text but cannot be styled in place: it is a ruby base, on an
earlier line, inside another construct, or one of several quoted targets.
The directive is retained and the text round-trips unchanged, but the
emphasis is **not** applied to the earlier run. (When the target is a plain
run in the same line the style *is* applied — see the bold/傍点 sections.)
The label spans the directive. **Fix:** move the `［＃…］` next to a plain
occurrence of the target, or accept that the styling cannot be represented.

## Mismatched bouten container

`aozora::lex::mismatched_bouten_container` · **Error**

```text
彼は［＃傍点］必ず［＃傍線終わり］来る   (傍点 opened, 傍線 closed)
```

A 傍点 / 傍線 range form (`［＃傍点］ … ［＃傍点終わり］`) was opened with one
family — 点 (dots) or 線 (line) — and closed by the other, e.g. a `［＃傍点］`
opener closed by `［＃傍線終わり］`. The two families render differently (dots
beside the text vs a line alongside it), so the run's emphasis is
ambiguous. The parser recovers by keying the run to the *opener's* variant.
A same-family variant difference (`白丸傍点` closed by `丸傍点終わり`) is
tolerated. The label points at the close marker. **Fix:** match the closer's
family to the opener — `［＃傍点終わり］` for any 点 variant, `［＃傍線終わり］`
for any 線 variant.

## Bracketed kaeriten no pair

`aozora::lex::bracketed_kaeriten_no_pair` · **Error**

```text
怪物［＃二］   (［＃二］ with no ［＃一］ anywhere in the document)
```

A bracketed kaeriten of rank ≥ 2 (`［＃二］` / `［＃下］` / `［＃乙］`) appears in a
document whose matching family base — `［＃一］` / `［＃上］` / `［＃甲］` — is
absent entirely, so the return mark has nothing to pair back to. The check
is document-wide and base-only by design: real 漢文 return-mark groups span
`、` / `。` and line boundaries (and write `二` before `一`), and 上下点 may
use just `上` … `下` (skipping `中`), so any narrower scope would wrongly
flag valid kanbun. `レ` (re-ten) is standalone and never flagged; 送り仮名
(`［＃（ス）］`) is not a ladder mark. **Fix:** add the missing base mark, or
check the mark is a genuine 返り点.

## Kaeriten outside kanbun

`aozora::lex::kaeriten_outside_kanbun` · **Warning**

```text
これは［＃レ］と書いた。   (a lone kaeriten in kana prose)
```

A kaeriten (`［＃二］` / `［＃レ］` / …) is the only one in the entire document
and its surroundings read as ordinary kana prose, so it is most likely a
stray `［＃…］` annotation rather than a genuine 返り点. The lookahead
heuristic is deliberately conservative — a document carrying a cluster of
kaeriten (real 漢文) is never flagged. The label points at the lone mark.
(Only the bracketed `［＃…］` form is recognised; a bare reading-mark glyph in
running text is left as plain text.) **Fix:** confirm the mark is intended;
remove it if it is not a reading mark.

## Break in single line container

`aozora::lex::break_in_single_line_container` · **Warning**

```text
［＃地付き］本文［＃改ページ］   (single-line directive shares its line with a break)
```

A single-line layout directive (`［＃地付き］`, `［＃N字下げ］`) or a warichu
range (`［＃割り注］ … ［＃割り注終わり］`) governs only the rest of its line. A
page / section break sharing that line — or, for warichu, falling between
the open and close — drops the container: the break starts a new block, so
the directive's run is cut short. Paired block forms (`［＃ここから…］ …
［＃ここで…終わり］`) persist across breaks and are **not** flagged (print
typography keeps the layout across pages). The label points at the break.
**Fix:** move the break off the line, or use the paired block form.

# Notation-hygiene lint

`aozora::lint::*` codes are *advisory* authoring lints, not lex faults: the
parser keeps the input verbatim (the body round-trips unchanged) and the exit
code stays `0` unless `--strict`. They flag notation spelled as a near-miss of
a recognized construct so an author can tidy it up.

## Non-canonical directive

`aozora::lint::non_canonical_directive` · **Warning**

```text
前段。［＃改行を挿入］後段。   (改行を挿入 is a non-canonical ［＃改行］)
```

A `［＃…］` body that is a verified near-miss of a recognized directive —
送り仮名 drift (`字下げ終り` → `ここで字下げ終わり`), a synonym (`表組` → `表`,
`中央寄せ` → `中央揃え`), or a malformed prefix / close (`ここか3字下げ` →
`ここから3字下げ`) — parses as an Unknown directive instead of the intended
construct. The lint suggests the canonical spelling in its message; the body
itself is left untouched (Unknown round-trips verbatim). The catalogue is a
closed, parser-verified map, so genuine editorial notes never fire. **Fix:**
rewrite to the canonical form — `aozora lint --fix` or `aozora fmt --fix` does
it in place. See [Notation hygiene](hygiene.md) for the full layer model.

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

None outstanding. Every authoring-error diagnostic in the catalogue —
including the four model-dependent ones (`mismatched_bouten_container`,
`bracketed_kaeriten_no_pair`, `kaeriten_outside_kanbun`,
`break_in_single_line_container`) — is now emitted; see the
[Source diagnostics](#source-diagnostics) above. New 記法 work adds new
codes here as it lands.

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
- [CLI reference](../ref/cli.md) — `aozora check --format` and
  the exit-code contract.
- [Library Quickstart → Diagnostics](../getting-started/library.md)
- [Bindings → Diagnostics as JSON](../recipes/diagnostics-json.md)
