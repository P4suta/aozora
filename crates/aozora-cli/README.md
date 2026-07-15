# aozora-cli

Builds `aozora`, the command-line parser for 青空文庫記法 (Aozora Bunko
notation).

## Install

Pre-built binaries are attached to every
[GitHub Release](https://github.com/P4suta/aozora/releases). Or:

```sh
cargo install --git https://github.com/P4suta/aozora --locked aozora-cli
```

## Use

```sh
$ printf '｜青梅《おうめ》へ' | aozora render -
<p><ruby>青梅<rp>(</rp><rt>おうめ</rt><rp>)</rp></ruby>へ</p>
```

Every document subcommand takes `-` for stdin. Encoding is auto-detected
— UTF-8 if the bytes are valid UTF-8, else Shift_JIS — so an Aozora Bunko
file works untouched; force it with `-E {utf8,sjis}` or
`AOZORA_ENCODING`.

`aozora --help` lists the commands, `aozora explain <code>` explains a
diagnostic, and the notation itself is in
[the specification](https://p4suta.github.io/aozora-notation-spec/).

## Exit codes

| | |
|---|---|
| `0` | success, or diagnostics printed and tolerated |
| `1` | a gate tripped, or a runtime error |
| `2` | usage error |
| `3` | an internal-source diagnostic — a parser bug, not bad input |

A reader closing the pipe early (`aozora render … | head`) is exit `0`,
per [ADR-0029](https://github.com/P4suta/aozora/blob/main/docs/adr/0029-broken-pipe-exit-semantics.md).

Part of the [aozora](https://github.com/P4suta/aozora) workspace.
Dual-licensed Apache-2.0 OR MIT.
