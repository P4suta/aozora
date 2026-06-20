# Batch: check or render many files at once

**Problem.** You have a directory of files and want to check or render
them all. The `aozora` binary is deliberately single-file — every
document subcommand takes one path (or `-` for stdin), with no glob and
no file-list argument. Batching is the shell's job, which keeps the CLI
small and lets it compose with `find`, `xargs`, GNU `parallel`, and the
CI runner you already have.

## Check every file, stop at the first problem

```sh
for f in *.txt; do
  aozora check --strict "$f" || break
done
```

`--strict` turns any diagnostic into a non-zero exit, so the loop stops
at the first file that is not clean.

## Check every file, then fail if any failed

```sh
status=0
for f in *.txt; do
  aozora check --strict "$f" || status=1
done
exit "$status"
```

Every file is still checked; the loop exits non-zero if any of them
did — the shape a CI gate wants.

## Recurse a directory tree

`aozora` takes a single path, so each file needs its own invocation —
use `-exec aozora check {} \;` (one run per file), **not** `-exec …
{} +` (which packs many paths into a single call and errors):

```sh
find . -name '*.txt' -exec aozora check --strict {} \;

# Or, one path per invocation via xargs (robust to odd filenames):
find . -name '*.txt' -print0 | xargs -0 -n1 aozora check --strict
```

## Render a whole directory

```sh
for f in *.txt; do
  aozora render "$f" > "${f%.txt}.html"
done
```

## Collect machine-readable diagnostics

Diagnostics print to **stderr**, so redirect that stream to feed `jq`
(see [Diagnostics as JSON](diagnostics-json.md) for the envelope shape).
A clean file prints nothing, so only files with diagnostics appear:

```sh
for f in *.txt; do
  aozora check --diagnostic-format json "$f" 2>&1 >/dev/null
done | jq -s 'map(.data) | add'
```

## Exit codes

The loops above lean on `aozora`'s exit-code contract:

| Code | Meaning |
|---|---|
| `0` | Success — diagnostics may print but are tolerated. |
| `1` | Diagnostics under `--strict` (or an `fmt --check` mismatch). |
| `2` | Usage error — bad flag, unreadable file, decode failure. |
| `3` | An internal diagnostic fired — a library bug, not bad input. |

Treat `3` specially in CI: it means the parser, not the file, is at
fault.

## See also

- [Diagnostics as JSON](diagnostics-json.md) — the per-file JSON path
  and the envelope schema.
- [Round-trip & fmt --check](round-trip.md) — gate a tree on canonical
  form.
- [CLI reference](../ref/cli.md) — every flag and the full exit-code
  table.
