# CLI Quickstart

The `aozora` binary covers three operations:

```sh
aozora check  FILE.txt          # lex + report diagnostics on stderr
aozora fmt    FILE.txt          # round-trip parse ∘ serialize, print to stdout
aozora render FILE.txt          # render to HTML on stdout
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
| `-E sjis`, `--encoding sjis` | all | Decode Shift_JIS source. Default is UTF-8. |
| `--strict` | `check` | Exit non-zero on any diagnostic. |
| `--check` | `fmt` | Exit non-zero if formatted output differs from input. |
| `--write` | `fmt` | Overwrite the input file with the canonical form. (Ignored when reading from stdin.) |
| `--no-color` | all | Disable ANSI colour in diagnostics output. |
| `--verbose` | all | Print parse phase timings to stderr. |

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Success. |
| `1` | Diagnostics emitted under `--strict`, or formatting mismatch under `--check`. |
| `2` | Usage error (bad flag, missing file, decode error). |

## Diagnostics format

`aozora check` prints diagnostics in
[`miette`](https://docs.rs/miette) style — a coloured source snippet
with carets pointing at the byte range, a short message, and (where
applicable) a help line:

```text
  × ruby reading mismatch: target spans 3 chars but ｜《》 reading is empty
   ╭─[input.txt:42:9]
42 │ ｜青梅《》
   · ───┬───
   ·    ╰── empty reading
   ╰────
  help: provide a reading inside 《…》 or remove the ｜ marker
```

Every diagnostic carries a stable error code (`E0001`, `E0002`, …);
see the [Diagnostics catalogue](../notation/diagnostics.md) for the
full list.

## Why not a single subcommand?

`check` / `fmt` / `render` are intentionally separate so each one has
a single, predictable failure mode in shell pipelines:

- `check` exits 0 on parse success, regardless of warnings (use
  `--strict` for "no diagnostics allowed").
- `fmt` is a *pure-text* transform: stdin in, canonical text out.
  `--check` upgrades it to a CI gate without forking a second binary.
- `render` is a *pure-text-to-HTML* transform with the same
  exit-code shape.

Combining them behind flags would make the exit-code semantics
ambiguous (does `--check` mean format-check or strict-check?). Keeping
them split is the same logic that splits `gofmt` from `vet` from
`go build`.
