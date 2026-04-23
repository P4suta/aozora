# 0004. Accent decomposition via pre-parse rewrite inside `〔...〕`

- Status: accepted
- Date: 2026-04-23
- Tags: architecture, encoding, aozora-compat

## Context

Aozora Bunko's
[accent decomposition convention](https://www.aozora.gr.jp/accent_separation.html)
encodes Latin-script accented characters using ASCII digraphs:

```
e`  = è    (grave)
e'  = é    (acute)
e^  = ê    (circumflex)
e:  = ë    (diaeresis)
a~  = ã    (tilde)
a&  = å    (ring above — *except* `s&` = ß, `ae&` = æ, `AE&` = Æ,
                          `oe&` = œ, `OE&` = Œ — ligature form)
c,  = ç    (cedilla)
d/  = đ    (stroke)
a_  = ā    (macron)
```

The spec lists **118 total entries** across 23 base letters plus 5 ligatures.
Full table extracted into the implementation; source page cited per-group.

The scheme was designed for JIS X 0208 times when direct Unicode was not
available. It has two consequences for afm:

1. **CommonMark collides with the backtick marker.** A lone `` ` `` in real
   Aozora text (e.g. `fune\`bre` inside `〔oraison fune\`bre〕`) is an accent
   marker, not the opener of a code span. Left unchanged, comrak opens a code
   span on the first such backtick and scans forward for a closing backtick,
   swallowing everything between — in 『罪と罰』 this stretches over ~25 KB of
   prose and hides 11 `［＃…］` annotations from the afm extension. Tier A on
   the golden fixture therefore fails until accent markers are resolved before
   comrak runs its inline parser.
2. **The convention is non-trivial to disambiguate globally.** Naively
   applying the 118-entry table to arbitrary text false-matches English
   (`isn't` → `is`ń`t` by `n'` = ń, `word's` handled only because `d'` isn't
   in the table, etc.). Global rewriting is not safe.

However, every real-world accent-decomposition occurrence in 『罪と罰』 (and,
by community convention, in the broader Aozora Bunko corpus) is wrapped in
a `〔...〕` *tortoiseshell-bracket* pair. The spec does not mandate this
wrapping, but it is the signal editors use to mark "this is a European
fragment, interpret accent decomposition inside."

## Decision

Implement accent decomposition as a **pre-parse rewrite** limited to the
interior of `〔...〕` spans.

### Scope of rewrite

```text
〔 ... 〕   : accent decomposition applies inside
〔〕 brackets themselves remain in output as Japanese punctuation
outside 〔〕 : no rewrite — prevents English-text false positives
```

### Algorithm

1. Scan input for `〔` (U+3014).
2. For each `〔`, find the matching `〕` (U+3015) on the **same line** (the
   spec and corpus never show a line-spanning fragment).
3. Inside the span, apply a longest-match token rewrite:
   - try 3-char ligatures (`ae&`, `AE&`, `oe&`, `OE&`) first
   - then 2-char `base+marker` entries
   - otherwise emit the character unchanged
4. If `〕` is absent, leave the span untouched (malformed; surface as a
   future diagnostic).

Pre-parse builds a single new `String` only if at least one rewrite fires,
returning `Cow::Borrowed(&input)` otherwise. Typical Aozora works contain
0–5 spans per hundred pages, so the allocation cost is negligible.

### API placement

```rust
// afm-syntax/src/accent.rs
pub const ACCENT_TABLE: &[(&str, char)] = ...;   // spec-complete
pub fn decompose_fragment(span_body: &str) -> Cow<'_, str>;

// afm-parser/src/preparse.rs
pub fn apply_preparse(input: &str) -> Cow<'_, str>;  // rewrites 〔...〕 bodies
```

`afm-syntax::accent` owns the lookup table; `afm-parser::preparse` owns the
span-finding + rewriting. Clear boundary: the table is stable Aozora spec
data, the application strategy may evolve.

### Integration

```rust
pub fn parse<'a>(arena: &'a Arena<'a>, input: &str, options: &Options<'_>)
    -> &'a AstNode<'a>
{
    let rewritten = preparse::apply_preparse(input);
    comrak::parse_document(arena, &rewritten, &options.comrak)
}
```

## Consequences

### Easier

- Backtick-inside-`〔...〕` cases no longer reach comrak, so code-span
  interaction with accent markers is eliminated by construction.
- `［＃…］` annotations hiding behind accent spans are reachable — Tier A
  on 『罪と罰』 becomes achievable with no further comrak-side work.
- The 118-entry table is spec-verbatim and easy to audit against
  <https://www.aozora.gr.jp/accent_separation.html>.
- Round-tripping back to the ASCII decomposed form is possible by inverting
  the table, useful for afm → Aozora-flavoured export later.

### Harder

- **Source-position drift.** The rewritten input is shorter than the
  original wherever `e\`` → `è` collapses 2 bytes → 1 char. Diagnostic spans
  from comrak now point into the rewritten buffer. `preparse` therefore
  returns not just the string but a byte-offset map for diagnostic back-
  mapping (see `preparse::OffsetMap`).
- **Non-`〔〕` accent decompositions are not handled.** Spec-compliant but
  unwrapped occurrences would survive as literal `e\``. This matches the
  observed corpus behaviour; extension to globally-scoped rewriting is
  deferred to a later ADR with rigorous disambiguation (e.g. requiring
  ≥N consecutive Latin letters as context).

## Alternatives considered

- **Option A (wrap 〔...〕 as an afm annotation):** consume the whole bracket
  span as an `AozoraNode::Annotation`, side-stepping the code-span issue
  without spec implementation. Rejected because it surfaces the ASCII digraphs
  in the HTML output, producing user-visible `fune\`bre` instead of `funèbre`.
- **Option B (disable CommonMark code spans when aozora is enabled):**
  rejected as a 100%-CommonMark-compatibility regression.
- **Option D (vendor-modify the fixture):** rejected; the 100%-Aozora-
  compatibility contract is "parse unmodified published works."
- **Option E (relax Tier A to permit `<code>`-wrapped leaks):** rejected as
  a bug-hiding workaround.

## References

- Official spec: <https://www.aozora.gr.jp/accent_separation.html>
- ADR-0001: fork/vendor strategy
- ADR-0003: afm-parser architecture
- Memory note: `feedback_read_aozora_spec_first` — spec-first response to
  Aozora encoding surprises
