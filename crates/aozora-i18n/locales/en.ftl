### Aozora CLI shell strings — English (canonical source locale).
###
### Localizes the CLI's own human-facing chrome only: the stdin guard, the
### `--watch` banner, and the `explain` footer / section labels. The machine
### axis (json / short / codes / exit / schema / timing-json) is never routed
### through here. See docs/adr/0033-cli-output-language-policy.md.

## Input guards

# Shown when a document subcommand would read stdin on a bare interactive
# terminal — it would otherwise block forever waiting for typed input. $cmd is
# the subcommand tag shown in the copy-pasteable examples (e.g. "check" or
# "inspect nodes").
stdin-empty =
    error: standard input is empty (reading from a terminal)
      hint: read a file →  aozora {$cmd} <FILE>
            or a pipe   →  cat f.txt | aozora {$cmd}
      all commands:  aozora --help

## Watch mode

# Printed to a terminal between `--watch` re-runs. $path is the watched file.
watch-banner = ── watching {$path} (Ctrl-C to stop) ──

## explain

# Footer after `aozora check`'s human diagnostics, pointing the reader at
# `aozora explain <code>`. The per-code command lines listed under it are
# literal shell commands and stay un-localized.
explain-hint-header = help: run `aozora explain <code>` for details, e.g.
# Tail shown when more distinct codes exist than the footer lists; $count is
# the remainder.
explain-hint-more = … and {$count} more

# Section labels inside `aozora explain <code>` output. The reproduction /
# fixed *examples* are language-neutral Aozora notation owned by aozora-spec;
# the localized title / body prose is below, keyed by diagnostic code.
explain-repro-label = Reproduction:
explain-fixed-label = After fix:
explain-see-label = see:

## Diagnostic prose
##
## One `diag-<slug>-title` (headline) and `diag-<slug>-body` (long form) per
## diagnostic code, where <slug> is the code's trailing `::` segment with `_`
## turned into `-` (e.g. `aozora::lex::unclosed_bracket` → `unclosed-bracket`).
## The body's `{$…}` placeables are filled from the live diagnostic by the
## consumer (CLI `explain`, LSP hover). This prose was migrated out of
## aozora-spec so the catalogue crate stays a pure machine contract; the codes,
## severities, `#[error]` Display strings, and JSON are never localized.

diag-source-contains-pua-title = Private-use codepoint in the source
diag-source-contains-pua-body =
    The source contains the private-use codepoint `U+{$codepoint}`.

    This character (`{$char}`) is a reserved codepoint that never appears in ordinary Aozora Bunko text; it collides with aozora-lex's internal markers (U+E001..U+E004).
    It usually creeps in through an editor's invisible-character setting or an unseen character pasted from elsewhere.

    How to fix: delete that single character.

diag-unclosed-bracket-title = Unclosed opening bracket
diag-unclosed-bracket-body =
    There is an unclosed `{$open}`.

    Add the matching `{$close}` somewhere — in Aozora notation a pair is normally closed within the same line.

    Example: {$example}

diag-unmatched-close-title = Close bracket with no matching open
diag-unmatched-close-body =
    A `{$close}` with no matching `{$open}`.

    Possible causes:
    1. an extra `{$close}` was typed → delete it
    2. the `{$open}` that should precede it is missing → add it in the right place
    3. another `{$close}` in between shifted the pairing by one → review the pairs around it

diag-accent-decomposition-applied-title = Accent decomposition applied (note)
diag-accent-decomposition-applied-body =
    An `〔…〕` accent notation was decomposed into its combined Unicode form during the sanitize stage (e.g. 〔e'〕 → é).

    This is intended behaviour (ADR-0003), surfaced as an informational note.

    How to fix: nothing to do. Serializing restores the original 〔…〕 form, so the transform is loss-free.

diag-unresolved-gaiji-title = Gaiji reference could not be resolved
diag-unresolved-gaiji-body =
    A gaiji reference (※［＃…］) resolved to neither a Unicode character nor a JIS X 0213 men-ku-ten cell.

    As a result the renderer shows the description text verbatim instead of the intended glyph.

    How to fix: give the reference a resolvable specifier — a men-ku-ten such as `第3水準1-15-23`, or a `U+XXXX` Unicode reference.

diag-mismatched-container-close-title = Container closed by a different kind
diag-mismatched-container-close-body =
    The container was opened as `{$open_kind}` but closed by a `{$close_kind}` close directive.

    The open and close families disagree, so the range cannot be resolved correctly.

    How to fix: close with the family you opened — pair `ここから字下げ` with `ここで字下げ終わり`, `ここから地付き` with `ここで地付き終わり`, and so on.

diag-empty-ruby-reading-title = Empty ruby reading
diag-empty-ruby-reading-body =
    An explicit-base ruby (｜base《…》) has a base but an empty reading.

    Because a `｜` precedes it, this is a genuine slip rather than a bare 《》 run, and the ruby degrades to plain text.

    How to fix: supply a reading (｜青空《あおぞら》), or, to drop the ruby, remove the ｜…《》 markers entirely and keep the base as body text.

diag-nested-ruby-title = Nested ruby inside a reading
diag-nested-ruby-body =
    Inside a ruby reading, another ruby (《…》) is opened.

    Ruby cannot nest, so the inner 《…》 is the offending part; the outer ruby is still interpreted as far as possible.

    How to fix: close the outer reading before the inner 《, or remove the inner 《…》.

diag-unrecognised-container-directive-title = Unrecognised container directive
diag-unrecognised-container-directive-body =
    `［＃ここから…］` looks like a container opener but names no known container (字下げ / 地付き / 地から N 字上げ, …).

    The output is preserved, but it is not treated as a container and remains a plain annotation.

    How to fix: change it to a known container name (e.g. ［＃ここから2字下げ］).

diag-tcy-target-not-found-title = 縦中横 target not found before it
diag-tcy-target-not-found-body =
    A 縦中横 forward reference (［＃「X」は縦中横］) names a target X that appears nowhere in the preceding text.

    With no run to style, the directive degrades to an Unknown annotation.

    How to fix: the target must occur earlier on the same line. Check the spelling, or place ［＃「X」は縦中横］ after the run it styles.

diag-bouten-target-ambiguous-title = Ambiguous bouten target
diag-bouten-target-ambiguous-body =
    The target X of a bouten forward reference (［＃「X」に傍点］) occurs more than once in the preceding text.

    Which occurrence gets the emphasis dots is not unique, so the wrong run may be styled (the parser picks one by its look-back rule).

    How to fix: reword so the target is unique (e.g. narrow it to 「白い花」).

diag-forward-referent-not-stylable-title = Forward-reference target not stylable in place
diag-forward-referent-not-stylable-body =
    The forward-reference target X does exist earlier, but it cannot be styled in place — it is a ruby base, on an earlier line, inside another construct, or one of several candidates.

    The directive is kept and the text round-trips, but the styling is not applied to the earlier run.

    How to fix: move the ［＃…］ next to a plain occurrence of the target.

diag-break-in-single-line-container-title = Page/section break inside a single-line container
diag-break-in-single-line-container-body =
    A page/section break appears on the same line as a single-line container (`{$container}`).

    A single-line container governs only the rest of its line, so a break on that line drops the container's effect.

    How to fix: move the break off the line, or use the block form ［＃ここから…］ … ［＃ここで…終わり］ that persists across breaks.

diag-bracketed-kaeriten-no-pair-title = Bracketed kaeriten with no matching base
diag-bracketed-kaeriten-no-pair-body =
    A bracketed kaeriten (［＃二］ / ［＃下］ / ［＃乙］, …) has no matching family base (［＃一］ / ［＃上］ / ［＃甲］) anywhere in the document.

    With nothing to return to, it does not hold as a return mark.

    How to fix: place the family base somewhere in the document — ［＃二］/［＃三］ need ［＃一］, ［＃下］/［＃中］ need ［＃上］, ［＃乙］… need ［＃甲］.

diag-kaeriten-outside-kanbun-title = Kaeriten outside a kanbun context
diag-kaeriten-outside-kanbun-body =
    A kaeriten (［＃二］ / ［＃レ］, …) appears outside a kanbun-like context — it is the only kaeriten in the document and its surroundings read as ordinary kana prose.

    It was judged more likely to be a stray annotation than a genuine return mark.

    How to fix: if it is a real 返り点, use it in a kanbun context; otherwise delete the ［＃…］ annotation.

diag-mismatched-bouten-container-title = Bouten/bouten-line range opened and closed by different families
diag-mismatched-bouten-container-body =
    A 傍点/傍線 range was opened as `{$open_family}` but closed by a `{$close_family}` closer.

    Dots and lines render differently, so the run's emphasis is ambiguous (the parser recovers using the opener's family).

    How to fix: close with the family you opened — 傍点 with ［＃傍点終わり］, 傍線 with ［＃傍線終わり］.

diag-non-canonical-directive-title = Non-canonically spelled ［＃…］ annotation
diag-non-canonical-directive-body =
    A non-canonically spelled ［＃…］ annotation. The canonical form is `［＃{$canonical}］`.

    Its body was judged to be a recognized notation written non-canonically (okurigana drift, a synonym, or a spelling variant), so it is kept as an Unknown annotation; the parser does not rewrite it.

    How to fix: rewrite it to `［＃{$canonical}］`. `aozora fmt --fix` can do this automatically.

diag-residual-annotation-marker-title = Unclassified ［＃…］ annotation (pipeline-internal)
diag-residual-annotation-marker-body =
    An unclassified ［＃...］ annotation (pipeline-internal).

    It did not match a keyword in the annotation dictionary (gaiji_chuki), or it may be a typo.

    How to check:
    1. verify the ［＃ body matches a registered keyword such as `改ページ` / `中央揃え`
    2. check a JIS X 0213 men-ku-ten code such as `第3水準1-...` was not omitted
    3. if still unclear, the description-only form (※［＃「説明」］) will pass for now

diag-unregistered-sentinel-title = Unregistered internal sentinel (pipeline-internal error)
diag-unregistered-sentinel-body =
    An unregistered private-use sentinel was detected (a pipeline-internal consistency error).

    This is most likely a bug in aozora-pipeline. Please report it with steps to reproduce: https://github.com/P4suta/aozora/issues

diag-registry-out-of-order-title = Placeholder registry out of order (pipeline-internal error)
diag-registry-out-of-order-body =
    The placeholder registry is out of order (a pipeline-internal consistency error).

    This may be a bug in aozora-pipeline. Please report it with steps to reproduce: https://github.com/P4suta/aozora/issues

diag-registry-position-mismatch-title = Placeholder registry position mismatch (pipeline-internal error)
diag-registry-position-mismatch-body =
    A placeholder registry entry's position disagrees with the expected sentinel (a pipeline-internal consistency error).

    This may be a bug in aozora-pipeline. Please report it with steps to reproduce: https://github.com/P4suta/aozora/issues
