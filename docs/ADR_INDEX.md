# Architecture Decision Records

`adr/` holds [MADR 4.0](https://adr.github.io/madr/) records of the
significant, hard-to-reverse decisions. Read the one that governs an area
before changing what it governs.

**Once accepted, an ADR is never edited.** It is a dated record of what was
decided and why, not a description of the code as it stands now. A decision
that no longer holds is *superseded* by a later ADR that links back to it.

Scaffold a new one with `just new-adr "Short imperative title"`; it lands
`accepted` unless the discussion is still open.

## Numbering

The sequence has gaps — `0002` and `0032` are absent. `aozora` was split out
of [`P4suta/afm`](https://github.com/P4suta/aozora-flavored-markdown), and
the parser-layer decisions that originated there were renumbered into this
repo's sequence (afm keeps `NNNN-MOVED.md` stubs pointing here). Gaps are
expected and never backfilled: `just new-adr` takes the highest number plus
one, and renumbering an accepted ADR would break the links into it.
