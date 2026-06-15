# Architecture Decision Records

Significant, hard-to-reverse decisions live here in
[MADR](https://adr.github.io/madr/) format. Read the one that governs an
area before changing what it governs. Scaffold a new one with:

```
just new-adr "Short imperative title"
```

## Index

| ADR | Title | Status |
|---|---|---|
| [0001](0001-zero-parser-hooks.md) | Zero parser hooks — Aozora-first lexer | accepted |
| [0003](0003-accent-decomposition-preparse.md) | Accent decomposition preparse | accepted |
| [0004](0004-lint-profile-policy.md) | Lint profile policy | accepted |
| [0005](0005-corpus-sweep-strategy.md) | Corpus sweep strategy | accepted |
| [0010](0010-bouten-and-bousen-range-containers-as-a-first-class-notation-feature.md) | Bouten / bousen range containers as a first-class notation feature | accepted |

## Numbering

`aozora` was split out of [`P4suta/afm`](https://github.com/P4suta/afm)
(afm ADR-0010, "extract aozora core"). The parser-layer decisions that
originated on the afm side moved here and were **renumbered** into this
repo's own sequence; afm keeps redirect stubs (`NNNN-MOVED.md`) pointing
at the canonical text here. The numbering therefore starts at 0001 and
has gaps relative to afm's — that is expected, not a mistake. New aozora
ADRs continue this repo's sequence.
