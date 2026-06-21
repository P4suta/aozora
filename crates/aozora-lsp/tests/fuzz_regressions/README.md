# Promoted fuzz regressions — `aozora-lsp`

Crash inputs that the `aozora-lsp/fuzz` libFuzzer targets once found, lifted
here so they replay on **every** `cargo test` / `just test` run — on the
stable toolchain, with no nightly or cargo-fuzz needed.

Layout: `tests/fuzz_regressions/<target>/<artifact>`, e.g.
`tests/fuzz_regressions/edit_pipeline/crash-abc123`.

Promote an artifact with:

```sh
just fuzz-promote aozora-lsp edit_pipeline crates/aozora-lsp/fuzz/artifacts/edit_pipeline/crash-abc123
```

`tests/fuzz_regressions.rs` walks every `<target>/` subdirectory and replays
each artifact through the same edit + coordinate property the libFuzzer
target asserts (no panic, position round-trip identity).
