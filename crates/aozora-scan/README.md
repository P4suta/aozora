# aozora-scan

The safe trigger-byte scanner behind the [aozora][repo] lexer:
aho-corasick-backed multi-pattern search that finds every notation
trigger (`｜`, `《`, `※`, `［`, …) in one SIMD-friendly pass, with a
naive fallback for tiny inputs.

**Internal implementation crate.** It carries no independent stability
contract — the API can change in any release. It exists to serve the
[aozora][repo] lexer; application code should depend on the umbrella
[`aozora`][crate] crate instead.

- 📦 [crates.io/crates/aozora][crate]
- 📖 [API reference (docs.rs)][docs]
- 📚 [Handbook][book] — notation reference, architecture, bindings

Part of the [aozora][repo] workspace. Dual-licensed Apache-2.0 OR MIT.

[crate]: https://crates.io/crates/aozora
[docs]: https://docs.rs/aozora
[book]: https://p4suta.github.io/aozora/
[repo]: https://github.com/P4suta/aozora
