# Promoted fuzz regressions — `aozora-fmt`

Crash inputs that the `aozora-fmt/fuzz` libFuzzer targets once found, lifted
here so they replay on **every** `cargo test` / `just test` run — on the
stable toolchain, with no nightly or cargo-fuzz needed.

Layout: `tests/fuzz_regressions/<target>/<artifact>`, e.g.
`tests/fuzz_regressions/format_idempotent/crash-abc123`.

Promote an artifact with:

```sh
just fuzz-promote aozora-fmt format_idempotent crates/aozora-fmt/fuzz/artifacts/format_idempotent/crash-abc123
```

`tests/fuzz_regressions.rs` walks every `<target>/` subdirectory and runs
each artifact back through `format_source`, asserting it no longer panics
and stays idempotent.
