# 0035. Delegate the trigger scan to aho-corasick

- Status: accepted
- Date: 2026-07-15
- Deciders: @P4suta
- Tags: perf, scanner, safety

## Context

This records a decision taken in #61 and never written down. It is being
recorded now because the repository has already oscillated once, and because
the only surviving prose account — the handbook's `arch/scanner.md` — argued
for the *rejected* side and had to be deleted.

The trigger scan finds the byte offsets of the 13 Aozora delimiters. It was, at
~35% of real-prose parse time, the dominant parse cost, and it carried the
tree's only `unsafe`: a hand-rolled Teddy multi-pattern matcher with one SIMD
inner kernel per ISA (`pshufb` / `vqtbl1q_u8` / `i8x16_swizzle`) plus a scalar
fallback (#31).

A real-prose flamegraph showed the problem was **filter selectivity, not SIMD
quality**. The candidate filter keyed on the lead byte's nibbles — and `0xE3`
is the lead byte of *every hiragana and katakana codepoint*. On Japanese prose
the filter fired across a large fraction of the text and paid a scalar trigram
verify to reject each kana byte. Micro-tuning the kernel would only have helped
the machine it was tuned on, which is meaningless for a parser other people run.

## Decision

Delegate the scan to **`aho-corasick`** (already a workspace dependency). Its
packed matcher fingerprints more than the lead byte, so it is *both*
algorithmically more selective and free of `unsafe`.

`aozora-scan` is `#![forbid(unsafe_code)]`. At the time, that left **no `unsafe`
in the tree at all**. The workspace lint is `unsafe_code = "forbid"`; the only
crates that relax it since are the ones whose whole job is a C boundary
(`aozora-ffi`'s ABI, `tree-sitter-aozora`'s binding to the generated C parser).
No parser crate has any.

`NaiveScanner` stays as a differential oracle: `property_backend_equiv`
(4096 cases at `prop-deep`) proves the two agree byte-for-byte over
`aozora_fragment`, `pathological_aozora`, and `unicode_adversarial`. Both derive
their trigger set from the same `aozora_spec` constant, so they cannot drift.

## Consequences

- **~1240 lines of per-ISA kernels and runtime dispatch deleted**, along with
  the obligation to keep them correct on hardware we do not own.
- **Portable by construction.** aho-corasick carries its own per-platform
  backends; `no_std` falls back to `NaiveScanner`.
- Faster, though that is not why: **~617 MB/s vs ~497 MB/s** on 8 MiB of real
  prose, byte-identical output (+24%); ~3–6% end-to-end across a 298-document
  corpus.
- **A benchmark can invert this conclusion.** On annotation-*dense* synthetic
  input — far denser than any real work — aho-corasick's per-match overhead can
  fall behind the naive walk. Anyone re-measuring must use real prose. This is
  the trap most likely to get the decision overturned by someone acting in good
  faith.

## Alternatives considered

**Keep the hand-rolled Teddy and improve the filter.** The fix would have been
to fingerprint more than the lead byte — which is what aho-corasick already
does, correctly, on more platforms than we would have written kernels for. We
would have been reimplementing a maintained library, in `unsafe`, to reach a
result it already gives.

**Keep it for the AVX2 win.** There was no AVX2 win to keep: the safe library
was faster on the same input. Even had it been slower, an OSS parser is not
tuned for one maintainer's CPU.

**`regex-automata`.** Rejected in the earlier round (#31) and not revisited: it
brings ~600 KB of state tables for a 13-pattern literal scan.

## References

- #61 (`92b298c`) — the flamegraph, the measurements, and the deletion.
- `crates/aozora-scan/src/lib.rs` — the surviving account, in the code.
- ADR-0004 (lint profile) — `unsafe_code = "forbid"` at the workspace level is
  what this decision made reachable.
- The handbook's `arch/scanner.md` argued the opposite — "Why a self-rolled
  Teddy" — for the whole of this decision's life, describing types
  (`BackendChoice`, `teddy_outer`) that #61 deleted. It survived a doc-rot sweep
  in that state. Recording the decision here, where decisions live, is the point:
  a narrative page nobody consults for decisions does not get corrected when one
  is taken.
