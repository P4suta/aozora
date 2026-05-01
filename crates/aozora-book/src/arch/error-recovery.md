# Error recovery

aozora is *non-fatal by design*: the parser always returns an
[`AozoraTree`][tree] even when the input violates the spec. Every
problem is reported as a structured [`Diagnostic`][diagnostic] whose
[`code`][code] tooling can match on; nothing is ever raised as a
panic from `Document::parse`.

This page documents what the parser actually *does* when each
diagnostic fires — useful when implementing editor surfaces, lint
fixers, or anything else that runs over imperfect documents.

## Recovery model

Every diagnostic carries two orthogonal axes:

| Axis | Values | Meaning |
| --- | --- | --- |
| [`severity`][severity] | `Error` / `Warning` / `Note` | Routing hint for downstream surfaces; **does not affect parsing**. |
| [`source`][source] | `Source` / `Internal` | Whether the issue is in the user's input (`Source`) or in the library's invariants (`Internal`). |

The parser keeps running regardless of severity. `Error` does not
short-circuit; it only marks the surrounding output region as
suspect so callers (CLI `--strict`, LSP) can decide policy. CI gates
typically treat any `Error` as failure, but the *AST is still safe
to walk* — the spans, classifications, and renderer all remain
consistent.

## Source-side codes

### `aozora::lex::source_contains_pua`

```text
Hello, …<U+E001>… world.
```

A user-supplied codepoint in the range U+E001..U+E004 collides with
one of the lexer's PUA sentinel reservations. The placeholder
registry keys on these codepoints, so a bare collision means the
classifier could no longer tell user-text occurrences from
lexer-inserted markers.

**Recovery:** the colliding bytes are *kept verbatim* in the
sanitised text — Phase 0 does not delete them. Downstream the
character flows through as plain text (the registry has no entry
for the position so it is treated as ordinary content). Editors
that want to surface the collision visually can match on this
code; ordinary HTML rendering is unaffected.

### `aozora::lex::unclosed_bracket`

```text
｜青梅《おうめ
```

An open delimiter (`｜`, `《`, `［`, `〔`, `「`, …) reached
end-of-input with no matching close on the pairing stack.

**Recovery:** no [`PairLink`][pair-link] is emitted for the orphaned
opener (`Unclosed` opens have no partner span and would only
confuse editor highlights). Phase 3 then sees no Aozora construct
covering the unclosed open and degrades the whole region to plain
text — the bytes from the opener to EOF are preserved literally,
just without ruby / annotation classification.

### `aozora::lex::unmatched_close`

```text
》orphaned
```

A close delimiter saw an empty pairing stack, or its `PairKind`
mismatched the stack top.

**Recovery:** the stray close is *not* matched against any opener;
no `PairLink` is emitted. The bytes flow through as plain text,
preserving the user's content; nothing on the stack pops. The
diagnostic span points at the close itself so editors can surface
it without corrupting the document tree.

## Internal codes

`Internal`-source diagnostics indicate library bugs — production
parses on well-formed input never emit these. They are kept
publicly visible so tooling can distinguish "user input has a
problem" from "the library has a problem"; the parse still
completes best-effort to keep editors usable.

| Code | What broke |
| --- | --- |
| [`residual_annotation_marker`][r1] | An `［＃` digraph survived classification — a recogniser is missing for the contained keyword. |
| [`unregistered_sentinel`][r2] | A PUA sentinel is in normalised text without a registry entry. |
| [`registry_out_of_order`][r3] | The placeholder-registry vector is not strictly position-sorted. |
| [`registry_position_mismatch`][r4] | A registry entry references a normalised position whose codepoint is not the expected sentinel kind. |

**Recovery:** the parser never *acts* on internal diagnostics —
the problematic stretch flows through as plain text, the diagnostic
records what was wrong, and `Document::parse` returns normally.
Reproductions belong in `aozora-spec` test fixtures so the bug
surface keeps shrinking over releases.

## What recovery is *not*

The parser does not attempt **fix-it suggestions**. There is no
"did you mean `［＃ここで字下げ終わり］`?" guess; the diagnostic's
`help` text describes the symptom, not the cure. Higher-level
tooling (LSPs, editor extensions) is the right place for fix-it
proposals — they have user context the parser does not.

The parser also does not try to **synthesise missing tokens**. A
truly unclosed bracket stays unclosed in the tree; we don't insert
a phantom `》` to "balance" it. Synthesising tokens hides the
diagnostic from any caller that walks the AST instead of the
diagnostic list, and turns a fixable user error into a silent
correction.

## Cross-references

- [Diagnostics catalogue](../notation/diagnostics.md) — code-by-code
  reference, including the `［＃改ページ］`-family directives this
  page does not cover.
- [Architecture → Seven-phase lexer](lexer.md) — which pipeline
  phase emits which code.
- [Wire format → DiagnosticWire](../wire/overview.md) — the JSON
  shape every binding (FFI, WASM, Python) carries diagnostics over.

[tree]: https://docs.rs/aozora/latest/aozora/struct.AozoraTree.html
[diagnostic]: https://docs.rs/aozora/latest/aozora/enum.Diagnostic.html
[code]: https://docs.rs/aozora/latest/aozora/enum.Diagnostic.html#method.code
[severity]: https://docs.rs/aozora/latest/aozora/enum.Severity.html
[source]: https://docs.rs/aozora/latest/aozora/enum.DiagnosticSource.html
[pair-link]: https://docs.rs/aozora/latest/aozora/struct.PairLink.html
[r1]: https://docs.rs/aozora/latest/aozora/enum.InternalCheckCode.html#variant.ResidualAnnotationMarker
[r2]: https://docs.rs/aozora/latest/aozora/enum.InternalCheckCode.html#variant.UnregisteredSentinel
[r3]: https://docs.rs/aozora/latest/aozora/enum.InternalCheckCode.html#variant.RegistryOutOfOrder
[r4]: https://docs.rs/aozora/latest/aozora/enum.InternalCheckCode.html#variant.RegistryPositionMismatch
