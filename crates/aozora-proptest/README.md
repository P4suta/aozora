# aozora-proptest

Shared [proptest][proptest] strategies (Aozora-shaped input generators
such as `aozora_fragment`, `pathological_aozora`, and
`unicode_adversarial`) for property-testing the [aozora][repo] parser.

**Internal, test-only implementation crate.** It carries no independent
stability contract — the API can change in any release. Application
code should depend on the umbrella [`aozora`][crate] crate and reach
these strategies through its `proptest` feature, never on this crate
directly.

- 📦 [crates.io/crates/aozora][crate]
- 📖 [API reference (docs.rs)][docs]
- 📚 [Handbook][book] — notation reference, architecture, bindings

Part of the [aozora][repo] workspace. Dual-licensed Apache-2.0 OR MIT.

[crate]: https://crates.io/crates/aozora
[docs]: https://docs.rs/aozora
[book]: https://p4suta.github.io/aozora/
[repo]: https://github.com/P4suta/aozora
[proptest]: https://crates.io/crates/proptest
