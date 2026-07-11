# aozora-fmt

Idempotent CLI formatter for [aozora-flavored markdown][aozora]
(`.afm` / `.aozora` / `.aozora.txt`).

The formatter is a thin wrapper around the upstream parser:
`Document::parse ∘ Tree::to_source`. Two passes are
guaranteed to produce a byte-identical output (the round-trip
fixed-point invariant is gated by `aozora`'s I3 corpus sweep).

## CLI

```sh
# Read from a file, write the canonical form to stdout
aozora-fmt path/to/doc.afm

# Read from stdin
cat doc.afm | aozora-fmt -

# Verify (exit 1 otherwise — `rustfmt --check` style); recurses directories
aozora-fmt --check path/to/docs/

# Show what would change, in colour
aozora-fmt --check --diff path/to/doc.afm

# List unformatted files (gofmt -l), or emit JSON for tooling
aozora-fmt --list .
aozora-fmt --check --json .

# Rewrite in place (no-op when already canonical); accepts many paths
aozora-fmt --write path/to/doc.afm other.afm
```

Multiple files and directories are accepted; directories are searched
recursively for `.afm` / `.aozora` / `.aozora.txt` sources. See
`aozora-fmt --help` or the
[CLI reference](https://p4suta.github.io/aozora/fmt/cli.html)
for the full surface.

Exit codes: `0` success or check-clean, `1` `--check` would
reformat, `2` any other error.

## Library

The single public entry point is `aozora_fmt::format_source`:

```rust
// Canonicalise an aozora document. The round-trip is a fixed point, so a
// second pass is byte-identical to the first.
let canonical = aozora_fmt::format_source("｜青梅《おうめ》へ");
assert_eq!(canonical, aozora_fmt::format_source(&canonical));
```

`aozora-lsp`'s `textDocument/formatting` handler calls into the
same function so editors and CI gates land on identical output.

## Install

```sh
cargo install --git https://github.com/P4suta/aozora --locked aozora-fmt
```

The same canonical formatting also ships as the `aozora fmt` subcommand of the
main [`aozora`](https://github.com/P4suta/aozora) CLI, whose pre-built binaries
are on [the releases page](https://github.com/P4suta/aozora/releases).

## Repository

Part of the [aozora][repo] workspace. See the
[workspace README][repo] for the full picture and
[`CONTRIBUTING.md`](https://github.com/P4suta/aozora/blob/main/CONTRIBUTING.md)
for the dev loop.

[aozora]: https://github.com/P4suta/aozora
[repo]: https://github.com/P4suta/aozora
