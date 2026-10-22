/**
 * Defines a lossless tree-sitter grammar for Aozora source.
 * The Rust parser is the semantic authority.
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

    text: $ => /[^\n《》｜［］＃※]+|[》］＃]/,

    newline: $ => /\n/,

    literal_markup: $ => token(prec(-1, /[^\n]/)),
  },
});
