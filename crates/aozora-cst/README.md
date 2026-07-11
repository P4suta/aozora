# aozora-cst

A lossless concrete syntax tree (CST) for the [aozora][repo] parser —
a [rowan][rowan]-backed projection of the lexer output that preserves
every byte (whitespace and trivia included), the surface editors and
formatters query.

**Internal implementation crate.** It carries no independent stability
contract — the API can change in any release. Application code should
depend on the umbrella [`aozora`][crate] crate and reach the CST
through its `cst` feature, never on this crate directly.

- 📦 [crates.io/crates/aozora][crate]
- 📖 [API reference (docs.rs)][docs]
- 📚 [Handbook][book] — notation reference, architecture, bindings

Part of the [aozora][repo] workspace. Dual-licensed Apache-2.0 OR MIT.

[crate]: https://crates.io/crates/aozora
[docs]: https://docs.rs/aozora
[book]: https://p4suta.github.io/aozora/
[repo]: https://github.com/P4suta/aozora
[rowan]: https://crates.io/crates/rowan
