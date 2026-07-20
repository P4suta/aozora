/**
 * tree-sitter-aozora — incremental grammar for Aozora Bunko source.
 *
 * The Rust parser is the semantic authority. This grammar accepts the same
 * source language without ERROR nodes and provides a lossless editing
 * projection.
 *
 * Complete constructs are atomic tokens. An unmatched opener therefore
 * cannot commit the parser to a partial production; it falls through to
 * `literal_markup` without invoking error recovery.
 *
 * Constructs without a dedicated projection are emitted as
 * `literal_markup`. That node is deliberately lower precedence than every
 * recognized construct, so recovery never needs an ERROR node and new
 * notation remains lossless until it receives a richer projection.
 */

module.exports = grammar({
  name: 'aozora',

  extras: $ => [],

  conflicts: $ => [],

  rules: {
    document: $ => repeat($._element),

    _element: $ => choice(
      $.gaiji,
      $.slug,
      $.explicit_ruby,
      $.implicit_ruby,
      $.text,
      $.newline,
      $.literal_markup,
    ),

    gaiji: $ => token(prec(4, /※［＃[^］\n]+］/)),

    slug: $ => token(prec(3, /［＃[^］\n]+］/)),

    explicit_ruby: $ => token(prec(2, /｜[^《｜\n]+《[^》\n]+》/)),

    implicit_ruby: $ => token(prec(2, /[\u4E00-\u9FFF\u3400-\u4DBF\uF900-\uFAFF\u3005\u30F5\u30F6]+《[^》\n]+》/)),

    // Catch-all text: any run of chars that aren't markup-significant.
    // The `|.` fallback matches a single char so the grammar never
    // gets stuck on a stray markup char (e.g. a lone 》 that didn't
    // close a ruby). Tree-sitter's error-recovery picks it up as
    // plain text.
    text: $ => /[^\n《》｜［］＃※]+|[》］＃]/,

    newline: $ => /\n/,

    literal_markup: $ => token(prec(-1, /[^\n]/)),
  },
});
