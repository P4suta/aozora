# aozora-ffi

A C ABI driver for the aozora parser — an opaque handle plus JSON
output.

Not published to crates.io, so there is no `cargo add` and no docs.rs
page. Build it:

```sh
cargo build --release -p aozora-ffi
```

That produces the cdylib and staticlib, plus a cbindgen-generated
`aozora.h` mirrored to `target/<profile>/aozora.h`.

- [C ABI reference](https://p4suta.github.io/aozora/api/aozora_ffi/index.html)
- [A reference consumer](https://github.com/P4suta/aozora/blob/main/crates/aozora-ffi/tests/c_smoke/smoke.c)

Part of the [aozora](https://github.com/P4suta/aozora) workspace.
Dual-licensed Apache-2.0 OR MIT.
