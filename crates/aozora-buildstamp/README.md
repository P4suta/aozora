# aozora-buildstamp

The compile-time, channel-aware build-version stamp (`dev` / `nightly`
/ `stable`) baked into the [aozora][repo] binaries — the `aozora` CLI
and `aozora-lsp`. It exposes a single `VERSION` string that folds the
crate version together with the git describe / channel so `--version`
reports exactly what was built.

**Internal build-identity leaf, not a parser API.** It carries no
independent stability contract. To parse Aozora Bunko notation, depend
on the umbrella [`aozora`][crate] crate instead; this crate only exists
so the binaries can stamp their build identity without invalidating the
library crates' caches.

- 📦 [crates.io/crates/aozora][crate]
- 📖 [API reference (docs.rs)][docs]
- 📚 [Handbook][book] — notation reference, architecture, bindings

Part of the [aozora][repo] workspace. Dual-licensed Apache-2.0 OR MIT.

[crate]: https://crates.io/crates/aozora
[docs]: https://docs.rs/aozora
[book]: https://p4suta.github.io/aozora/
[repo]: https://github.com/P4suta/aozora
