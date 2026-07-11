# aozora-pipeline

The lex pipeline behind the [aozora][repo] parser: the
`sanitize → tokenize → pair → classify` lexer plus the `lex`
orchestrator — a pure `fn(&str, &Arena) -> LexOutput<'_>` that produces
the owned, lifetime-free AST.

**Internal implementation crate.** It carries no independent stability
contract — the API can change in any release. Application code should
depend on the umbrella [`aozora`][crate] crate, which drives this
pipeline through `Document::parse` and re-exports it as the `pipeline`
module.

- 📦 [crates.io/crates/aozora][crate]
- 📖 [API reference (docs.rs)][docs]
- 📚 [Handbook][book] — notation reference, architecture, bindings

Part of the [aozora][repo] workspace. Dual-licensed Apache-2.0 OR MIT.

[crate]: https://crates.io/crates/aozora
[docs]: https://docs.rs/aozora
[book]: https://p4suta.github.io/aozora/
[repo]: https://github.com/P4suta/aozora
