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
xtask conformance run          # the Rust parser, invoked directly
xtask conformance run --implementation tree-sitter           # the reference grammar
xtask conformance run --implementation tree-sitter --update  # refresh its snapshots
```

A successful run also writes
`crates/aozora-conformance/conformance-results.json` with per-case
detail. The JSON shape is stable; downstream dashboards / shields
parse it.

## What gets compared

The runner pins six axes per fixture:

1. `tree.to_html()` byte-identical to `expected.html`.
2. `tree.to_source()` byte-identical to `expected.serialize.txt`.
3. `aozora::json::diagnostics(tree.diagnostics())`
   byte-identical to `expected.diagnostics.json`.
4. `aozora::json::nodes(&tree)` byte-identical to
   `expected.nodes.json`.
5. `aozora::json::pairs(&tree)` byte-identical to
   `expected.pairs.json`.
6. `aozora::json::container_pairs(&tree)` byte-identical to
   `expected.container_pairs.json`.

Axes 1–2 anchor the human-readable surface; axes 3–6 pin the JSON
projections that drivers (FFI / WASM / PyO3) consume in production,
so a regression that survives the renderer gate but breaks a JSON
client lights up here.

All six goldens regenerate via
`UPDATE_GOLDEN=1 cargo test -p aozora-conformance --test render_gate`
after intentional output changes.

## Implementations

The `--implementation` flag selects what the runner measures. The
results.json format carries an `implementation` field so each run
publishes its own pass-rate without disturbing the others.

### `rust` (default)

The canonical Rust parser, held to all six axes above.

### `tree-sitter`

The [reference grammar](arch/grammar-tree-sitter.md)
(`crates/tree-sitter-aozora`). The grammar is a syntactic skeleton —
it classifies bracket structure but cannot render HTML — so the
byte-equality axes do not apply. It is measured along two orthogonal
signals instead:

- **Per-tier pass rate.** A fixture passes when the grammar parses it
  without ERROR / MISSING nodes. This is a coverage measurement,
  printed per `must` / `should` / `may`. Constructs the grammar
  deliberately does not model — stateful container pairing, forward
  bouten resolution, unclosed brackets — honestly count as
  non-passing.
- **Snapshot drift gate.** Each fixture's `root.to_sexp()` is pinned to
  `expected.tree-sitter.txt`. The S-expression carries node kinds and
  fields with no byte offsets, so it is deterministic and changes only
  when the grammar's structure changes. **Any** mismatch exits
  non-zero, tier-independent: unlike the Rust path's must/should/may
  leniency (which models *partial conformance*), a snapshot is a
  fingerprint where every change is a regression-or-intentional-update
  worth review. Refresh the snapshots with `--update` after an
  intentional grammar change, and commit the diff.

The drift gate runs inside `just conformance`, so it is enforced in
both pre-push and CI.

## See also

- [Architecture → Error recovery](arch/error-recovery.md) — what the
  parser does after each diagnostic fires; the `recovery`-feature
  fixtures pin those semantics.
- [Node reference](nodes/index.md) — per-`NodeKind` documentation.
