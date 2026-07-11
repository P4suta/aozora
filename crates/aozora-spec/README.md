# aozora-spec

The canonical vocabulary shared by every layer of the
[aozora][repo] parser: `Span`, `TriggerKind`, `PairKind`,
`Diagnostic`, the private-use sentinel codepoints, and the `SLUGS`
directive dispatch table.

**Internal implementation crate.** It carries no independent stability
contract — the API can change in any release. Application code should
depend on the umbrella [`aozora`][crate] crate, which re-exports the
supported surface (`Document` + `Tree`, plus the `SLUGS` catalogue and
the `Diagnostic` type).

- 📦 [crates.io/crates/aozora][crate]
- 📖 [API reference (docs.rs)][docs]
- 📚 [Handbook][book] — notation reference, architecture, bindings

Part of the [aozora][repo] workspace. Dual-licensed Apache-2.0 OR MIT.

[crate]: https://crates.io/crates/aozora
[docs]: https://docs.rs/aozora
[book]: https://p4suta.github.io/aozora/
[repo]: https://github.com/P4suta/aozora
