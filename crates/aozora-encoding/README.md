# aozora-encoding

Shift_JIS decoding and 外字 (gaiji) resolution for the [aozora][repo]
parser — compile-time PHF lookup tables over JIS X 0213 with UCS
fallback. Aozora Bunko ships its corpus as Shift_JIS; this is the
strict decoder that turns those bytes into UTF-8 the parser can read.

**Internal implementation crate.** It carries no independent stability
contract — the API can change in any release. Application code should
depend on the umbrella [`aozora`][crate] crate and decode through its
`aozora::encoding` module (`decode_sjis`), never on this crate
directly.

- 📦 [crates.io/crates/aozora][crate]
- 📖 [API reference (docs.rs)][docs]
- 📚 [Handbook][book] — notation reference, architecture, bindings

Part of the [aozora][repo] workspace. Dual-licensed Apache-2.0 OR MIT.

[crate]: https://crates.io/crates/aozora
[docs]: https://docs.rs/aozora
[book]: https://p4suta.github.io/aozora/
[repo]: https://github.com/P4suta/aozora
