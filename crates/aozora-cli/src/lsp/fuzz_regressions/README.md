# Promoted LSP fuzz regressions

Crash inputs that an LSP libFuzzer target once found, lifted here so they
replay on **every** `cargo test` / `just test` run — on the stable
toolchain, with no nightly or cargo-fuzz needed.

Layout: `<target>/<artifact>`, e.g. `edit_pipeline/crash-abc123`.

`fuzz_regressions.rs` walks every `<target>/` subdirectory and replays each
artifact through the same edit + coordinate property the libFuzzer target
asserts (no panic, position round-trip identity).
