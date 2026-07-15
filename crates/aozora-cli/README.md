# aozora-cli

The `aozora` command-line frontend for
[aozora-flavored markdown](https://github.com/P4suta/aozora) — parse
Aozora Bunko notation (青空文庫記法) files: ruby, bouten, 縦中横, 外字
references, kaeriten, indent containers, and page breaks.

This crate builds the `aozora` binary. There is **one parser** behind
every surface, so the HTML, the canonical source, and the diagnostic
stream are byte-identical to the library and the other bindings.

## Install

Pre-built binaries for **Linux x86_64**, **macOS arm64**, and
**Windows x86_64** are attached to every
[GitHub Release](https://github.com/P4suta/aozora/releases) as
`aozora-vX.Y.Z-<target>.{tar.gz,zip}` archives with `SHA256SUMS`.

Or build from source (installs the `aozora` binary):

```sh
cargo install --git https://github.com/P4suta/aozora --locked aozora-cli
```

## Subcommands

| Command | What it does |
|---|---|
| `check` | Lex a file and report diagnostics. |
| `lint` | Report notation-hygiene lints (non-canonical near-misses); `--fix` rewrites them. |
| `fmt` | Round-trip `parse ∘ to_source` to the canonical form (`--check` / `--write` / `--diff`). |
| `render` | Render to semantic HTML5 on stdout. |
| `inspect` | Emit a document's JSON (`nodes` / `pairs` / `container-pairs` / `diagnostics` / `gaiji`). |
| `pandoc` | Project to the Pandoc AST — 50+ output formats via Pandoc writers. |
| `explain` | Prose for a `NodeKind` tag, notation concept, or a diagnostic code. |
| `spec` | Query the tool's own contracts: `kinds` (wire-tag tables) / `schema <which>` (JSON Schema) / `slugs` (static ［＃…］ catalogue). |
| `lsp` | Exec-delegate to the `aozora-lsp` language server (forwards `--stdio` / … verbatim). |
| `completions` | Print a shell completion script (bash / zsh / fish / …). |

All document subcommands accept `-` (or no path) to read from stdin.

## Examples

```sh
# Render to HTML on stdout
$ printf '｜青梅《おうめ》へ' | aozora render -
<p><ruby>青梅<rp>(</rp><rt>おうめ</rt><rp>)</rp></ruby>へ</p>

# Parsed nodes as JSON — byte-identical to every binding's *_json() output
$ printf '｜青梅《おうめ》へ' | aozora inspect nodes -
{"schemaVersion":2,"data":[{"kind":"ruby","span":{"start":0,"end":24}}]}

# CI format gate; reads Shift_JIS source straight from Aozora Bunko
aozora fmt --check FILE.txt
aozora check -E sjis FILE.txt
```

Encoding is auto-detected (UTF-8 if the bytes are valid UTF-8, else
Shift_JIS); force it with `-E {utf8,sjis}` or `AOZORA_ENCODING`.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Success — check-clean, or diagnostics were printed but tolerated. |
| `1` | A gate tripped (`--strict` with a diagnostic, `fmt --check` would reformat) or a runtime error. |
| `2` | Usage error — bad arguments, oversize input, or `--watch` on stdin. |
| `3` | An internal-source diagnostic fired (a parser bug, not bad input). |

A reader that closes the pipe early (`aozora render … | head`) is a
silent success (exit `0`), per [ADR-0029](https://github.com/P4suta/aozora/blob/main/docs/adr/0029-broken-pipe-exit-semantics.md).

## Documentation

See the
[CLI reference chapter](https://p4suta.github.io/aozora/ref/cli.html)
of the handbook for the full subcommand surface, or run
`aozora <cmd> --help`.

## Repository

Part of the [aozora](https://github.com/P4suta/aozora) workspace.
Dual-licensed Apache-2.0 OR MIT.
