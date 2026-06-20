# CLI reference

Full reference for the `aozora` binary. For a guided tour, see
[CLI Quickstart](../getting-started/cli.md).

## Synopsis

```text
aozora <SUBCOMMAND> [OPTIONS] [ARGS]
```

| Subcommand | What it does |
|---|---|
| `check` | Lex + report diagnostics. |
| `fmt` | Round-trip `parse ∘ serialize` (canonicalise). |
| `render` | Render to HTML on stdout. |
| `wire` | Emit a document's wire JSON (`nodes`/`pairs`/`container-pairs`/`diagnostics`/`gaiji`) or the static `slugs` catalogue. |
| `pandoc` | Project to a Pandoc AST (JSON, or pipe through `pandoc`). |
| `kinds` | Tabulate every `NodeKind` / `PairKind` / `Severity` / … wire tag. |
| `schema` | Print the JSON Schema for a wire envelope. |
| `explain` | Print prose for a `NodeKind` tag, or help / severity / URL for a diagnostic code. |

There are **no global options** beyond clap's `-h`/`--help` and
`-V`/`--version`; the input-shaping flags below are per-subcommand. All
document subcommands accept `-` (or no path) to read **stdin**.

| Common flag | Subcommands | Effect |
|---|---|---|
| `-E`, `--encoding {auto,utf8,sjis}` | check / fmt / render / wire / pandoc | Source encoding. **Default `auto`** — UTF-8 if the bytes are valid UTF-8, else Shift_JIS. |

Colour follows the terminal and the `NO_COLOR` environment variable
(miette honours it); there is no `--no-color` flag.

## `aozora check`

```text
aozora check [OPTIONS] [PATH]
```

Lex the source and report diagnostics. `PATH` of `-` (or omitted) reads
from stdin.

| Option | Effect |
|---|---|
| `--strict`, `-s` | Exit non-zero (`1`) on any diagnostic. |
| `--encoding`, `-E` | Source encoding (see above). |
| `--diagnostic-format {human,json,short}` | How to render diagnostics. Default **`auto`**: `human` when stderr is a terminal, `json` when piped. |

The three formats:

- **`human`** — a graphical [`miette`](https://docs.rs/miette) report:
  the source line, a caret under the span, the label, the help, and a
  link to the [diagnostics catalogue](../notation/diagnostics.md).
- **`json`** — the `aozora::wire` diagnostics envelope, byte-identical to
  every other binding. The machine / agent path (the default when piped).
- **`short`** — one grep-able line: `path:offset: severity[code]: msg`.

Exit codes: `0` (parse succeeded; diagnostics may have been printed but
were tolerated), `1` (`--strict` and at least one diagnostic), `2` (usage
error), `3` (an `Internal`-source diagnostic fired — a library bug, not
bad input; please report it).

```sh
aozora check src.txt                       # human on a TTY, json when piped
aozora check --strict src.txt              # any diagnostic -> exit 1
aozora check -E sjis crime.txt             # Shift_JIS source
aozora check --diagnostic-format short -   # one line per diagnostic, from stdin
cat src.txt | aozora check                 # json envelope (stderr is piped)
```

## `aozora fmt`

```text
aozora fmt [OPTIONS] [PATH]
```

Round-trip the source through `parse ∘ serialize`. Default prints the
canonical form on stdout.

| Option | Effect |
|---|---|
| `--check` | Exit non-zero if the formatted output differs from the input (after Phase 0 sanitize: BOM strip, CRLF→LF). Mutually exclusive with `--write`. |
| `--write` | Overwrite the input file with the canonical form. Ignored when reading from stdin. |
| `--encoding`, `-E` | Source encoding (see above). |

Exit codes: `0` (success, or no diff under `--check`), `1` (formatting
mismatch under `--check`), `2` (usage error).

```sh
aozora fmt src.txt > formatted.txt
aozora fmt --check src.txt                 # CI gate
aozora fmt --write src.txt                 # in-place
cat src.txt | aozora fmt                    # stdin -> stdout
```

## `aozora render`

```text
aozora render [OPTIONS] [PATH]
```

Render the parsed tree to HTML on stdout. Accepts `--encoding`/`-E`.

```sh
aozora render src.txt > out.html
aozora render -E sjis crime.txt > crime.html
cat src.txt | aozora render -
```

The output is semantic HTML5 with `aozora-*` class hooks (no inline
styles). See [HTML renderer](../arch/renderer.md#class-name-scheme) for
the class-name reference.

## `aozora wire`

```text
aozora wire <KIND> [OPTIONS] [PATH]
```

Emit a parsed document's data as the shared `aozora::wire` JSON
envelope — the **data** counterpart to `aozora schema` (which prints the
*contract*). The bytes are identical to every binding's `*_json()`
output (Python `.nodes_json()`, WASM `.nodes_json()`, the C FFI
`aozora_nodes_json`), so the CLI is a first-class way to get structured
parser output into a shell pipeline.

| `KIND` | Envelope |
|---|---|
| `nodes` | Source-keyed nodes: `{ kind, span }`. |
| `pairs` | Matched delimiter pairs: `{ kind, open, close }`. |
| `container-pairs` | Container open/close pairs (normalized coordinates). |
| `diagnostics` | The diagnostics stream as data (same shape as `check --diagnostic-format json`, but always exit `0`). |
| `gaiji` | Resolved `※［＃…］` references: `{ span, description, mencode, codepoint, resolved }`. Alias: `gaiji-resolutions`. |
| `slugs` | The static `［＃…］` slug catalogue — needs no input. |

Every envelope is `{ "schema_version": 1, "data": [ … ] }`; the per-kind
item schema is the one `aozora schema <kind>` prints (see
[Wire format](../wire/overview.md)). `PATH` of `-` (or omitted) reads
stdin and `--encoding`/`-E` applies; `slugs` ignores any input. Unlike
`check`, `wire` is a pure projection — it always exits `0`.

```sh
aozora wire nodes src.txt                  # source nodes as JSON
cat src.txt | aozora wire pairs            # matched pairs, from stdin
aozora wire gaiji -E sjis crime.txt        # resolved 外字 references
aozora wire slugs                          # the static slug catalogue
aozora schema nodes                        # the *contract* for `wire nodes`
```

## `aozora pandoc`

```text
aozora pandoc [OPTIONS] [PATH]
```

Project the parsed document to a Pandoc AST. Without `--format`/`-t`,
prints Pandoc JSON to stdout (consumable by `pandoc -f json -t …`); with
`--format`, spawns `pandoc` and pipes the JSON through it. Accepts
`--encoding`/`-E`.

```sh
aozora pandoc src.txt | pandoc -f json -t epub3 -o out.epub
aozora pandoc src.txt -t latex > src.tex          # spawns pandoc directly
```

See [Bindings → Pandoc](../bindings/pandoc.md).

## Introspection subcommands

`kinds`, `schema {diagnostics|nodes|pairs|container-pairs}`, and
`explain <target>` print typed contracts and need no input file. They
back the drift-gated wire artefacts; see [Wire format](../wire/overview.md).
The **data** counterpart to `schema` is [`aozora wire`](#aozora-wire),
which projects a parsed document into those same envelopes.

`aozora explain` accepts either a `NodeKind` camelCase tag (printing the
node's handbook chapter) or a **diagnostic code** — the full
`aozora::lex::unclosed_bracket` or the short `unclosed_bracket` —
printing the same severity, help, and docs URL that `aozora check`
attaches to that diagnostic:

```sh
aozora explain ruby                          # NodeKind handbook chapter
aozora explain aozora::lex::unclosed_bracket # diagnostic code → help + URL
aozora explain unresolved_gaiji              # short form of the code
```

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Success. |
| `1` | Diagnostics under `--strict`, or a formatting mismatch under `fmt --check`, or a spawned tool (`pandoc`) exited non-zero. |
| `2` | Usage error (bad flag, unreadable file, decode failure). |
| `3` | An `Internal`-source diagnostic fired during `check` — a library bug. |

## Environment

| Variable | Effect |
|---|---|
| `NO_COLOR` | If set (any value), disable ANSI colour in diagnostics output. |
| `AOZORA_LOG` | `tracing-subscriber` filter (e.g. `aozora_pipeline=debug`). Internal debugging; not part of the stable surface. |

See [Reference → Environment variables](env.md) for the full matrix.

## See also

- [CLI Quickstart](../getting-started/cli.md) — examples and the
  subcommand rationale.
- [Notation overview](../notation/overview.md) — what the parser
  recognises.
- [Diagnostics catalogue](../notation/diagnostics.md) — the codes you'll
  see in `check`'s output and how `--diagnostic-format` renders them.
