/**
 * tree-sitter-aozora — incremental grammar for aozora-flavored markdown.
 *
 * Goal: enable size-independent LSP responses (hover, inlay, codeAction)
 * by exposing a tree the LSP can query in microseconds even on 100k+
 * docs. The semantic Rust parser (`aozora-parser`) stays the source
 * of truth for HTML rendering / formatting / diagnostics; this grammar
 * is the *syntactic skeleton* used by the LSP request handlers.
 *
 * Coverage (Stage 1):
 *   - gaiji          ※［＃...］
 *   - slug           ［＃...］
 *   - explicit_ruby  ｜base《reading》
 *   - implicit_ruby  kanji-run《reading》
 *   - text / newline (catch-all)
 *
 * Disambiguation:
 *   - The body of a slug is captured via `token.immediate(/[^］\n]+/)`
 *     so the lexer locks onto "everything until ］" the moment it
 *     sees `［＃`. Without this, kanji inside the body collide with
 *     the `ruby_base_implicit` token's lookahead.
 *   - The reading inside `《...》` uses the same trick: `token.immediate`
 *     after `《` consumes the reading body atomically.
 *   - Implicit-ruby base uses `prec.dynamic` so it only wins when
 *     followed by `《`; otherwise the kanji-run is just text.
 *
 * Out of scope (Stage 2+):
 *   - 〔...〕 accent decomposition
 *   - 《《...》》 double-bracket emphasis
 *   - kaeriten / 縦中横
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
    ),

    // ※［＃description, mencode］ — annotation marker for "this glyph
    // is not in the base character set; here is its description and
    // its JIS/Unicode reference".
    gaiji: $ => seq(
      $.gaiji_marker,
      $.slug,
    ),

    gaiji_marker: $ => '※',

    // ［＃...］ — bare annotation slug. When preceded by ※ it binds
    // into a `gaiji` node above; standalone slugs are typesetting
    // directives (e.g. ［＃改ページ］).
    slug: $ => seq(
      '［＃',
      field('body', $.slug_body),
      '］',
    ),

    // `token.immediate` — the body is consumed atomically as soon as
    // `［＃` is seen; no other tokenisation interleaves until the
    // closing ］ arrives. Required to keep CJK chars inside the
    // body from racing with `ruby_base_implicit`.
    slug_body: $ => token.immediate(/[^］\n]+/),

    // ｜base《reading》 — explicit-delimiter ruby. The pipe pins the
    // base run; the reading is whatever sits between 《 and 》.
    explicit_ruby: $ => seq(
      '｜',
      field('base', $.ruby_base_explicit),
      '《',
      field('reading', $.ruby_reading),
      '》',
    ),

    ruby_base_explicit: $ => token.immediate(/[^《｜\n]+/),

    // kanji-run《reading》 — implicit ruby. The base is the longest
    // preceding kanji run; aozora typesetters rely on this when the
    // base is unambiguous. Static `prec(1, ...)` is enough because
    // the LL(1) decision point is `《` immediately after the kanji
    // run — no genuine ambiguity remains for the parser to defer
    // until runtime. (Was `prec.dynamic` historically; switched to
    // static after the per-paragraph rearchitecture pushed
    // ts_subtree_compress / summarize_children to ~14% of the trace
    // and the dynamic-prec runtime decision became a measurable
    // share of that.)
    implicit_ruby: $ => prec(1, seq(
      field('base', $.ruby_base_implicit),
      '《',
      field('reading', $.ruby_reading),
      '》',
    )),

    // CJK kanji + iteration marks + small Katakana that count as
    // kanji in the aozora implicit-ruby scanner. Ranges spelled
    // with explicit \uXXXX escapes so static analyzers can verify
    // the bounds without misreading a literal CJK char as a stray
    // unicode point — the CodeQL `js/overly-large-range` check
    // false-positives on the literal-char form here:
    //   一-鿿     U+4E00..U+9FFF  CJK Unified Ideographs
    //   㐀-䶿     U+3400..U+4DBF  CJK Unified Ideographs Ext A
    //   豈-﫿    U+F900..U+FAFF  CJK Compatibility Ideographs
    //   々       U+3005          ideographic iteration mark
    //   ヵ ヶ    U+30F5, U+30F6  small katakana counted as kanji
    ruby_base_implicit: $ => /[\u4E00-\u9FFF\u3400-\u4DBF\uF900-\uFAFF\u3005\u30F5\u30F6]+/,

    ruby_reading: $ => token.immediate(/[^》\n]+/),

    // Catch-all text: any run of chars that aren't markup-significant.
    // The `|.` fallback matches a single char so the grammar never
    // gets stuck on a stray markup char (e.g. a lone 》 that didn't
    // close a ruby). Tree-sitter's error-recovery picks it up as
    // plain text.
    text: $ => /[^\n《》｜［］＃※]+|[》］＃]/,

    newline: $ => /\n/,
  },
});
