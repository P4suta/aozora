# Release profile & PGO

aozora's `[profile.release]` is tuned for cross-crate inlining at
the expense of compile time:

```toml
[profile.release]
lto           = "fat"        # full LTO across the whole workspace
codegen-units = 1            # single CGU so LTO sees everything
strip         = "symbols"    # smaller binary, faster cold start
panic         = "abort"      # no unwinding tables in the binary
opt-level     = 3
```

## Why fat LTO over thin

A thin LTO build keeps each crate's IR isolated; the cross-crate
inliner only inlines through summary stubs. Fat LTO concatenates
every crate's IR into one module before optimisation, so the
inliner can see across the whole pipeline.

For aozora that pays off because the lex pipeline is *deep*:
`aozora-render` → `aozora` → `aozora-lex` → `aozora-lexer` Phase
functions, each in its own crate. A function call across that depth
under thin LTO costs four indirect calls and four stack frames; the
fat LTO build folds the chain into ~40 inlined instructions on the
hot per-byte path.

Measured on the corpus sweep: fat LTO is 30%+ faster than thin LTO
once the lex orchestrator is split across crates. Compile-time cost
is real (release builds take ~3 minutes vs ~1 minute for thin), but
release builds happen at tag time, not on every iteration.

## Why `codegen-units = 1`

`codegen-units = N` splits each crate into N parallel codegen jobs
during compilation. Each unit optimises independently, then the
linker stitches them together. With `N > 1` the LLVM inliner can't
see across unit boundaries inside a single crate — which under fat
LTO defeats half the point.

`codegen-units = 1` ensures fat LTO actually sees every function in
every crate. Compile time grows; runtime wins back.

## Why `panic = "abort"`

aozora is a parser, not a server. There's no panic handler to
recover into — a panic on user input would be a parser bug, not a
recoverable error. `panic = "abort"`:

- Drops the unwinding tables from the binary (~80 KiB savings on
  the CLI).
- Removes the panic-handling overhead from every function call (the
  compiler doesn't insert landing pads).
- Surfaces parser bugs as `SIGABRT` immediately, which is what we
  want — a panic always indicates an invariant violation that needs
  fixing, not a state to gracefully degrade through.

For library consumers that *want* unwinding (e.g. embedding in a
long-running server), the dependency-mode build inherits the
consumer's profile, so this only affects the binaries we publish.

## Profile-guided optimisation (PGO)

The release pipeline supports PGO via `scripts/pgo-build.sh`:

```sh
./scripts/pgo-build.sh
```

Three-stage build:

1. **Instrumented build** — `cargo build --release` with
   `RUSTFLAGS="-Cprofile-generate=/tmp/pgo-data"`. The resulting
   binary is slower than vanilla release because of the
   instrumentation overhead.
2. **Profile collection** — run the corpus sweep against the
   instrumented binary. The corpus must contain a representative
   spread of document sizes and notation density. The
   `aozora-bench` `throughput_by_class` probe handles this.
3. **Final build** — `cargo build --release` with
   `RUSTFLAGS="-Cprofile-use=/tmp/pgo-data/merged.profdata"`. LLVM
   uses the profile to drive its inliner, branch-prediction hints,
   and basic-block ordering decisions.

Measured win on the corpus sweep: 8–12% faster than non-PGO release
build. The cost is operational complexity (the build-script needs a
real corpus available); the win compounds with fat LTO, since both
target the same hot paths.

## BOLT (post-link optimisation)

BOLT is the *next* layer after PGO: it reorders basic blocks in the
final binary based on the same profile. `scripts/pgo-build.sh` ends
with an optional BOLT pass when `llvm-bolt` is on `PATH`.

BOLT wins another ~3% on top of PGO, mostly by improving I-cache
density for the lex hot path. The win is smaller than PGO's because
PGO already used the profile during compilation; BOLT only refines
the final binary's layout.

## Why we *don't* use specific tricks

- **`-Cforce-frame-pointers=yes`** — would help samply unwind on
  some platforms, but the workspace `[profile.bench]` covers the
  profiling case (debug = 1 + strip = none). Release builds get the
  smaller binary.
- **`unsafe` perf shortcuts** — `unsafe_code = "forbid"` at the
  workspace level. Three crates locally relax it (FFI / scan /
  xtask), each with `// SAFETY:` comments and `#[deny(unsafe_op_in_unsafe_fn)]`.
  Where a perf opportunity needs unsafe, we measure it first and
  cite the win in the comment.
- **`#[inline(always)]`** — used sparingly. The compiler's default
  heuristics have improved enough that forcing inlining usually
  costs binary size for negligible win. Where it does help (e.g.
  the per-byte scanner inner loop), the call site has a measurement
  comment.

## See also

- [Profiling with samply](samply.md) — how to *measure* whether
  a perf change helped.
- [Benchmarks](bench.md) — the harness that produces the PGO
  profile.
- [Corpus sweeps](corpus.md) — the input the bench harness consumes.
