# 0005. Corpus sweep strategy

- Status: accepted
- Date: 2026-05-31
- Deciders: @P4suta
- Tags: testing

> Originated as afm ADR-0007 and moved here with the lexer / parser
> (afm ADR-0010): these are aozora-layer invariants.

## Context

Synthetic property tests cover the shapes we *thought* of. A real
青空文庫 corpus (tens of thousands of works) contains notation
combinations, malformed annotations, and encoding edge cases no generator
reproduces. We want that adversarial signal without committing a
multi-gigabyte corpus to the repo or making it a hard CI dependency.

## Decision

A `corpus-sweep` pass (`just corpus-sweep`, backed by
`crates/aozora-corpus` + `tests/corpus_sweep.rs`) walks every document
under `$AOZORA_CORPUS_ROOT` and asserts a fixed set of invariants on the
public `aozora::Document` surface:

- **I1** no bare `［＃` leaks into rendered HTML;
- **I2** no PUA sentinel leaks into output;
- **I3** serialize is a round-trip fixed point (`serialize(parse(x))`
  re-parses to the same tree);
- parse / render / serialize are **total** (never panic) on every input;
- diagnostics are well-formed and span-aligned.

It is **opt-in**: with `$AOZORA_CORPUS_ROOT` unset the recipe prints an
informational line and exits 0, so `just ci` stays green on machines
without a corpus checkout. On a machine that has one, it runs as an extra
adversarial gate.

## Consequences

- Real-world adversarial coverage without bloating the repo or gating CI
  on an external corpus.
- The same invariants are the contract the fuzz harnesses and synthetic
  proptests target, so a corpus failure is reproducible as a unit case.
- Cost: the corpus is out-of-band; CI on hosted runners does not exercise
  it (developers + scheduled runs with a corpus do).

## Alternatives considered

- **Vendor a corpus subset.** Rejected: licensing + repo size; a fixed
  subset also ossifies (stops surfacing new edge cases).
- **Make it a required CI job.** Rejected: couples CI to an external,
  large, slow-to-fetch dependency.

## References

- `crates/aozora-corpus`, `crates/aozora/tests/corpus_sweep.rs`,
  `just corpus-sweep`.
