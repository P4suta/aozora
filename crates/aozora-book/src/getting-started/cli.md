# CLI Quickstart

The `aozora` binary covers six document operations:

```sh
aozora check   FILE.txt         # lex + report every diagnostic on stderr
aozora lint    FILE.txt         # report notation-hygiene lints (--fix rewrites)
aozora fmt     FILE.txt         # round-trip parse ∘ to_source, print to stdout
aozora render  FILE.txt         # render to HTML on stdout
aozora inspect nodes FILE.txt   # emit parsed data as JSON (nodes/pairs/gaiji/…)
aozora pandoc  FILE.txt         # project to a Pandoc AST (JSON on stdout)
```

`-` (or no path argument) reads from stdin. `--encoding sjis` (alias
`-E sjis`) decodes Shift_JIS source — Aozora Bunko's distributed
`.txt` files are Shift_JIS, so this flag is the common case for real
corpus work.

## Common invocations

```sh
# Lex an Aozora Bunko file and print diagnostics
aozora check -E sjis crime_and_punishment.txt

# Render to HTML (stdout)
aozora render -E sjis crime_and_punishment.txt > out.html

# Pipe from stdin
cat src.txt | aozora render -

# CI gate: fail if format is not idempotent
aozora fmt --check src.txt
```

## Flag reference

| Flag | Subcommand | Effect |
|---|---|---|
| `-E sjis`, `--encoding sjis` | all | Decode Shift_JIS source. Default `auto` (UTF-8, else Shift_JIS). |
| `--strict` | `check` / `lint` | Exit non-zero on any diagnostic / lint. |
| `--fix` | `fmt` / `lint` | Rewrite flagged directive near-misses to canonical form (Tier1). |
| `--check` | `fmt` | Exit non-zero if formatted output differs from input. |
| `--write` | `fmt` | Overwrite the input file with the canonical form. (Ignored on stdin.) |
| `--diff` / `--list` / `--json` | `fmt` | Report what would change (unified diff / paths / JSON) without writing. |
| `--normalize` / `--degraded` | `render` | Render near-misses as their canonical (Tier1) / degraded (Tier2) form. |
| `--color {auto,always,never}` | all | ANSI colour policy (global). Honours `NO_COLOR` / `CLICOLOR`. |
| `--timing` | all | Print per-phase timing to stderr (stdout stays byte-identical). |

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Success. |
| `1` | Diagnostics emitted under `--strict`, or formatting mismatch under `--check`. |
| `2` | Usage error (bad flag, missing file, decode error). |
| `3` | An `Internal`-source diagnostic fired during `check` — a library bug, not bad input; please report it. |

Piping into a reader that quits early — `aozora render FILE | head` — exits `0`
quietly: the broken pipe is a success, not an error
([ADR-0029](https://github.com/P4suta/aozora/blob/main/docs/adr/0029-broken-pipe-exit-semantics.md)).

## Diagnostics format

`aozora check` prints diagnostics in
[`miette`](https://docs.rs/miette/latest/miette/) style — the stable dotted
code and its catalogue URL, a source snippet with carets pointing at the byte
range, a short message, a help line, and a pointer to `aozora explain`.
Running `check` on a one-line `input.txt` whose only content is `｜青空《》`
(an explicit ruby base with an empty `《》` reading) prints:

```text
aozora::lex::empty_ruby_reading (https://p4suta.github.io/aozora-notation-spec/diagnostics.html#empty-ruby-reading)

  × ruby base given but reading is empty
   ╭─[input.txt:1:1]
 1 │ ｜青空《》
   · ─────┬────
   ·      ╰── empty reading
   ╰────
  help: the `《…》` reading after the `｜` base is empty — supply a reading or
        remove the `｜…《》` markers to keep the base as plain text

help: run `aozora explain <code>` for details, e.g.
      aozora explain empty_ruby_reading
```

Every diagnostic carries a stable dotted code (here
`aozora::lex::empty_ruby_reading`); run `aozora explain <code>` for the same
help and URL, or see the [Diagnostics catalogue](../notation/diagnostics.md)
for the full list.

## Why not a single subcommand?

`check` / `lint` / `fmt` / `render` are intentionally separate so each one has
a single, predictable failure mode in shell pipelines:

- `check` exits 0 on parse success, regardless of warnings (use
  `--strict` for "no diagnostics allowed").
- `lint` is `check` filtered to the advisory notation-hygiene lints
  (`aozora::lint::*`); `--fix` applies the Tier1 autofix in place. See
  [Notation hygiene](https://github.com/P4suta/aozora/blob/main/docs/hygiene.md).
- `fmt` is a *pure-text* transform: stdin in, canonical text out.
  `--check` upgrades it to a CI gate without forking a second binary.
- `render` is a *pure-text-to-HTML* transform with the same
  exit-code shape.

Combining them behind flags would make the exit-code semantics
ambiguous (does `--check` mean format-check or strict-check?). Keeping
them split is the same logic that splits `gofmt` from `vet` from
`go build`.
