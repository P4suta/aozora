# Fuzz workflow

Three cargo-fuzz harnesses live in this workspace:

| Crate | Target | What it asserts |
|-------|--------|-----------------|
| `aozora-pipeline` | `lex_into_arena` | `lex_into_arena` is panic-free, normalized text stays valid UTF-8, every diagnostic span is in-bounds |
| `aozora-render` | `render_html` | `lex_into_arena` → `render_to_string` is panic-free and never leaks PUA sentinels (U+E001..U+E004) into the rendered HTML |
| `aozora-render` | `serialize_round_trip` | I3 fixed-point invariant: `serialize(serialize(x))` byte-equals `serialize(x)` |
| `aozora-encoding` | `decode_sjis` | `decode_sjis` is panic-free on adversarial bytes and returns valid UTF-8 on success |

Each crate keeps its harness binaries under `crates/<crate>/fuzz/`
(an out-of-workspace nightly-only sub-crate, mirroring the
[afm](https://github.com/P4suta/afm) layout).

## Workflow

```sh
# 60-second smoke fuzz of one target.
just fuzz-quick aozora-pipeline lex_into_arena

# 5-minute deep fuzz — release pre-flight gate.
just fuzz-deep aozora-render render_html

# 15-minute marathon — strongest single-target soak.
just fuzz-marathon aozora-render serialize_round_trip

# Sweep every registered target in turn.
just fuzz-all-quick      # 60 s × 4 targets
just fuzz-all-deep       # 5 min × 4 targets

# At-a-glance health: pending crashes vs pinned regressions per target.
just fuzz-status

# Replay every artifact under fuzz/artifacts/<target>/ and print just
# the panic block — tier label + src + html excerpt + violation —
# filtered out of libFuzzer's stack-trace noise. Exit status is the
# count of still-crashing artifacts so it can drive a CI gate.
just fuzz-triage aozora-pipeline lex_into_arena

# Lift a triaged artifact into the permanent regression set so
# `tests/fuzz_regressions.rs` replays it on every `just test`.
just fuzz-promote aozora-pipeline lex_into_arena crash-<sha>
```

When libFuzzer flags a crash it writes the offending input to
`crates/<crate>/fuzz/artifacts/<target>/crash-<sha>` and exits
non-zero. Triage to see the panic, fix the underlying bug (or, if
the fuzz target's invariant was over-strict, fix the assertion),
then promote. From the next `just test` run onwards, the artifact
replays under `tests/fuzz_regressions.rs` — no nightly required for
the permanent regression test.

## Diagnostic philosophy

Fuzz targets `panic!()` with a self-contained message that includes
the input bytes and the relevant slice of output. You should never
need a stack-trace pass to know what failed: the `panicked at` line
plus the next two or three lines are enough to reproduce the failure
anywhere.

`just fuzz-triage` extracts exactly that block out of libFuzzer's
verbose preamble + stack trace, so a triage call always reads
linearly.
