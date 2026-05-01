# API reference (rustdoc)

The full rustdoc surface for every crate in the workspace is
auto-deployed alongside this handbook. Browse it at:

> <https://p4suta.github.io/aozora/api/aozora/>

The landing redirects to the top-level facade (`aozora`); from there
every workspace crate is reachable via the side panel.

## Why `/api/` instead of `docs.rs`?

aozora is not yet on crates.io — public release tracks the v1.0 API
freeze. Until then, `docs.rs` has nothing to render against, so the
rustdoc API reference is built directly from the workspace and
deployed under the GitHub Pages site that serves this handbook.

When the v1.0 release lands and we publish to crates.io, `docs.rs`
will pick up the API reference automatically; the in-tree `/api/`
copy will keep working as a mirror, since the GitHub Pages deploy
runs on every `main` push regardless.

## Layout

| Path | What |
|---|---|
| `/aozora/` (this site) | Handbook (this mdbook) |
| `/aozora/api/aozora/` | Public facade crate |
| `/aozora/api/aozora_lex/` | Lexer orchestrator |
| `/aozora/api/aozora_lexer/` | Seven-phase lexer |
| `/aozora/api/aozora_render/` | HTML / serialise renderers |
| `/aozora/api/aozora_syntax/` | AST node types |
| `/aozora/api/aozora_spec/` | Shared types |
| `/aozora/api/aozora_scan/` | SIMD scanner |
| `/aozora/api/aozora_veb/` | Eytzinger sorted-set |
| `/aozora/api/aozora_encoding/` | SJIS + 外字 |
| `/aozora/api/aozora_cli/` | CLI binary internals |
| `/aozora/api/aozora_ffi/` | C ABI driver |
| `/aozora/api/aozora_wasm/` | WASM driver |
| `/aozora/api/aozora_py/` | Python binding |
| `/aozora/api/aozora_bench/` | Bench probes |
| `/aozora/api/aozora_corpus/` | Corpus runner |
| `/aozora/api/aozora_proptest/` | Proptest strategies |
| `/aozora/api/aozora_trace/` | Samply trace loader |
| `/aozora/api/aozora_xtask/` | Dev tooling |

## Doc-link discipline

The workspace `[workspace.lints.rustdoc]` block sets every
documentation lint to `warn` (target: deny). Specifically:

- `broken_intra_doc_links = "warn"` — every `[name]` link in a doc
  comment must resolve.
- `private_intra_doc_links = "warn"` — links to `pub(crate)` items
  flagged so the public docs don't dangle into private structures.
- `invalid_codeblock_attributes = "warn"` — typos in
  ` ```rust,no_run ` style attributes get caught.
- `invalid_html_tags = "warn"` — accidental `<foo>` in prose flagged.
- `invalid_rust_codeblocks = "warn"` — every ` ```rust ` block must
  parse as Rust.
- `bare_urls = "warn"` — links must be `<https://...>` or `[label](url)`,
  not bare URLs (which markdown parses inconsistently).
- `redundant_explicit_links = "warn"` — `[x](x)` where the autolink
  form would do.
- `unescaped_backticks = "warn"` — stray backticks flagged.

The deferred deny upgrade is tracked separately; once the existing
warnings are cleaned up the gate will tighten.

## Local rustdoc build

```sh
just doc                        # workspace-wide rustdoc (no deps)
just doc-open                   # rustdoc + open in default browser
```

Both run inside the dev container; output lands at
`target/doc/aozora/index.html`.

## Building this handbook

```sh
just book-build                 # render to crates/aozora-book/book/
just book-serve                 # live-preview at localhost:3000
just book-linkcheck             # lychee link verification
```

See [Contributing → Development loop](../contrib/dev.md) for the
full toolchain.

## See also

- [Crate map](../arch/crates.md) — narrative description of each
  crate.
- [Library Quickstart](../getting-started/library.md) — common API
  patterns.
