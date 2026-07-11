# aozora-syntax

The owned, lifetime-free AST types for the [aozora][repo] parser:
`Node` variants (ruby, bouten, 縦中横, 外字, kaeriten), plus
`ContainerKind`, `BoutenKind`, and `Indent`.

**Internal implementation crate.** It carries no independent stability
contract — the API can change in any release. Application code should
depend on the umbrella [`aozora`][crate] crate, which re-exports these
types through its `syntax` module and hands you a parsed tree via
`Document` + `Tree`.

- 📦 [crates.io/crates/aozora][crate]
- 📖 [API reference (docs.rs)][docs]
- 📚 [Handbook][book] — notation reference, architecture, bindings

Part of the [aozora][repo] workspace. Dual-licensed Apache-2.0 OR MIT.

[crate]: https://crates.io/crates/aozora
[docs]: https://docs.rs/aozora
[book]: https://p4suta.github.io/aozora/
[repo]: https://github.com/P4suta/aozora
