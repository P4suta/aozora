# Aozora notation — tree-sitter reference grammar

A [tree-sitter][tree-sitter] grammar for Aozora Bunko notation,
shipped as a **reference implementation** alongside the canonical
Rust parser at `../../crates/aozora-pipeline/`.

When the two disagree, **the Rust parser wins**. This grammar
exists to plug Aozora documents into the tree-sitter ecosystem
(neovim / helix / web-tree-sitter highlighting), to serve as a
teaching artefact, and to provide a second implementation that
can run against the WPT-style conformance suite (Phase O4).

## Build

```sh
# One-time install of the tree-sitter CLI
npm install -g tree-sitter-cli

# Generate parser.c from grammar.js
cd grammars/aozora.tree-sitter
tree-sitter generate

# Smoke test (uses tree-sitter's own corpus harness; no test
# fixtures yet — see "Conformance" below)
tree-sitter test
```

## Coverage

This grammar handles the *bracket structure* of Aozora notation
faithfully and leaves stateful semantic resolution to the
consumer. Supported constructs:

- `｜base《reading》` — explicit-delimiter ruby
- `base《reading》` — implicit-delimiter ruby
- `《《content》》` — double-bracket bouten
- `※［＃...］` — gaiji marker
- `［＃...］` — generic bracket annotation (page break, indent,
  bouten directive, kaeriten, sashie, heading, …)
- `〔...〕` — tortoise-bracket / accent-decomposition span

What this grammar deliberately does **not** model
(context-free-without-scanner.c is too weak):

- **Stateful container pairing** — `［＃ここから2字下げ］` matches
  `［＃ここで字下げ終わり］` even when other annotations intervene.
  A hand-written `scanner.c` could close this gap, but that
  contradicts the "declarative reference" framing.
- **Forward `「target」に傍点` resolution** — the bouten directive
  walks back through recent text to bind to a quoted target. The
  bracket grammar accepts the directive faithfully; the lookup is
  the consumer's job.
- **Ruby base disambiguation** — when the glyph run preceding
  `《...》` could extend further, the canonical parser uses a
  more nuanced classification rule. This grammar accepts the
  greedy match.

The conformance percentage against the Rust parser's `must`-tier
fixtures is documented in the handbook at
[arch/grammar-tree-sitter](../../crates/aozora-book/src/arch/grammar-tree-sitter.md).

## Conformance

The fixture set in `crates/aozora-conformance/fixtures/render/`
defines per-case `feature` and `level` metadata. A future
extension of `xtask conformance run` will accept
`--implementation tree-sitter` and run those fixtures through this
grammar to compute a per-tier pass rate. See
`crates/aozora-book/src/conformance.md` for the canonical
runner.

[tree-sitter]: https://tree-sitter.github.io/
