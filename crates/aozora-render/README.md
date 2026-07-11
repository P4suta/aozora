# aozora-render

The HTML and canonical-source renderers for the [aozora][repo] AST:
`html::render_to_string` (semantic HTML5) and `serialize::serialize`
(the byte-canonical round-trip form).

**Internal implementation crate.** It carries no independent stability
contract — the API can change in any release. Application code should
depend on the umbrella [`aozora`][crate] crate, which drives these
renderers through `Tree::to_html` / `Tree::to_source` and re-exports
them as the `render` module.

- 📦 [crates.io/crates/aozora][crate]
- 📖 [API reference (docs.rs)][docs]
- 📚 [Handbook][book] — notation reference, architecture, bindings

Part of the [aozora][repo] workspace. Dual-licensed Apache-2.0 OR MIT.

[crate]: https://crates.io/crates/aozora
[docs]: https://docs.rs/aozora
[book]: https://p4suta.github.io/aozora/
[repo]: https://github.com/P4suta/aozora
