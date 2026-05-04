# API reference (rustdoc)

The full rustdoc surface for every crate in the workspace is
auto-deployed alongside this handbook. Browse it at:

> <https://p4suta.github.io/aozora/api/aozora/>

The landing redirects to the top-level facade (`aozora`); from there
every workspace crate is reachable via the side panel.

## Why `/api/` instead of `docs.rs`?

aozora is not yet published on crates.io — public release tracks the
pre-1.0 stabilisation work and we publish on a deliberate cadence
once the public surface settles. Until that happens, `docs.rs` has
nothing to render against, so the rustdoc API reference is built
directly from the workspace and deployed under the GitHub Pages site
that serves this handbook.

When the first crates.io release lands, `docs.rs` will pick up the
API reference automatically; the in-tree `/api/` copy will keep
working as a mirror, since the GitHub Pages deploy runs on every
`main` push regardless.

## Layout

| Path | What |
|---|---|
| `/aozora/` (this site) | Handbook (this mdbook) |
| `/aozora/api/aozora/` | Public facade crate |
| `/aozora/api/aozora_pipeline/` | Four-phase lexer + `lex_into_arena` orchestrator |
| `/aozora/api/aozora_render/` | HTML / serialise renderers |
| `/aozora/api/aozora_syntax/` | AST node types |
| `/aozora/api/aozora_spec/` | Shared types + `SLUGS` dispatch table |
| `/aozora/api/aozora_scan/` | SIMD trigger scanner |
| `/aozora/api/aozora_veb/` | Eytzinger sorted-set |
| `/aozora/api/aozora_encoding/` | SJIS + 外字 |
| `/aozora/api/aozora_cst/` | rowan-backed lossless CST |
| `/aozora/api/aozora_query/` | tree-sitter-flavoured pattern DSL |
| `/aozora/api/aozora_pandoc/` | Pandoc AST projection |
| `/aozora/api/aozora_cli/` | CLI binary internals |
| `/aozora/api/aozora_ffi/` | C ABI driver |
| `/aozora/api/aozora_wasm/` | WASM driver |
| `/aozora/api/aozora_py/` | Python binding |
| `/aozora/api/aozora_bench/` | Bench probes |
| `/aozora/api/aozora_conformance/` | Conformance fixture runner |
| `/aozora/api/aozora_corpus/` | Corpus runner |
| `/aozora/api/aozora_proptest/` | Proptest strategies |
| `/aozora/api/aozora_trace/` | Samply trace loader |
| `/aozora/api/aozora_xtask/` | Dev tooling |

## Doc-link discipline

The workspace `[workspace.lints.rustdoc]` block denies every
documentation lint:

- `broken_intra_doc_links = "deny"` — every `[name]` link in a doc
  comment must resolve.
- `private_intra_doc_links = "deny"` — links to `pub(crate)` items
  flagged so the public docs don't dangle into private structures.
- `invalid_codeblock_attributes = "deny"` — typos in
  ` ```rust,no_run ` style attributes get caught.
- `invalid_html_tags = "deny"` — accidental `<foo>` in prose flagged.
- `invalid_rust_codeblocks = "deny"` — every ` ```rust ` block must
  parse as Rust.
- `bare_urls = "deny"` — links must be `<https://...>` or `[label](url)`,
  not bare URLs (which markdown parses inconsistently).
- `redundant_explicit_links = "deny"` — `[x](x)` where the autolink
  form would do.
- `unescaped_backticks = "deny"` — stray backticks flagged.

Every workspace-internal `pub` item that lands in rustdoc is
verified by `cargo doc --workspace --no-deps` running with
`RUSTDOCFLAGS=-D warnings`.

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
