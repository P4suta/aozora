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

# Section labels inside `aozora explain <code>` output. The surrounding
# diagnostic prose (title / body / repro / fixed example) is spec-owned and is
# NOT localized here.
explain-repro-label = Reproduction:
explain-fixed-label = After fix:
explain-see-label = see:
