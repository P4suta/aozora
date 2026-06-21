# 0016. Consolidate the editor/CLI tooling into the aozora monorepo

- Status: accepted
- Date: 2026-06-21
- Deciders: @P4suta
- Tags: architecture, cli, workspace, consolidation

## Context

The language tooling lived in a **separate** repository, `aozora-tools`
(`github.com/P4suta/aozora-tools`): a formatter (`aozora-fmt`), a language
server (`aozora-lsp`), a renderer-agnostic diagnostic catalogue
(`aozora-diagnostics`), a Tree-sitter grammar (`tree-sitter-aozora`), and a VS
Code extension. That repo consumed `aozora` as a pinned crates.io dependency and
mirrored its version in lockstep.

Preparing `aozora-tools` for its first crates.io publish surfaced a structural
problem: `aozora-tools` had added an `aozora` umbrella binary (crate
`aozora-cli`) that **collided** with this repo's own already-published
`aozora-cli` (same crate name, same `aozora` binary name). The umbrella also
duplicated subcommands this repo's `aozora` CLI already ships (`fmt`, `render`,
`explain`, `check`), adding only `lsp`.

Stepping back, the entire two-repo split was the root cause of a *class* of
recurring problems: the name collision, manual lockstep version pinning, a
cross-repo "sync model", a dependabot carve-out to stop `aozora` bumps, and
version mirroring. None of these are essential complexity — they exist only
because the tooling lived in a second repo. `aozora-fmt` and `aozora-diagnostics`
in particular **duplicated** capabilities this repo already has: formatting is
`Document::parse ∘ Tree::to_source` (pure parser-domain), and the diagnostic
catalogue already lives in `aozora-spec` + the CLI's `explain`/`kinds`/`schema`.

The project is pre-1.0 (v0.4.x) with a single owner, so a large structural
change is cheap now and expensive to defer.

## Decision

Consolidate **all** language tooling into this monorepo and **retire**
`aozora-tools`. Anything that operates on the language / depends on the parser
lives here; only editor *clients* (a different ecosystem and release cadence)
stay separable.

- `aozora-fmt` (formatter), `aozora-lsp` (language server),
  `aozora-diagnostics` (diagnostic catalogue), and `tree-sitter-aozora`
  (incremental grammar) move into this workspace as members.
- The VS Code extension moves under `editors/vscode/` (kept on its independent
  Marketplace cadence, like rust-analyzer / biome / deno).
- The single canonical `aozora` CLI stays here and gains `fmt`/`lsp` reach; the
  standalone `aozora-fmt` and `aozora-lsp` binaries are kept (rust/go model:
  `rustfmt`/`rust-analyzer`, `gofmt`/`gopls`).
- `aozora-tools` is archived. Crates publish from here, under this repo's single
  workspace version — no more cross-repo lockstep.

Migration is **phased**, each phase left green (build/test/clippy/conformance):
1. `aozora-fmt` (this ADR's first step).
2. `aozora-lsp` + `tree-sitter-aozora` + `aozora-diagnostics` (coupled: the LSP
   depends on the grammar and the catalogue).
3. VS Code extension; then archive `aozora-tools`.
4. Publish the consolidated crates from this repo.

`aozora-fmt` is moved faithfully as a library + binary. Because the crate version
now equals the parser version (shared workspace version), its `--version` uses
`CARGO_PKG_VERSION` directly and the old `build.rs` that parsed `Cargo.lock` for
the upstream version is dropped.

## Consequences

- **Easier:** one repo, one version, one CI, one conformance gate. The name
  collision, lockstep pinning, sync model, dependabot carve-out, and version
  mirroring all disappear. The diagnostic catalogue and formatter stop being
  duplicated across repos.
- **Harder / cost:** this workspace grows and gains heavier dev dependencies
  (tokio / tower-lsp once the LSP lands). Cargo resolves dependencies per crate,
  so downstream `aozora` library users are unaffected (`cargo build -p aozora`
  never builds the LSP); the cost is contributor build/CI time, contained with
  `default-members` + feature gating + a CI matrix.
- **Follow-up:** there are now two formatter surfaces in one repo — the new
  `aozora-fmt` binary (multi-file / `--diff` / `--json`) and the existing
  `aozora fmt` subcommand (single-file, encoding-aware, `--watch`). Unifying them
  onto one implementation (the `aozora fmt` subcommand delegating to the
  `aozora-fmt` library, with the encoding/watch handling preserved) is a tracked
  follow-up, deliberately deferred so each migration phase stays small.

## Alternatives considered

- **Keep the two-repo split; just rename the colliding crate.** Rejected: it
  fixes the symptom (the name) but keeps the whole class of cross-repo problems
  (lockstep pinning, sync model, duplicated fmt/diagnostics).
- **Host the unified CLI in `aozora-tools` instead (make it canonical, retire
  this repo's `aozora-cli`).** Rejected: the parser-introspection subcommands
  (`inspect`/`kinds`/`schema`) are intrinsically parser-domain and belong with
  the spec; and the LSP can only assemble fmt+lsp+render at the top of the
  dependency graph, which is here. Hosting the CLI in the tooling repo inverts
  the natural layering.
- **Separate specialised binaries across the two repos (rust/go model) without
  merging repos.** This was the interim direction, but it still leaves two repos
  to keep in lockstep. Full consolidation removes the split entirely, which is
  the actual win.

## References

- Supersedes `aozora-tools` ADR
  `0002-consolidate-clis-into-aozora-umbrella.md`.
- ADR [0009](./0009-version-single-source-of-truth.md) (version single source),
  [0012](./0012-release-time-generated-cli-artefacts.md) (completions/man),
  [0015](./0015-spec-syntax-layer-boundary.md) (spec/syntax boundary).
- The consolidation plan file.
