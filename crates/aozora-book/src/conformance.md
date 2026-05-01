# Conformance suite

aozora ships a WPT-style conformance corpus so other implementations
of the Aozora Bunko notation (the [tree-sitter reference
grammar](arch/grammar-tree-sitter.md), third-party ports, alternate
parsers in other languages) can measure their adherence against the
same set of cases the Rust parser is held to.

## Tier model

| Level    | Meaning                                          | Effect on `xtask conformance run` |
| -------- | ------------------------------------------------ | --------------------------------- |
| `must`   | Required for any conforming implementation.      | A failure here exits non-zero.    |
| `should` | Recommended but not strictly required.           | A failure here logs a warning.    |
| `may`    | Optional; implementations decide.                | Pure information; never fails.    |

The tier is declared per case in
`crates/aozora-conformance/fixtures/render/<case>/meta.toml`
alongside a `feature` tag (`ruby`, `bouten`, `composite`, `recovery`,
…). The runner aggregates pass / fail counts by `(feature, level)`.

## Running

```sh
just conformance               # full suite, exits non-zero on must-fail
just render-gate               # the byte-identical render gate, K3-style
xtask conformance run          # invoke the runner directly
```

A successful run also writes
`crates/aozora-book/src/conformance-results.json` with per-case
detail. The JSON shape is stable; downstream dashboards / shields
parse it.

## What gets compared

The runner checks two outputs per fixture:

1. `tree.to_html()` byte-identical to `expected.html`.
2. `tree.serialize()` byte-identical to `expected.serialize.txt`.

Both goldens regenerate via
`UPDATE_GOLDEN=1 cargo test -p aozora-conformance --test render_gate`
after intentional output changes. The runner does *not* yet compare
diagnostics or wire-format output; layering those in is a 0.5.0+
extension of the same manifest.

## Implementations

For 0.4.0 the runner targets a single implementation — the Rust
parser itself. The results.json format carries an `implementation`
field so external runs can append their own results without
disturbing the canonical Rust pass-rate.

## See also

- [Architecture → Error recovery](arch/error-recovery.md) — what the
  parser does after each diagnostic fires; the `recovery`-feature
  fixtures pin those semantics.
- [Node reference](nodes/index.md) — per-`NodeKind` documentation.
