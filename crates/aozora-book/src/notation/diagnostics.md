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
| `empty_ruby_reading` | Error | `｜青梅《》` — base given, reading empty |
| `nested_ruby` | Error | ruby inside ruby (`｜…《｜…《…》…》`) |
| `mismatched_bouten_container` | Error | 傍点 opener closed by a 傍線 closer (or vice-versa) |
| `mismatched_container_close` | Error | `［＃ここから2字下げ］…［＃ここで地付き終わり］` — kinds differ |
| `bracketed_kaeriten_no_pair` | Error | a bracketed kaeriten (`［＃二］`) with no paired `［＃一］` |
| `tcy_target_not_found` | Warning | 縦中横 quoted run absent from the look-back window |
| `bouten_target_ambiguous` | Warning | two candidate runs in the look-back window |
| `unresolved_gaiji` | Warning | a 外字 reference resolving to neither Unicode nor JIS X 0213 |
| `kaeriten_outside_kanbun` | Warning | a kaeriten char in a non-漢文 context |
| `break_in_single_line_container` | Warning | a page break terminating a single-line container early |
| `unrecognised_container_directive` | Warning | `［＃ここから…］` matching no known container kind |
| `accent_decomposition_applied` | Note | a 〔…〕 accent digraph decomposed in Phase 0 |

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
