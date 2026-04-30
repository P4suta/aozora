# CLI reference

Full reference for the `aozora` binary. For a guided tour, see
[CLI Quickstart](../getting-started/cli.md).

## Synopsis

```text
aozora [OPTIONS] <SUBCOMMAND> [ARGS]
```

Subcommands:

| Subcommand | What it does |
|---|---|
| `check` | Lex + report diagnostics. |
| `fmt` | Round-trip parse ∘ serialize. |
| `render` | Render to HTML on stdout. |

Global options apply to every subcommand:

| Option | Effect |
|---|---|
| `-E sjis`, `--encoding sjis` | Decode Shift_JIS source. Default is UTF-8. |
| `--no-color` | Disable ANSI colour in diagnostics output. |
| `--verbose` | Print parse phase timings to stderr. |
| `--diagnostics LEVEL` | Filter diagnostics by minimum level (`error` \| `warning` \| `info`). Default: `warning`. |
| `-V`, `--version` | Print version and exit. |
| `-h`, `--help` | Print help and exit. |

## `aozora check`

```text
aozora check [OPTIONS] [PATH]
```

Lex the source and print diagnostics. `PATH` of `-` (or omitted)
reads from stdin.

| Option | Effect |
|---|---|
| `--strict` | Exit non-zero on any diagnostic. |

Exit codes: `0` on parse success (regardless of diagnostics, unless
`--strict`); `1` on diagnostics under `--strict`; `2` on usage error.

```sh
aozora check src.txt                # warnings shown, exit 0
aozora check --strict src.txt       # warnings -> exit 1
aozora check -E sjis crime.txt      # SJIS source
cat src.txt | aozora check          # stdin
```

## `aozora fmt`

```text
aozora fmt [OPTIONS] [PATH]
```

Round-trip the source through `parse ∘ serialize`. Default behaviour
prints the canonical form on stdout.

| Option | Effect |
|---|---|
| `--check` | Exit non-zero if the formatted output differs from the input. Don't print the canonical form. |
| `--write` | Overwrite the input file with the canonical form. (Ignored when reading from stdin.) |

Exit codes: `0` on success (or no diff under `--check`); `1` on a
formatting mismatch under `--check`; `2` on usage error.

```sh
aozora fmt src.txt > formatted.txt
aozora fmt --check src.txt          # CI gate
aozora fmt --write src.txt          # in-place
cat src.txt | aozora fmt            # stdin → stdout
```

## `aozora render`

```text
aozora render [OPTIONS] [PATH]
```

Render the parsed tree to HTML on stdout.

```sh
aozora render src.txt > out.html
aozora render -E sjis crime.txt > crime.html
cat src.txt | aozora render -
```

The output is semantic HTML5 with `aozora-*` class hooks (no inline
styles). See [HTML renderer](../arch/renderer.md#class-name-scheme)
for the class-name reference.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Success. |
| `1` | Diagnostics emitted under `--strict`, or formatting mismatch under `--check`. |
| `2` | Usage error (bad flag, missing file, decode error). |

## Environment

| Variable | Effect |
|---|---|
| `NO_COLOR` | If set (any value), disable ANSI colour output. Same as `--no-color`. |
| `AOZORA_LOG` | `tracing-subscriber` filter (e.g. `aozora_lex=debug`). For internal debugging; not part of the stable surface. |

See [Reference → Environment variables](env.md) for the full env
matrix (which includes the bench / profiling vars).

## See also

- [CLI Quickstart](../getting-started/cli.md) — examples and the
  three-subcommand rationale.
- [Notation overview](../notation/overview.md) — what the parser
  recognises.
- [Diagnostics catalogue](../notation/diagnostics.md) — the codes
  you'll see in `check`'s output.
