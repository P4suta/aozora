# CLI reference

Full reference for the `aozora` binary. For a guided tour, see
[CLI Quickstart](../getting-started/cli.md).

## Synopsis

```text
aozora <SUBCOMMAND> [OPTIONS] [ARGS]
```

| Subcommand | What it does |
|---|---|
| `check` | Lex + report every diagnostic. |
| `lint` | Report notation-hygiene lints (`aozora::lint::*`); `--fix` rewrites in place. |
| `fmt` | Round-trip `parse ∘ to_source` (canonicalise); one or many files. |
| `render` | Render to HTML on stdout. |
| `inspect` | Emit a document's JSON (`nodes`/`pairs`/`container-pairs`/`diagnostics`/`gaiji`). |
| `pandoc` | Project to a Pandoc AST (JSON, or pipe through `pandoc`). |
| `explain` | Print prose for a `NodeKind` tag or notation concept, or help / severity / URL for a diagnostic code. |
| `spec` | Query the tool's own contracts: `kinds` (JSON-tag tables), `schema <which>` (a JSON envelope's JSON Schema), `slugs` (the static ［＃…］ catalogue). |
| `lsp` | Exec-delegate to the `aozora-lsp` language server, forwarding every argument (`--stdio`, …) verbatim. |
| `completions` | Print a shell completion script (bash / zsh / fish / powershell / elvish / nushell). |

The one **global** option is `--color {auto,always,never}` (accepted after
any subcommand). Every other flag below is per-subcommand. All document
subcommands accept `-` (or no path) to read **stdin**.

| Common flag | Subcommands | Effect |
|---|---|---|
| `-E`, `--encoding {auto,utf8,sjis}` | check / lint / fmt / render / inspect / pandoc | Source encoding. **Default `auto`** — UTF-8 if the bytes are valid UTF-8, else Shift_JIS. |
| `--color {auto,always,never}` | global | ANSI colour policy. `auto` honours `NO_COLOR` / `CLICOLOR` / `CLICOLOR_FORCE` and whether the stream is a terminal. |
| `--timing` | check / lint / fmt / render / inspect / pandoc | Print per-phase timing to stderr — aligned `human` lines on a TTY, the `{"schemaVersion":1,"data":{phases,totalNanos}}` envelope when piped; stdout stays byte-identical. |
| `--config PATH` / `--watch` | check / lint / fmt / render / inspect / pandoc | Use a specific `.aozora.toml`; re-run on file change (needs a path). |

There is no `--color` *disable* flag beyond `--color never`; the
`NO_COLOR` environment variable disables colour too.

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
| `--format {auto,human,json,short}` | How to render diagnostics. Default **`auto`**: `human` when stderr is a terminal, `json` when piped. Falls back to `AOZORA_FORMAT`, then the `format` key in `.aozora.toml`. |

The three formats:

- **`human`** — a graphical [`miette`](https://docs.rs/miette/latest/miette/) report:
  the source line, a caret under the span, the label, the help, and a
  link to the [diagnostics catalogue](../notation/diagnostics.md).
- **`json`** — the `aozora::json` diagnostics envelope, byte-identical to
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
aozora check --format short -              # one line per diagnostic, from stdin
cat src.txt | aozora check                 # json envelope (stderr is piped)
```

## `aozora lint`

```text
aozora lint [OPTIONS] [PATH]
```

Report the advisory **notation-hygiene** lints (the `aozora::lint::*`
namespace, currently `non_canonical_directive`) — the authoring-hygiene view,
where `check` reports *every* diagnostic. See
[Notation hygiene](../notation/hygiene.md).

| Option | Effect |
|---|---|
| `--strict`, `-s` | Exit non-zero (`1`) if any lint fired. |
| `--fix` | Rewrite the flagged directive near-misses to their canonical form in place — the zero-false-positive Tier1 autofix. Needs a file (not stdin). Same transform as `fmt --fix --write`. |
| `--format {auto,human,json,short}` | How to render lints (shared with `check`). |
| `--encoding`, `-E` | Source encoding (see above). |

Exit codes mirror `check`: `0` (tolerant default), `1` (`--strict` with a
lint present), `2` (usage error).

```sh
aozora lint src.txt                        # report notation-hygiene lints
aozora lint --strict src.txt               # any lint -> exit 1 (CI gate)
aozora lint --fix src.txt                  # rewrite flagged near-misses in place
```

## `aozora fmt`

```text
aozora fmt [OPTIONS] [PATH]...
```

Round-trip the source through `parse ∘ to_source`. Reads stdin, one file,
many files, or directories (searched for `*.afm`, `*.aozora`, `*.aozora.txt`).
Default prints the canonical form on stdout (single input only).

| Option | Effect |
|---|---|
| `--check` | Exit non-zero if any input differs from its canonical form. |
| `--write`, `-w` | Overwrite each input file in place. Ignored on stdin. |
| `--diff` | Print a unified diff of every file that would change (implies `--check`). |
| `--list`, `-l` | List only the paths that would change (`gofmt -l`). |
| `--json` | Emit the check result as JSON (implies `--check`). |
| `--fix` | Also canonicalise flagged directive near-misses (Tier1). Composes with every mode. |
| `--encoding`, `-E` | Source encoding (see above). |

The standalone `aozora-fmt` binary shares this exact surface and formatting
core, so `aozora fmt` and `aozora-fmt` can never drift.

Exit codes: `0` (success, or no diff under `--check`), `1` (formatting
mismatch under `--check`), `2` (usage error).

```sh
aozora fmt src.txt > formatted.txt
aozora fmt --check src.txt                 # CI gate
aozora fmt --write src.txt                 # in-place
aozora fmt --diff docs/                     # unified diff over a directory
aozora fmt --fix --write src.txt            # canonicalise notation in place
cat src.txt | aozora fmt                    # stdin -> stdout
```

## `aozora render`

```text
aozora render [OPTIONS] [PATH]
```

Render the parsed tree to HTML on stdout. Accepts `--encoding`/`-E`.

| Option | Effect |
|---|---|
| `--normalize` | Render verified Tier1 near-misses as their canonical spelling (e.g. `［＃「梅」は小書き］` as small-letter emphasis instead of an inert span). Read-only; never rewrites source. |
| `--degraded` | Additionally reduce the lossy / judgment **Tier2** forms Tier1 refuses (e.g. `［＃ここから最後まで３字下げ］` as an indent to the document end). Implies `--normalize`, render-only. See [Notation hygiene](../notation/hygiene.md). |

```sh
aozora render src.txt > out.html
aozora render -E sjis crime.txt > crime.html
aozora render --normalize src.txt          # interpret Tier1 near-misses
aozora render --degraded src.txt           # + Tier2 degraded reductions
cat src.txt | aozora render -
```

The output is semantic HTML5 with `aozora-*` class hooks (no inline
styles). See [HTML renderer](../arch/renderer.md#class-name-scheme) for
the class-name reference.

## `aozora inspect`

```text
aozora inspect <KIND> [OPTIONS] [PATH]
```

Emit a parsed document's data as the shared `aozora::json` JSON
envelope — the **data** counterpart to `aozora spec schema` (which prints
the *contract*). The bytes are identical to every binding's `*_json()`
output (Python `.nodes_json()`, WASM `.nodes_json()`, the C FFI
`aozora_nodes_json`), so the CLI is a first-class way to get structured
parser output into a shell pipeline.

| `KIND` | Envelope |
|---|---|
| `nodes` | Source-keyed nodes: `{ kind, span }`. |
| `pairs` | Matched delimiter pairs: `{ kind, open, close }`. |
| `container-pairs` | Container open/close pairs (normalized coordinates). |
| `diagnostics` | The diagnostics stream as data (same shape as `check --format json`, but always exit `0`). |
| `gaiji` | Resolved `※［＃…］` references: `{ span, description, mencode, codepoint, resolved }`. Alias: `gaiji-resolutions`. |

Every envelope is `{ "schemaVersion": 2, "data": [ … ] }`; the per-kind
item schema is the one `aozora spec schema <kind>` prints (see
[JSON output](../json/overview.md)). `PATH` of `-` (or omitted) reads
stdin and `--encoding`/`-E` applies. Unlike `check`, `inspect` is a pure
projection — it always exits `0`. The static `［＃…］` slug catalogue is not
a document view; it lives under [`aozora spec slugs`](#aozora-spec).

```sh
aozora inspect nodes src.txt               # source nodes as JSON
cat src.txt | aozora inspect pairs         # matched pairs, from stdin
aozora inspect gaiji -E sjis crime.txt     # resolved 外字 references
aozora spec schema nodes                   # the *contract* for `inspect nodes`
aozora spec slugs                          # the static slug catalogue
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

## `aozora completions`

```text
aozora completions <SHELL>
```

Print a shell completion script for `<SHELL>` (`bash` / `zsh` / `fish` /
`powershell` / `elvish` / `nushell`) on stdout. The script is generated
from the live command tree, so it always matches the installed binary —
there is no committed copy to drift (ADR-0012). Release tarballs also
bundle these under `completions/`.

```sh
# bash — system-wide (or source the file from ~/.bashrc)
aozora completions bash | sudo tee /etc/bash_completion.d/aozora >/dev/null

# zsh — drop into a directory on your $fpath, then restart the shell
aozora completions zsh > ~/.zfunc/_aozora

# fish
aozora completions fish > ~/.config/fish/completions/aozora.fish

# nushell — save the module, then `use` it from your config
aozora completions nushell | save -f ($nu.default-config-dir | path join aozora.nu)
```

The release archive likewise ships man pages under `man/man1/`
(`aozora.1` plus one page per subcommand), generated the same way. The
hidden `aozora man [SUBCOMMAND]` subcommand renders a page to stdout if
you want to install one locally.

## `aozora spec`

```text
aozora spec <kinds | schema <WHICH> | slugs>
```

Query the tool's own typed contracts — no document input. Groups the
introspection subcommands that print the parser's vocabulary and wire
shapes; they back the drift-gated JSON artefacts (see
[JSON output](../json/overview.md)). The **data** counterpart to
`spec schema` is [`aozora inspect`](#aozora-inspect), which projects a
parsed document into those same envelopes.

| Subcommand | What it prints |
|---|---|
| `spec kinds` | Every `NodeKind` / `PairKind` / `Severity` / `DiagnosticSource` / `Sentinel` / `InternalCheckCode` variant with its wire tag. |
| `spec schema {diagnostics\|nodes\|pairs\|container-pairs}` | The JSON Schema for one of the four `aozora::json` envelopes. |
| `spec slugs` | The static `［＃…］` slug catalogue as the shared `aozora::json` envelope. |

`spec kinds --format` defaults to **`auto`** — human tables when stdout is
a terminal, the machine envelope when piped (`--format {human,json}` forces
either). The envelope is
`{"schemaVersion":1,"data":{"nodeKinds":[{"tag","summary"}],
"pairKinds":[…],"severities":[…],"diagnosticSources":[…],"sentinels":[…],
"internalCheckCodes":[…]}}` (one line, sharing the two-key `{schemaVersion,data}`
shape of the `inspect` envelopes; `schemaVersion` is a CLI-local counter,
distinct from the wire `SCHEMA_VERSION`). `spec schema` and `spec slugs`
emit machine JSON on every stream; only `spec kinds` has a human view.

```sh
aozora spec kinds                # enum / wire-tag tables (json when piped)
aozora spec kinds --format json  # force the machine envelope
aozora spec schema nodes         # the JSON Schema for the `nodes` envelope
aozora spec slugs                # the static ［＃…］ slug catalogue
```

## `aozora explain`

```text
aozora explain <TARGET>
```

`aozora explain` accepts a `NodeKind` camelCase tag (printing the node's
handbook chapter), a **notation concept** — a notation-family key such as
`tcy` or the Japanese name `傍点`, printing a short localized blurb — or a
**diagnostic code** — the full `aozora::lex::unclosed_bracket` or the short
`unclosed_bracket` — printing the same severity, help, and docs URL that
`aozora check` attaches to that diagnostic. Resolution is deterministic in
that order (tag → concept → code). An unrecognised target suggests its
nearest known neighbour ("did you mean …?"):

```sh
aozora explain ruby                          # NodeKind handbook chapter
aozora explain tcy                           # notation concept (縦中横)
aozora explain aozora::lex::unclosed_bracket # diagnostic code → help + URL
aozora explain unresolved_gaiji              # short form of the code
```

The suggestion and concept prose follow `--lang`; the machine axis (which
target resolves, and the exit code) does not.

## `aozora lsp`

```text
aozora lsp [ARGS]...
```

Delegate to the `aozora-lsp` language-server daemon — a git-`<x>`-style
exec-delegate. `aozora lsp` locates the `aozora-lsp` binary (on `PATH`, else
next to the `aozora` executable) and hands the whole process over to it,
forwarding every argument (e.g. `--stdio`) verbatim; on Unix it `exec`s, so no
wrapper lingers for the editor session. The CLI bundles no LSP machinery of its
own — no tokio, no tower-lsp — so an editor can spawn `aozora lsp` and reach the
same server it would by running `aozora-lsp` directly.

Because the arguments pass straight through, `aozora lsp --help` prints the
*daemon's* help, not this shim's — the shim's own summary is `aozora help lsp`.

If `aozora-lsp` is not installed, `aozora lsp` prints an actionable hint and
exits `2` (a usage error — "the server is not installed", distinct from the
generic failure `1` a crash would give). The daemon ships alongside `aozora` in
the release tarball; for other setups, `cargo install --git … aozora-lsp`.

```sh
aozora lsp --stdio                 # what an editor spawns (LSP over stdio)
aozora lsp --help                  # forwarded: prints aozora-lsp's own help
```

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Success. |
| `1` | Diagnostics under `--strict`, or a formatting mismatch under `fmt --check`, or a spawned tool (`pandoc`) exited non-zero. |
| `2` | Usage error (bad flag, unreadable file, decode failure). |
| `3` | An `Internal`-source diagnostic fired during `check` — a library bug. |

A downstream reader that closes stdout early — `aozora render FILE | head` — is
a normal success: the resulting broken pipe is swallowed and the command exits
`0` silently rather than reporting an error ([ADR-0029](https://github.com/P4suta/aozora/blob/main/docs/adr/0029-broken-pipe-exit-semantics.md)).

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
  see in `check`'s output and how `--format` renders them.
