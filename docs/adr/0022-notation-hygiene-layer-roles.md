# 0022. Notation-hygiene layer roles: parser / linter / formatter

- Status: accepted
- Date: 2026-07-02
- Deciders: @P4suta
- Tags: architecture, notation, linter, formatter

## Context

The 2026-07 notation-hygiene campaign added three tools that all touch the
same phenomenon — a `［＃…］` directive body spelled as a *verified
near-miss* of a recognised construct (送り仮名 drift, a synonym, a
malformed prefix / close), e.g. `字下げ終わり` for `ここで字下げ終わり`,
`黒丸傍点` for `丸傍点`, `中央寄せ` for `中央揃え`:

- the pipeline lint `aozora::lint::non_canonical_directive` (#371),
- the `aozora fmt --fix-notation` autofix (#373),
- the LSP "replace with canonical" quick-fix (#374).

Three consumers now reason about "what did this near-miss mean?". Without a
written contract it is tempting — and would be a mistake — to let the parser
itself start recognising the variants, or to let each tool keep its own
variant→canonical table. Both blur *which layer is allowed to reinterpret or
rewrite source*, and the parser's lossless guarantee (every `Unknown`
directive round-trips its raw bytes verbatim, which the splice / incremental
engine depends on) is exactly the thing that must not erode. Record the
role split before it drifts.

## Decision

Three layers, three roles, one catalogue.

- **Parser (`aozora-pipeline` classifier) — lossless and non-judgemental.**
  A `［＃…］` body it does not recognise stays `DirectiveKind::Unknown` and
  round-trips its raw bytes verbatim. The parser never guesses that a
  near-miss "meant" a recognised directive. Guessing would silently launder
  an input-typist's error into notation, erase the distinction between "the
  author wrote it non-canonically" and "this is canonical", and break the
  verbatim round-trip.

- **Linter (`aozora::lint::*`) — advisory only.** It flags a closed,
  parser-verified catalogue of near-misses with a did-you-mean, at `Warning`
  severity, exit 0 unless `--strict`. Zero false positives by construction:
  a fixed map, not a fuzzy matcher, pinned by the `lint_catalogue`
  parse-round-trip self-test (every variant still parses to `Unknown`; every
  suggested canonical parses to a recognised node). The linter observes; it
  never mutates source.

- **Formatter (`aozora fmt --fix-notation`) — the only layer that rewrites
  source, and only opt-in.** Default `fmt` preserves the parser's verbatim
  contract for `Unknown` directives; `--fix-notation` rewrites the flagged
  near-misses to canonical form and re-normalises, staying idempotent (the
  `write_back` guard depends on it).

**Single canonical authority.** All three resolve the canonical spelling
through the one `aozora_syntax::lint::canonical_directive` catalogue — the
same single-authority discipline as `accent.rs` for glyph composition. No
consumer keeps its own copy, so they cannot disagree, and a new near-miss
family is added once.

## Consequences

- A new near-miss family is added in exactly one place (the catalogue); the
  lint, the CLI/LSP explain text, the fmt autofix, and the LSP quick-fix all
  pick it up.
- The parser stays free of "helpfulness" heuristics: correctness and the
  round-trip contract are never traded for convenience.
- A tool that wants to reinterpret a directive goes through the linter
  (advisory) or the formatter (opt-in rewrite) — never the parser.
- The catalogue's zero-false-positive discipline (fixed map + parse-round-trip
  self-test) is the guardrail. Broadening it to fuzzy / edit-distance
  matching would relax that guarantee and needs its own ADR.

## Alternatives considered

- **Parser absorbs the variants** (recognise `字下げ終わり` as
  `ここで字下げ終わり`, and so on). Rejected: it launders typos into notation,
  discards the author-wrote-it-non-canonically signal, and breaks the
  verbatim `Unknown` round-trip the splice / incremental engine relies on.
  Correctness must not be traded for a convenience the linter already
  provides losslessly.

- **Each tool keeps its own variant→canonical map.** Rejected: three copies
  drift; a fix in one would not reach the others. The single-authority
  catalogue is the entire point.

- **Make `--fix-notation` the default.** Rejected: `fmt`'s contract is that
  it never changes meaning-bearing bytes it cannot prove equivalent.
  Rewriting a directive body is a semantic edit the user should opt into, so
  it stays behind a flag while the verbatim round-trip remains the default.

## References

- Plan: the 2026-07 notation-hygiene + parser-coverage campaign (#372).
- #371 (linter), #373 (`fmt --fix-notation`), #374 (LSP quick-fix).
- [ADR-0015](./0015-spec-syntax-layer-boundary.md) — the spec / syntax layer
  boundary this role split sits above.
