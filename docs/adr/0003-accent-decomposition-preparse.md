# 0003. Accent decomposition preparse

- Status: accepted
- Date: 2026-05-31
- Deciders: @P4suta
- Tags: parser, encoding

> Parser-layer decision; originated as afm ADR-0004 and moved here on the
> parser extraction (afm ADR-0010).

## Context

青空文庫 represents Latin accented letters with the `〔…〕` accent
notation (e.g. `〔e'〕` for é, `〔a`〕` for à) because the source corpus is
predominantly Shift_JIS, which cannot encode them directly. These appear
anywhere body text appears, are orthographic — not structural — and must
round-trip byte-for-byte on serialize.

Handling them inside the main classifier would smear a character-encoding
concern across the structural grammar (ruby / bouten / containers) and
force every later phase to special-case `〔…〕`.

## Decision

Decompose `〔…〕` accent sequences in **Phase 0 (sanitize)**, before the
structural lexer runs, driven by a static accent table (combining base
letter + diacritic mark). Sanitize is a pure function over the byte
stream; downstream phases then see already-normalised text and never
need to know `〔…〕` existed. The original spans are tracked so serialize
can reconstruct the source form exactly.

## Consequences

- The structural grammar stays free of encoding concerns.
- Round-trip (serialize) is exact because sanitize records the mapping.
- The accent table is data, not code — extending coverage is a table edit.
- Sanitize runs twice in the CST path (`aozora::cst::from_tree` re-derives
  it); acceptable because it is pure and cheap.

## Alternatives considered

- **Decompose during classify.** Rejected: leaks an encoding concern into
  every structural phase.
- **Decompose in the renderer only.** Rejected: diagnostics + CST + query
  surfaces would all see the raw `〔…〕`, each re-implementing the mapping.

## References

- `crates/aozora-syntax/src/accent.rs` (the table).
- `crates/aozora-pipeline/src/lexer/sanitize.rs`.
