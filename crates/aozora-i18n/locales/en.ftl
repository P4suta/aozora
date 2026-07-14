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

## fmt batch UX
##
## Chrome for a directory `aozora fmt` run, drawn to stderr and gated to an
## interactive terminal (off under `--quiet`, never over `--json`). Only these
## strings localize: the progress bar/spinner structure, the formatted output,
## the `--json` envelope, and the exit code are the machine axis (ADR-0033).

# Spinner message while `aozora fmt DIR/` walks directories for source files —
# indeterminate, since the file count is not yet known.
fmt-progress-discovering = discovering source files…

# End-of-run batch summary. $formatted files were changed (or, under --check /
# --list, would change), $unchanged were already canonical, $errors could not
# be read or formatted.
fmt-summary = {$formatted} formatted, {$unchanged} unchanged, {$errors} errors

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

# Shown (to stderr, exit non-zero) when `aozora explain <TARGET>` names nothing
# the resolver recognises. $target is the unrecognised argument.
explain-unknown = unknown explain target `{$target}`
# Appended to the line above when a near neighbour of the unknown target exists
# (edit-distance match over node tags, concepts, and diagnostic codes).
# $suggestion is the closest known target — a literal identifier, not localized.
explain-did-you-mean = did you mean `{$suggestion}`?
# Tail listing where the valid set lives. `aozora spec kinds` and the example code
# are literal shell text and stay the same in every locale.
explain-unknown-hint =
    expected a NodeKind tag or notation concept (run `aozora spec kinds`), or a
    diagnostic code such as `aozora::lex::unclosed_bracket`

## Notation concepts
##
## `aozora explain <concept>` prose for notation families the reader is likely
## to type but that are not a one-to-one NodeKind handbook page — abbreviations
## (`tcy`) and Japanese names (`傍点`, `ルビ`, …). One `concept-<slug>-title`
## (headline) and `concept-<slug>-body` (short prose) per family. The aozora
## notation glyphs woven in are literal syntax, the same in every locale.

concept-ruby-title = Ruby (ルビ) — reading gloss
concept-ruby-body =
    A small reading printed alongside a base run: 青空《あおぞら》. A leading `｜`
    pins an explicit base when the extent is otherwise ambiguous: ｜青空《あおぞら》.

    Run `aozora explain ruby` for the full handbook page.

concept-gaiji-title = Gaiji (外字) — non-Unicode character reference
concept-gaiji-body =
    A character with no plain-Unicode spelling, written as a ※［＃…］ reference —
    typically a JIS X 0213 men-ku-ten cell (`第3水準1-15-23`) or a `U+XXXX` code.

    Run `aozora explain gaiji` for the full handbook page; see also the
    `unresolved_gaiji` diagnostic.

concept-kaeriten-title = Kaeriten (返り点) — kanbun return marks
concept-kaeriten-body =
    Return marks that reorder classical Chinese (kanbun) for Japanese reading,
    written as ［＃…］ annotations — the レ mark and the 一/二/上/下/甲/乙 families.

    Run `aozora explain kaeriten` for the full handbook page; see also the
    `bracketed_kaeriten_no_pair` and `kaeriten_outside_kanbun` diagnostics.

concept-bouten-title = Bouten (傍点) — emphasis dots
concept-bouten-body =
    Emphasis dots set alongside a run, the Aozora counterpart of italics:
    ［＃「ここ」に傍点］. Sibling families set 傍線 (lines) instead of dots.

    Run `aozora explain bouten` for the full handbook page; see also the
    `bouten_target_ambiguous` diagnostic.

concept-warichu-title = Warichu (割注) — split-line inline note
concept-warichu-body =
    A note set in two half-height lines inside the main run, opened with
    ［＃割り注］ and closed with ［＃割り注終わり］.

    Run `aozora explain warichu` for the full handbook page.

concept-tcy-title = Tate-chu-yoko (縦中横) — horizontal run in vertical text
concept-tcy-body =
    A short horizontal run — usually two-digit numerals — set upright within
    vertical text, written ［＃「25」は縦中横］.

    The NodeKind tag is `combineUpright`; see also the `tcy_target_not_found`
    diagnostic.

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

## doctor
##
## `aozora doctor` — the end-user runtime self-check (distinct from the
## contributor `just doctor`). Section headings, status words, and hints are
## localized; the setting / tool identifiers, enum tags, source labels
## (flag / env / project / global / default), and tool versions woven into the
## report are machine vocabulary and stay literal in every locale.

doctor-title = aozora doctor — runtime self-check
doctor-config-heading = Configuration
doctor-settings-heading = Effective settings
doctor-tools-heading = External tools
doctor-terminal-heading = Terminal

# Configuration section. $dir is the working directory the upward `.aozora.toml`
# search began from; $error is the underlying (English) loader message a
# malformed file surfaces.
doctor-project-none = none (searched up from {$dir})
doctor-global-none = none
doctor-parse-ok = configuration parsed cleanly (no unknown keys)
doctor-parse-error = configuration error: {$error}

# Effective-settings section. A blocking row: $var is set to $value, which the
# CLI runtime's clap parser rejects (a case-sensitive value-enum mismatch, or a
# bool that is not exactly true / false). $var and $value stay literal.
doctor-setting-rejected = {$var}={$value} is set but not a valid value; aozora would reject it

# External tools. The hint line follows a missing tool.
doctor-tool-missing = not found on PATH
doctor-hint-pandoc = needed for `aozora pandoc -t FMT`; install from https://pandoc.org
doctor-hint-lsp = needed for `aozora lsp`; part of the aozora toolchain

# Terminal section. $value is the raw environment-variable value when set.
doctor-terminal-yes = a terminal
doctor-terminal-no = not a terminal
doctor-env-set = set ({$value})
doctor-env-unset = unset
doctor-colour-label = effective colour
doctor-colour-on = on
doctor-colour-off = off

# Closing summary. $count is the number of blocking problems.
doctor-all-passed = All checks passed.
doctor-problems = {$count} problem(s) found.

## init
##
## `aozora init` — scaffold a new project. Only the report chrome is
## localized: the scaffolded file names, the file contents themselves, and the
## literal `aozora …` next-step commands are language-neutral project
## artifacts, identical in every locale.

init-heading = aozora init — scaffold a project

# Per-file outcome words shown before each scaffolded file name.
init-created = created
init-overwritten = overwritten
init-skipped = skipped
# Parenthetical after a skipped (already-present) file; `--force` is a literal
# flag name and stays the same in every locale.
init-skipped-hint = already exists; use --force to overwrite

# Next-steps footer. The `aozora …` commands are literal; only the trailing
# comments here are localized.
init-next-steps = Next steps:
init-step-render = render the sample to HTML
init-step-check = report diagnostics
init-step-doctor = verify the effective configuration

## repl
##
## `aozora repl` — the interactive read-eval-print loop. Everything here is
## human chrome: the banner, the section labels, the meta-command
## acknowledgements, the help, and the inline errors are localized. The view
## *contents* the loop wraps — node JSON, rendered HTML, the Pandoc AST, and the
## English diagnostic report — are the machine axis and are never routed through
## here (ADR-0033). The `:command` names and their literal arguments stay the
## same in every locale.

# Shown once at startup.
repl-banner = aozora repl — type notation to see nodes / HTML / diagnostics. :help for commands, :quit to exit.

# `:help` — the meta-command reference. Only the trailing descriptions localize.
repl-help =
    Commands:
      :mode  nodes | html | pandoc | all   choose which view(s) to show
      :lang  en | ja | zh                  message language of this chrome
      :encoding  auto | utf8 | sjis        decoder used by :load
      :load  FILE                          parse a file's contents
      :help                                show this help
      :quit                                leave the loop (or Ctrl-D)

    Type a line of Aozora notation to see it parsed.

# Section labels prefixing each shown view and the diagnostics block.
repl-label-nodes = nodes:
repl-label-html = html:
repl-label-pandoc = pandoc:
repl-label-diag = diagnostics:
# Placeholder in the diagnostics block when the parse is clean.
repl-diag-none = (no diagnostics)

# Acknowledgements after a `:mode` / `:lang` / `:encoding` switch. The value
# ($mode / $lang / $encoding) is a literal tag and stays the same in every locale.
repl-mode-set = mode → {$mode}
repl-lang-set = language → {$lang}
repl-encoding-set = encoding → {$encoding}

# `:load` — the header shown before a loaded file's evaluation, and the inline
# (non-fatal) read/decode error. $path is the file; $error is the English
# engine message.
repl-loaded = loaded {$path}
repl-load-error = cannot read {$path}: {$error}

# An unrecognised `:command` ($cmd, without the leading colon), and the usage
# line for a missing / invalid argument ($expected lists the accepted values).
repl-unknown-meta = unknown command `:{$cmd}` — type :help for the list
repl-usage = usage: :{$cmd} {$expected}

## TUI live editor
##
## Chrome for the full-screen `aozora tui` editor: the three pane titles, the
## unsaved-changes marker, the clean-parse placeholder, the footer keybind
## legend (action words only — the ^S / ^L / ^P / ^Q glyphs and the html /
## nodes / en literals stay the same in every locale), and the save / error
## status lines. The pane *contents* — rendered HTML, node JSON, the Pandoc
## AST, and the English diagnostic report — are the machine axis and are never
## routed through here (ADR-0033).

# Pane titles (a file path, view tag, or diagnostic count is appended in code).
tui-title-source = source
tui-title-preview = preview
tui-title-diagnostics = diagnostics
# The unsaved-changes marker appended to the source title.
tui-modified = modified
# Placeholder in the diagnostics pane when the parse is clean.
tui-diag-none = (no diagnostics)

# Footer keybind legend — the action word after each Ctrl glyph.
tui-key-save = save
tui-key-lang = lang
tui-key-preview = preview
tui-key-quit = quit

# Footer status after Ctrl-S. $path is the saved file; $error is the English
# OS message. The no-file line fires when the buffer was opened without a path.
tui-saved = saved {$path}
tui-save-error = cannot save {$path}: {$error}
tui-no-file = no file to save — reopen with a path: aozora tui FILE
# Refusal when stdout / stdin is not a terminal (piped): the TUI needs a tty.
tui-no-tty = aozora tui needs an interactive terminal (try aozora repl or --watch)

## LSP editor surface
##
## Human-facing chrome emitted by aozora-lsp: the gaiji hover / inlay tooltip,
## code-action titles, and completion detail / documentation. The protocol
## data axis (custom-method payloads, diagnostic ranges / codes, semantic
## tokens, formatting edits) is never routed through here. The notation glyphs
## and Tab-stop templates woven into these strings are literal aozora syntax,
## the same in every locale; only the surrounding prose is translated.

# Gaiji (外字) reference — hover header and the labels in its Markdown body.
lsp-hover-gaiji-header = **Gaiji (外字)**
lsp-hover-resolved-label = Resolved
lsp-hover-composed-seq-label = composed sequence
lsp-hover-unresolved = (no dictionary match — showing the description instead)
lsp-hover-description-label = Description
# Gaiji inlay-hint tooltip header (the resolved label above is reused).
lsp-inlay-gaiji-header = **Gaiji**

# Code-action (quick-fix / refactor) titles shown in the editor's lightbulb
# menu. `SEL` marks where the current selection lands inside the woven glyphs.
lsp-action-ruby = Add ruby ｜SEL《》
lsp-action-ruby-double = Add double ruby ｜SEL《《》》
lsp-action-wrap-quote = Wrap in 「」
lsp-action-wrap-accent = Wrap in 〔〕 (accent decomposition)
lsp-action-wrap-annotation = Turn into a ［＃...］ annotation
lsp-action-bouten = Add emphasis dots ［＃「SEL」に傍点］
# $close is the missing close glyph, $open the pair's open glyph.
lsp-action-close-bracket = Insert `{$close}` to close ({$open} pair)
# $close is the stray close glyph with no matching open.
lsp-action-delete-unmatched = Delete the unmatched `{$close}`
# $directive is the full canonical ［＃…］ the near-miss is rewritten to.
lsp-action-rewrite = Rewrite to `{$directive}`
# $codepoint is the private-use scalar in `04X` hex (no `U+` prefix).
lsp-action-delete-pua = Delete private-use character U+{$codepoint}

# Completion detail / documentation fragments.
lsp-completion-half-to-full-hint = (half-width → full-width)
lsp-completion-takes-param = (takes a parameter)

# Half-width → full-width "emmet" completion details. Each names the target
# and the half-width trigger that produces it.
lsp-emmet-ruby-open = Ruby reading (half-width 『<』 → full-width pair 『《》』)
lsp-emmet-ruby-close = Ruby reading close (half-width 『>』 → full-width 『》』)
lsp-emmet-bracket-open = Full-width left bracket (half-width 『[』 → full-width 『［』)
lsp-emmet-bracket-close = Full-width right bracket (half-width 『]』 → full-width 『］』)
lsp-emmet-ruby-base = Ruby base marker (half-width 『|』 → full-width 『｜』)
lsp-emmet-gaiji-marker = Gaiji marker (half-width 『*』 → full-width 『※』)
# $prefix is the typed half-width char, $glyph the full-width target.
lsp-emmet-doc = Half-width `{$prefix}` → `{$glyph}`

# Structured-snippet completion detail / documentation. The `${…}` and
# `<…>` placeholders mirror the Tab-stop slots the snippet body fills in.
lsp-snippet-empty-wrap-detail = Empty annotation-slug template (edit the body)
lsp-snippet-empty-wrap-doc = Convert `#` to `［＃<cursor>］`. Press Enter to confirm.
lsp-snippet-ruby-detail = Ruby ｜base《reading》 (Tab to the reading)
lsp-snippet-ruby-doc = Insert `<base>《<reading>》` after `｜`. Start at `<base>`, Tab to `<reading>`.
lsp-snippet-reading-detail = Ruby reading (auto-closes the bracket)
lsp-snippet-reading-doc = Insert `<reading>》` after `《`. Edit `<reading>`.
lsp-snippet-gaiji-detail = Gaiji annotation (description, mencode)
lsp-snippet-gaiji-doc = Insert `［＃「<desc>」、<men>］` after `※`. Start at `<desc>`, Tab to `<men>`.

# Document-outline placeholder for a heading whose title is not yet typed.
lsp-outline-untitled = (untitled)
