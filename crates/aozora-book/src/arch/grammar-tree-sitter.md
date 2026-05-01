# tree-sitter reference grammar

aozora ships a [tree-sitter][tree-sitter] grammar at
`grammars/aozora.tree-sitter/grammar.js` as a **reference
implementation** alongside the canonical Rust parser. When the two
disagree the Rust parser wins; this grammar exists to plug Aozora
documents into the tree-sitter ecosystem (neovim, helix,
web-tree-sitter / CodeMirror) and to serve as a teaching artefact.

## Why a separate grammar at all

The Rust parser is a seven-phase pipeline with a hand-rolled
classifier; reading it tells you *how* the canonical
implementation works but not *what* the spec accepts. A
declarative grammar is the language community's preferred form
for "what the spec accepts." Shipping one alongside the parser
lets external tooling consume Aozora without binding to the Rust
ABI.

## What it does cover

The grammar handles bracket structure faithfully:

- `｜base《reading》` and `base《reading》` — explicit / implicit
  ruby
- `《《content》》` — double-bracket bouten
- `※［＃...］` — gaiji marker
- `［＃...］` — generic bracket annotation
- `〔...〕` — tortoise-bracket / accent-decomposition span

Plain text — any byte that is not one of the bracket openers —
flows through as a `plain_text` token, keeping the grammar lossless
against the byte stream.

## What it deliberately does not cover

Three classes of behaviour are intentionally out of reach:

1. **Stateful container pairing.** `［＃ここから2字下げ］` matches
   `［＃ここで字下げ終わり］` across intervening content; a context-
   free grammar without a hand-written `scanner.c` cannot close
   this. Consumers rely on the body content of the bracket
   annotation to recognise the pairing themselves, or fall back to
   the Rust parser.
2. **Forward `「target」に傍点` resolution.** The bouten directive
   walks back through preceding text to bind to a quoted run.
   The grammar accepts the directive faithfully; the lookup
   stays the consumer's job.
3. **Ruby base disambiguation.** When the glyph run preceding
   `《...》` could extend further, the Rust classifier uses a more
   nuanced rule. The grammar accepts the greedy base match
   uniformly.

A `scanner.c` extension could plug some of these gaps, but doing
so contradicts the declarative-reference framing of the artefact
and would put the canonical-parser-replacement question on the
table prematurely.

## Status

The grammar covers approximately 40 % of the canonical parser's
constructs as measured by an unweighted variant count. The gap to
full coverage is documented; closing it would require a `scanner.c`
extension, which trades the declarative-reference framing for a
higher ceiling.

## Cross-references

- [Architecture → Concrete syntax tree](cst.md) — the rowan-backed
  in-process equivalent.
- [Conformance suite](../conformance.md) — a future
  `xtask conformance run --implementation tree-sitter` will run
  the fixture set against this grammar to compute the per-tier
  pass rate against `must` / `should` / `may`.
- [`grammars/aozora.tree-sitter/README.md`](https://github.com/P4suta/aozora/blob/main/grammars/aozora.tree-sitter/README.md)
  — build instructions.

[tree-sitter]: https://tree-sitter.github.io/
