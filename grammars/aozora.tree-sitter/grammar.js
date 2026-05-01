// Aozora Bunko notation — tree-sitter reference grammar.
//
// This file is a *reference implementation* of the Aozora notation,
// not the canonical parser. The canonical parser lives in
// `crates/aozora-pipeline/` (Rust). When the two disagree the Rust
// parser wins; this grammar exists to:
//
//  1. Plug Aozora-shaped buffers into the tree-sitter ecosystem
//     (neovim / helix / tree-sitter-cli highlighting, CodeMirror
//     via web-tree-sitter, etc.).
//  2. Serve as a teaching artefact — declarative grammar reads
//     more accessibly than the seven-phase Rust pipeline.
//  3. Provide a second implementation for the WPT-style
//     conformance suite (Phase O4) to measure adherence against.
//
// What the grammar *does not* cover (out of reach for a
// context-free formalism without GLR or scanner.c hooks):
//
//  - Stateful container pairing (`［＃ここから2字下げ］` matches
//    `［＃ここで字下げ終わり］` even when other annotations
//    intervene). Tree-sitter does support stateful scanners via
//    a hand-written `scanner.c`, but that defeats the
//    "declarative reference" goal of this artefact.
//  - Forward / backward `［＃「target」に傍点］` resolution that
//    walks back through the recent text to find the quoted run.
//  - Ruby base disambiguation when the preceding glyph could
//    extend the base run further.
//
// These features stay in the Rust parser; the tree-sitter grammar
// classifies the *bracket structure* faithfully and leaves
// semantic resolution to the consumer.

module.exports = grammar({
  name: 'aozora',

  // The grammar's "extra" tokens — whitespace and newlines flow
  // through every rule by default.
  extras: $ => [],

  // The classifier is deliberately permissive: any byte the
  // bracket rules don't claim falls through as plain text. This
  // keeps the grammar context-free and lossless against the
  // byte stream.
  conflicts: $ => [],

  rules: {
    document: $ => repeat($._element),

    _element: $ => choice(
      $.ruby_explicit,
      $.ruby_implicit,
      $.double_ruby,
      $.gaiji_marker,
      $.bracket_annotation,
      $.tortoise_segment,
      $.plain_text,
    ),

    // `｜base《reading》` — explicit-delimiter ruby. The leading
    // `｜` (U+FF5C) anchors the base run.
    ruby_explicit: $ => seq(
      '｜',
      field('base', $.ruby_base),
      '《',
      field('reading', $.ruby_reading),
      '》',
    ),

    // `base《reading》` — implicit-delimiter ruby. Any glyph run
    // immediately followed by a `《...》` pair classifies as
    // ruby; the canonical Rust parser disambiguates trickier
    // cases that this grammar accepts uniformly.
    ruby_implicit: $ => prec(-1, seq(
      field('base', $.ruby_base),
      '《',
      field('reading', $.ruby_reading),
      '》',
    )),

    // `《《content》》` — double-bracket bouten. Matched explicitly
    // so the inner `《` / `》` pair does not fall back to ruby.
    double_ruby: $ => seq(
      '《《',
      field('content', $.double_ruby_content),
      '》》',
    ),

    // `※［＃description、mencode］` — gaiji marker. The body has
    // its own grammar inside the brackets in the Rust parser; we
    // fall back to a single inline run here.
    gaiji_marker: $ => seq(
      '※',
      $.bracket_annotation,
    ),

    // `［＃...］` — generic annotation (page break, indent
    // marker, bouten directive, kaeriten, …). The grammar
    // accepts any non-`］` body; downstream consumers parse the
    // body content for kind discrimination.
    bracket_annotation: $ => seq(
      '［＃',
      field('body', $.annotation_body),
      '］',
    ),

    // `〔...〕` — accent-decomposition span. Phase 0 of the Rust
    // parser rewrites these via accent decomposition; we
    // preserve the bracket structure here.
    tortoise_segment: $ => seq(
      '〔',
      field('body', $.tortoise_body),
      '〕',
    ),

    ruby_base: $ => token(/[\p{Letter}\p{Number}\p{Mark}_]+/),
    ruby_reading: $ => token(/[^》]+/),
    double_ruby_content: $ => token(/(?:[^》]|》(?!》))+/),
    annotation_body: $ => token(/[^］]+/),
    tortoise_body: $ => token(/[^〕]+/),

    // Plain text run — any byte that is NOT one of the bracket
    // openers / structural markers. This is the fallback that
    // makes the grammar lossless.
    plain_text: $ => token(/[^｜《》［］〔〕※]+/),
  },
});
