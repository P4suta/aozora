set shell := ["bash", "-euo", "pipefail", "-c"]
set dotenv-load := false

_FUZZ_TOOLCHAIN := "nightly-2026-07-15"

[doc("List available recipes.")]
default:
    @just --list --unsorted

[doc("Install the locked development tools and hooks, then run tests.")]
setup:
    #!/usr/bin/env bash
    set -euo pipefail
    command -v mise >/dev/null || { echo "❌ mise not found — install it from https://mise.jdx.dev/"; exit 1; }
    scripts/check-native-prerequisites.sh
    export MISE_IGNORED_CONFIG_PATHS="${XDG_CONFIG_HOME:-$HOME/.config}/mise/config.toml"
    mapfile -t tools < <(mise config get -f mise.toml tools \
      | sed '/^\[\[/,$d' \
      | sed -nE 's/^"?([^" =]+)"?[[:space:]]*=.*/\1/p')
    tools+=(rust@1.99.1 rust@nightly-2026-07-15)
    mise install --locked "${tools[@]}"
    stable_toolchain=$(awk -F'"' '/^channel/ { print $2; exit }' rust-toolchain.toml)
    rustup component add --toolchain "$stable_toolchain" clippy rustfmt rust-src llvm-tools-preview
    rustup target add --toolchain "$stable_toolchain" wasm32-unknown-unknown wasm32-wasip1
    rustup component add --toolchain "{{_FUZZ_TOOLCHAIN}}" rust-src miri
    echo "▶ [1/2] Installing git hooks…"
    just hooks
    echo "▶ [2/2] Verifying the toolchain…"
    just test
    echo "✅ Setup complete. Try 'just --list'."

[doc("Diagnose the native development environment without changing it.")]
doctor:
    #!/usr/bin/env bash
    set -uo pipefail
    fail=0; warn=0
    export MISE_IGNORED_CONFIG_PATHS="${XDG_CONFIG_HOME:-$HOME/.config}/mise/config.toml"
    ok()   { echo "✅ $1"; }
    bad()  { echo "❌ $1"; echo "      ↳ $2"; fail=$((fail + 1)); }
    note() { echo "⚠️  $1"; echo "      ↳ $2"; warn=$((warn + 1)); }

    command -v mise >/dev/null \
      && ok "mise installed" \
      || bad "mise not found" "install mise from https://mise.jdx.dev/"
    mapfile -t tools < <(mise config get -f mise.toml tools \
      | sed '/^\[\[/,$d' \
      | sed -nE 's/^"?([^" =]+)"?[[:space:]]*=.*/\1/p')
    tools+=(rust@1.99.1 rust@nightly-2026-07-15)
    mise install --locked --dry-run "${tools[@]}" >/dev/null 2>&1 \
      && ok "mise.lock matches mise.toml" \
      || bad "mise.lock is stale or missing" "run 'mise lock' and commit the result"
    for tool in cargo rustc just lefthook typos committed actionlint taplo bun node go quicktype wasm-opt; do
      command -v "$tool" >/dev/null \
        && ok "tool: $tool" \
        || bad "tool missing: $tool" "run 'mise install --locked'"
    done
    if [ -f .git/hooks/pre-commit ] && grep -q lefthook .git/hooks/pre-commit 2>/dev/null; then
      ok "git hooks installed (lefthook)"
    elif [ -n "$(git config --get core.hooksPath 2>/dev/null)" ]; then
      ok "git hooks via core.hooksPath ($(git config --get core.hooksPath))"
    else
      bad "git hooks not installed" "run 'just hooks'"
    fi
    { [ "$(git config --get commit.gpgsign 2>/dev/null)" = "true" ] \
        && [ -n "$(git config --get user.signingkey 2>/dev/null)" ]; } \
      && ok "commit signing configured" \
      || note "commit signing not configured" "commits must be signed — see CONTRIBUTING.md"
    [ -f rust-toolchain.toml ] \
      && ok "rust-toolchain.toml present" \
      || bad "rust-toolchain.toml missing" "restore the repository toolchain declaration"
    stable_toolchain=$(awk -F'"' '/^channel/ { print $2; exit }' rust-toolchain.toml)
    installed_components=$(rustup component list --toolchain "$stable_toolchain" --installed 2>/dev/null)
    grep -q '^clippy-' <<<"$installed_components" \
      && ok "Rust component: clippy" \
      || bad "Rust component missing: clippy" "run 'just setup'"
    grep -q '^rustfmt-' <<<"$installed_components" \
      && ok "Rust component: rustfmt" \
      || bad "Rust component missing: rustfmt" "run 'just setup'"
    grep -qx 'rust-src' <<<"$installed_components" \
      && ok "Rust component: rust-src" \
      || bad "Rust component missing: rust-src" "run 'just setup'"
    grep -q '^llvm-tools-' <<<"$installed_components" \
      && ok "Rust component: llvm-tools-preview" \
      || bad "Rust component missing: llvm-tools-preview" "run 'just setup'"
    for target in wasm32-unknown-unknown wasm32-wasip1; do
      rustup target list --toolchain "$stable_toolchain" --installed 2>/dev/null | grep -qx "$target" \
        && ok "Rust target: $target" \
        || bad "Rust target missing: $target" "run 'just setup'"
    done
    rustup component list --toolchain "{{_FUZZ_TOOLCHAIN}}" --installed 2>/dev/null | grep -q '^miri-' \
      && ok "Rust component: miri ({{_FUZZ_TOOLCHAIN}})" \
      || bad "Rust component missing: miri ({{_FUZZ_TOOLCHAIN}})" "run 'just setup'"
    { [ -n "${AOZORA_CORPUS_ROOT:-}" ] && [ -d "${AOZORA_CORPUS_ROOT:-}" ]; } \
      && ok "AOZORA_CORPUS_ROOT set ($AOZORA_CORPUS_ROOT)" \
      || note "AOZORA_CORPUS_ROOT unset (corpus sweeps skip)" "export AOZORA_CORPUS_ROOT=\$HOME/aozora-corpus (optional)"
    avail_kb=$(df -Pk . | awk 'NR==2{print $4}')
    [ "${avail_kb:-0}" -ge 5242880 ] \
      && ok "disk headroom (>= 5 GB free)" \
      || note "less than ~5 GB free here" "Cargo, Bun, and browser artifacts need headroom"
    if native_output=$(scripts/check-native-prerequisites.sh 2>&1); then
      ok "native prerequisites: Clang/libclang and Valgrind"
    else
      printf '%s\n' "$native_output"
      bad "native prerequisites unavailable" "install the packages listed above"
    fi
    if [ -r /proc/sys/kernel/perf_event_paranoid ]; then
      lvl=$(cat /proc/sys/kernel/perf_event_paranoid)
      [ "${lvl:-9}" -le 1 ] \
        && ok "perf_event_paranoid=$lvl (samply profiling ready)" \
        || note "perf_event_paranoid=$lvl (samply needs <= 1)" "echo 1 | sudo tee /proc/sys/kernel/perf_event_paranoid"
    fi

    echo "────"
    if [ "$fail" -gt 0 ]; then
      echo "❌ $fail blocking issue(s), $warn warning(s) — fix the ❌ above (often 'just setup')."
      exit 1
    fi
    echo "✅ no blocking issues ($warn warning(s))."

[doc("Check Clang, libclang, and Valgrind availability.")]
native-prerequisites:
    scripts/check-native-prerequisites.sh

# Bench-only dependencies are excluded to keep routine builds fast (ADR-0004).
_ws := "--workspace --exclude aozora-bench"

[doc("Build workspace targets except the benchmark harness.")]
build:
    cargo build {{_ws}} --all-targets

[doc("Type-check workspace targets except the benchmark harness.")]
check:
    cargo check {{_ws}} --all-targets

[doc("Build workspace release binaries except the benchmark harness.")]
build-release:
    cargo build --release {{_ws}}

[doc("Run the aozora CLI with the supplied arguments.")]
run *ARGS:
    cargo run --package aozora-cli --quiet -- {{ARGS}}

[doc("Run a library example by name.")]
example NAME *ARGS:
    cargo run -p aozora --example {{NAME}} {{ARGS}}

[doc("Run unit, integration, and snapshot tests with nextest.")]
test *ARGS:
    cargo nextest run {{_ws}} --all-targets {{ARGS}}

[doc("Run tests matching a nextest filter expression.")]
t FILTER *ARGS:
    cargo nextest run {{_ws}} -E 'test({{FILTER}})' {{ARGS}}

[doc("Run workspace doctests.")]
test-doc:
    cargo test --workspace --doc

[doc("Run aozora doctests with all optional features.")]
test-doc-all:
    cargo test -p aozora --doc --all-features

[doc("Refresh snapshots with all optional features enabled.")]
snapshot-update:
    env INSTA_UPDATE=always cargo nextest run {{_ws}} --all-features --all-targets

[doc("Check rendered output against conformance goldens.")]
render-gate:
    cargo test -p aozora-conformance --test render_gate

[doc("Refresh conformance render goldens.")]
render-gate-update:
    env UPDATE_GOLDEN=1 cargo test -p aozora-conformance --test render_gate

[doc("Regenerate wire JSON schemas.")]
schema:
    cargo run -p aozora-xtask -q -- schema dump

[doc("Check committed wire JSON schemas for drift.")]
schema-check:
    cargo run -p aozora-xtask -q -- schema check

[doc("Regenerate TypeScript wire types.")]
types:
    cargo run -p aozora-xtask -q -- types ts

[doc("Check committed TypeScript wire types for drift.")]
types-check:
    cargo run -p aozora-xtask -q -- types check

[doc("Regenerate host SDK wire types from JSON schemas.")]
types-langs:
    cargo run -p aozora-xtask -q -- types langs

[doc("Check committed host SDK wire types for drift.")]
types-langs-check:
    cargo run -p aozora-xtask -q -- types langs-check

[doc("Regenerate the tree-sitter parser.")]
grammar:
    cargo run -p aozora-xtask -q -- conformance grammar --update

[doc("Check the committed tree-sitter parser for drift.")]
grammar-check:
    cargo run -p aozora-xtask -q -- conformance grammar --check

[doc("Check publishable manifests and changelog coverage offline.")]
publish-check:
    cargo run -p aozora-xtask -q -- publish check

[doc("Build and verify local crate archives without publishing.")]
artifact-crates *ARGS:
    cargo run -p aozora-xtask -q -- artifacts crates {{ARGS}}

[doc("Verify local and remote release preconditions.")]
rearm-preflight *ARGS:
    cargo run -p aozora-xtask -q -- release preflight {{ARGS}}

[doc("Dispatch PyPI and npm publishing rehearsals.")]
rearm-rehearse *ARGS:
    cargo run -p aozora-xtask -q -- release rehearse {{ARGS}}

[doc("Check MSRV policy and toolchain pin consistency.")]
msrv-check:
    cargo run -p aozora-xtask -q -- msrv check

[doc("Check workspace builds on the declared MSRV.")]
msrv-local:
    #!/usr/bin/env bash
    set -euo pipefail
    v=$(awk -F'"' '/^rust-version/ { print $2; exit }' Cargo.toml)
    rustup toolchain install "$v" --profile minimal
    # Extism host-smoke dependencies declare an MSRV above the public library contract (ADR-0034).
    cargo "+$v" check --locked --workspace --all-targets --all-features --exclude aozora-extism
    cargo "+$v" check --locked -p aozora-extism --all-targets
    echo "msrv-local: workspace builds on $v"

[doc("Check generated artifacts and release contracts for drift.")]
drift-gate:
    cargo run -p aozora-xtask -q -- schema check
    cargo run -p aozora-xtask -q -- types check
    cargo run -p aozora-xtask -q -- types langs-check
    cargo run -p aozora-xtask -q -- conformance grammar --check
    cargo run -p aozora-xtask -q -- publish check
    cargo run -p aozora-xtask -q -- release check
    cargo run -p aozora-xtask -q -- msrv check
    cargo run -p aozora-xtask -q -- lint coordinates

[doc("Create an ADR from the template using the supplied title.")]
new-adr TITLE:
    #!/usr/bin/env bash
    set -euo pipefail
    last=$(ls docs/adr/ | grep -oE '^[0-9]{4}' | sort -n | tail -1)
    n=$(printf '%04d' $((10#$last + 1)))
    slug=$(printf '%s' "{{TITLE}}" | tr '[:upper:] ' '[:lower:]-' | tr -cd 'a-z0-9-')
    f="docs/adr/${n}-${slug}.md"
    [[ -e "$f" ]] && { echo "$f already exists" >&2; exit 1; }
    cp docs/adr/0000-template.md "$f"
    sed -i -e "s/^# NNNN. TITLE_HERE/# ${n}. {{TITLE}}/" -e "s/YYYY-MM-DD/$(date +%F)/" "$f"
    echo "Created $f"

[doc("Run parser, tree-sitter, and real-work conformance gates.")]
conformance:
    bash -c 'set -euo pipefail; cargo run -p aozora-xtask -q -- conformance run && cargo run -p aozora-xtask -q -- conformance vectors && cargo run -p aozora-xtask -q -- conformance run --implementation tree-sitter && cargo run -p aozora-xtask -q -- conformance vectors --implementation tree-sitter && cargo test -p aozora-conformance --test works_gate && cargo run -p aozora-xtask -q -- corpus family-coverage'

[doc("Sync vendored vectors from the specification repository.")]
sync-spec-vectors:
    #!/usr/bin/env bash
    set -euo pipefail
    spec="${AOZORA_SPEC_REPO:-$PWD/../aozora-notation-spec}"
    [[ -d "$spec" ]] || { echo "spec repository not found: $spec" >&2; exit 1; }
    AOZORA_SPEC_REPO="$spec" cargo run -q --release -p aozora-xtask -- spec-vectors sync

[doc("Check vendored vectors against the configured specification repository.")]
verify-spec-vectors:
    #!/usr/bin/env bash
    set -euo pipefail
    spec="${AOZORA_SPEC_REPO:-$PWD/../aozora-notation-spec}"
    if [[ ! -d "$spec" ]]; then
        echo "spec repository not configured; vendored vectors remain covered by conformance"
        exit 0
    fi
    AOZORA_SPEC_REPO="$spec" cargo run -q --release -p aozora-xtask -- spec-vectors check

[doc("Run property tests with the default case budget.")]
prop:
    cargo nextest run --workspace --all-features --test 'property_*' --run-ignored default

[doc("Run property tests with 4096 cases per block.")]
prop-deep:
    bash -c 'AOZORA_PROPTEST_CASES=4096 cargo nextest run --workspace --all-features --test "property_*" --run-ignored default'

[doc("Check parsing, round trips, source tiling, and edits over the corpus.")]
corpus-sweep:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${AOZORA_CORPUS_ROOT:-}" ]]; then
        echo "AOZORA_CORPUS_ROOT is not set; sweep has nothing to walk."
        echo "Set it to a directory of aozora-format .txt files, e.g.:"
        echo "  export AOZORA_CORPUS_ROOT=\$HOME/aozora-corpus"
        echo "Then re-run 'just corpus-sweep'."
        exit 0
    fi
    if [[ ! -d "$AOZORA_CORPUS_ROOT" ]]; then
        echo "AOZORA_CORPUS_ROOT=$AOZORA_CORPUS_ROOT is not a directory." >&2
        exit 1
    fi
    AOZORA_REQUIRE_CORPUS=1 cargo nextest run --release --package aozora --test corpus_sweep --test corpus_document_edits --no-capture

[doc("Check incremental edit performance against corpus speedup thresholds.")]
incremental-speedup-gate:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${AOZORA_CORPUS_ROOT:-}" ]] || [[ ! -d "$AOZORA_CORPUS_ROOT" ]]; then
        echo "AOZORA_CORPUS_ROOT must name a readable corpus directory." >&2
        exit 1
    fi
    AOZORA_INCREMENTAL_MIN_SPEEDUP=1.10 \
      AOZORA_INCREMENTAL_MAX_SLOWDOWN=1.50 \
      cargo run -q --release -p aozora-bench --example incremental_speedup

[doc("Run the strict parser gate over the configured corpus.")]
audit-gate:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${AOZORA_CORPUS_ROOT:-}" ]]; then
        echo "AOZORA_CORPUS_ROOT is not set; audit-gate skipped (no corpus to walk)."
        exit 0
    fi
    if [[ ! -d "$AOZORA_CORPUS_ROOT" ]]; then
        echo "AOZORA_CORPUS_ROOT=$AOZORA_CORPUS_ROOT is not a directory." >&2
        exit 1
    fi
    cargo run -p aozora-xtask -q -- corpus audit-gate --root "$AOZORA_CORPUS_ROOT"

[doc("Check verbatim reconstruction against sanitized corpus sources.")]
verbatim-gate:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${AOZORA_CORPUS_ROOT:-}" ]]; then
        echo "AOZORA_CORPUS_ROOT is not set; verbatim-gate skipped (no corpus to walk)."
        exit 0
    fi
    if [[ ! -d "$AOZORA_CORPUS_ROOT" ]]; then
        echo "AOZORA_CORPUS_ROOT=$AOZORA_CORPUS_ROOT is not a directory." >&2
        exit 1
    fi
    cargo run --release -p aozora-xtask -q -- corpus verbatim --root "$AOZORA_CORPUS_ROOT"

[doc("Check corpus renders for unexplained visible notation.")]
render-leak-gate:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${AOZORA_CORPUS_ROOT:-}" ]]; then
        echo "AOZORA_CORPUS_ROOT is not set; render-leak-gate skipped (no corpus to walk)."
        exit 0
    fi
    if [[ ! -d "$AOZORA_CORPUS_ROOT" ]]; then
        echo "AOZORA_CORPUS_ROOT=$AOZORA_CORPUS_ROOT is not a directory." >&2
        exit 1
    fi
    cargo run --release -p aozora-xtask -q -- corpus render-leak-gate --root "$AOZORA_CORPUS_ROOT"

[doc("Check corpus renders for structural correctness.")]
render-correctness-gate:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${AOZORA_CORPUS_ROOT:-}" ]]; then
        echo "AOZORA_CORPUS_ROOT is not set; render-correctness-gate skipped (no corpus to walk)."
        exit 0
    fi
    if [[ ! -d "$AOZORA_CORPUS_ROOT" ]]; then
        echo "AOZORA_CORPUS_ROOT=$AOZORA_CORPUS_ROOT is not a directory." >&2
        exit 1
    fi
    cargo run --release -p aozora-xtask -q -- corpus render-correctness-gate --root "$AOZORA_CORPUS_ROOT"

[doc("Select representative corpus works under a source-byte budget.")]
works-select:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${AOZORA_CORPUS_ROOT:-}" ]]; then
        echo "AOZORA_CORPUS_ROOT is not set; cannot select works." >&2
        exit 1
    fi
    cargo run -p aozora-xtask -q -- corpus select-works --root "$AOZORA_CORPUS_ROOT"

[doc("Vendor selected corpus works and seed their render goldens.")]
works-vendor:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${AOZORA_CORPUS_ROOT:-}" ]]; then
        echo "AOZORA_CORPUS_ROOT is not set; cannot vendor works." >&2
        exit 1
    fi
    cargo run -p aozora-xtask -q -- corpus vendor-works --root "$AOZORA_CORPUS_ROOT"
    UPDATE_GOLDEN=1 cargo test -p aozora-conformance --test works_gate

[doc("Check allocation pressure against the corpus baseline.")]
alloc-gate: baseline-ratchet
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${AOZORA_CORPUS_ROOT:-}" ]]; then
        echo "AOZORA_CORPUS_ROOT is not set; alloc-gate skipped (no corpus to walk)."
        exit 0
    fi
    if [[ ! -d "$AOZORA_CORPUS_ROOT" ]]; then
        echo "AOZORA_CORPUS_ROOT=$AOZORA_CORPUS_ROOT is not a directory." >&2
        exit 1
    fi
    cargo run --release -p aozora-bench --example alloc_gate -- --root "$AOZORA_CORPUS_ROOT" --baseline corpus/alloc-baseline.json

[doc("Refresh the corpus allocation baseline.")]
alloc-gate-update:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${AOZORA_CORPUS_ROOT:-}" ]]; then
        echo "AOZORA_CORPUS_ROOT is not set; cannot capture a baseline." >&2
        exit 1
    fi
    cargo run --release -p aozora-bench --example alloc_gate -- --root "$AOZORA_CORPUS_ROOT" --baseline corpus/alloc-baseline.json --update

[doc("Measure corpus lexer throughput by document size.")]
throughput:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${AOZORA_CORPUS_ROOT:-}" ]]; then
        echo "AOZORA_CORPUS_ROOT is not set; throughput skipped (no corpus to walk)."
        exit 0
    fi
    cargo run --release -p aozora-bench --example throughput

[doc("Reject increases in committed performance baselines.")]
baseline-ratchet:
    cargo run -p aozora-xtask -q -- ratchet

[doc("Check instruction counts against the performance baseline.")]
perf-gate: baseline-ratchet
    cargo run -p aozora-xtask -q -- perf check

[doc("Build distribution artifacts and check their size budgets.")]
artifact-size-gate: baseline-ratchet extism-build
    env CARGO_PROFILE_RELEASE_OPT_LEVEL=z wasm-pack build --target web --release crates/aozora-wasm --locked
    sh -c 'set -eu; wasm=crates/aozora-wasm/pkg/aozora_wasm_bg.wasm; \
        before=$(wc -c < "$wasm"); \
        wasm-opt -Oz --strip-debug --strip-dwarf --vacuum \
            --enable-bulk-memory --enable-mutable-globals \
            --enable-nontrapping-float-to-int "$wasm" -o "$wasm"; \
        after=$(wc -c < "$wasm"); test "$after" -lt "$before"'
    bash -euc 'cargo build --locked --profile dist -p aozora-cli; \
        mkdir -p target/release-ready-build; \
        cp target/dist/aozora target/release-ready-build/'
    cargo run -p aozora-xtask -q -- artifacts size-check

_fuzz_target := "x86_64-unknown-linux-gnu"

[doc("Run a fuzz package with caller-supplied target and arguments.")]
fuzz CRATE *ARGS:
    bash -c 'cd crates/aozora/fuzz/{{CRATE}} && cargo +{{_FUZZ_TOOLCHAIN}} fuzz run --fuzz-dir . {{ARGS}}'

[doc("Fuzz one target for 60 seconds.")]
fuzz-quick CRATE TARGET:
    bash -c 'cd crates/aozora/fuzz/{{CRATE}} && cargo +{{_FUZZ_TOOLCHAIN}} fuzz run --fuzz-dir . --target {{_fuzz_target}} {{TARGET}} -- -max_total_time=60'

[doc("Fuzz one target for 5 minutes.")]
fuzz-deep CRATE TARGET:
    bash -c 'cd crates/aozora/fuzz/{{CRATE}} && cargo +{{_FUZZ_TOOLCHAIN}} fuzz run --fuzz-dir . --target {{_fuzz_target}} {{TARGET}} -- -max_total_time=300'

[doc("Fuzz one target for 15 minutes.")]
fuzz-marathon CRATE TARGET:
    bash -c 'cd crates/aozora/fuzz/{{CRATE}} && cargo +{{_FUZZ_TOOLCHAIN}} fuzz run --fuzz-dir . --target {{_fuzz_target}} {{TARGET}} -- -max_total_time=900'

[doc("Fuzz every registered target for 60 seconds each.")]
fuzz-all-quick:
    just fuzz-quick pipeline lex
    just fuzz-quick pipeline ffi_no_abort
    just fuzz-quick render render_html
    just fuzz-quick render serialize_round_trip
    just fuzz-quick render catalogue_normalization
    just fuzz-quick encoding decode_sjis

[doc("Fuzz every registered target for 5 minutes each.")]
fuzz-all-deep:
    just fuzz-deep pipeline lex
    just fuzz-deep pipeline ffi_no_abort
    just fuzz-deep render render_html
    just fuzz-deep render serialize_round_trip
    just fuzz-deep render catalogue_normalization
    just fuzz-deep encoding decode_sjis

[doc("Replay fuzz artifacts and report panic context and remaining crashes.")]
fuzz-triage CRATE TARGET:
    #!/usr/bin/env bash
    set -euo pipefail
    crate="{{CRATE}}"
    target="{{TARGET}}"
    art_dir="crates/aozora/fuzz/${crate}/artifacts/${target}"
    if [[ ! -d "$art_dir" ]]; then
        echo "fuzz-triage: no artifacts for ${crate} / ${target}"
        exit 0
    fi
    failed=0
    for art in $(find "$art_dir" -type f \( -name 'crash-*' -o -name 'leak-*' -o -name 'oom-*' \) | sort); do
        # cargo-fuzz resolves artifact paths relative to crates/aozora/fuzz/<package>/.
        rel="${art#crates/aozora/fuzz/${crate}/}"
        echo "==> ${art}"
        out=$(bash -c "cd crates/aozora/fuzz/${crate} && cargo +{{_FUZZ_TOOLCHAIN}} fuzz run --fuzz-dir . --target {{_fuzz_target}} ${target} ${rel} 2>&1" || true)
        panic_block=$(awk '
            /^thread .* panicked at/ { capturing = 1 }
            capturing {
                if (/^stack backtrace:/ || /^=================/) exit
                print
            }
        ' <<<"$out")
        if [[ -n "$panic_block" ]]; then
            printf "%s\n" "$panic_block"
        else
            tail -5 <<<"$out"
        fi
        if grep -q "exit status: 77" <<<"$out"; then
            failed=$((failed + 1))
        fi
        echo
    done
    if (( failed > 0 )); then
        echo "fuzz-triage: ${failed} artifact(s) still crash" >&2
        exit "${failed}"
    fi
    echo "fuzz-triage: every artifact replays cleanly"

[doc("Move a fuzz artifact into the permanent regression corpus.")]
fuzz-promote CRATE TARGET ARTIFACT:
    #!/usr/bin/env bash
    set -euo pipefail
    src="crates/aozora/fuzz/{{CRATE}}/artifacts/{{TARGET}}/{{ARTIFACT}}"
    dst_dir="crates/aozora/tests/fuzz_regressions/{{CRATE}}/{{TARGET}}"
    if [[ ! -f "$src" ]]; then
        echo "fuzz-promote: artifact not found: $src" >&2
        exit 1
    fi
    bash -c "mkdir -p '$dst_dir' && mv '$src' '$dst_dir/{{ARTIFACT}}'"
    echo "promoted ${src} -> ${dst_dir}/{{ARTIFACT}}"

[doc("Count pending crashes and pinned regressions per fuzz target.")]
fuzz-status:
    #!/usr/bin/env bash
    set -euo pipefail
    targets=(
        "pipeline lex"
        "pipeline ffi_no_abort"
        "render render_html"
        "render serialize_round_trip"
        "render catalogue_normalization"
        "encoding decode_sjis"
    )
    printf "%-22s  %-22s  %-10s  %-12s\n" crate target pending_crashes pinned_regressions
    printf "%-22s  %-22s  %-10s  %-12s\n" ---------------------- ---------------------- ---------- ------------
    for entry in "${targets[@]}"; do
        crate="${entry% *}"
        target="${entry#* }"
        crashes=0
        regressions=0
        art_dir="crates/aozora/fuzz/${crate}/artifacts/${target}"
        reg_dir="crates/aozora/tests/fuzz_regressions/${crate}/${target}"
        if [[ -d "$art_dir" ]]; then
            crashes=$(find "$art_dir" -maxdepth 1 -type f \( -name 'crash-*' -o -name 'leak-*' -o -name 'oom-*' \) 2>/dev/null | wc -l | tr -d ' ')
        fi
        if [[ -d "$reg_dir" ]]; then
            regressions=$(find "$reg_dir" -maxdepth 1 -type f ! -name '*.txt' ! -name '*.md' 2>/dev/null | wc -l | tr -d ' ')
        fi
        printf "%-22s  %-22s  %-10s  %-12s\n" "$crate" "$target" "$crashes" "$regressions"
    done

[doc("Run Criterion benchmarks.")]
bench *ARGS:
    cargo bench --workspace {{ARGS}}

[doc("Save benchmark results as a named baseline.")]
bench-baseline NAME="main":
    cargo bench --workspace -- --save-baseline {{NAME}}

[doc("Compare benchmark results with a named baseline.")]
bench-compare NAME="main":
    cargo bench --workspace -- --baseline {{NAME}}

[doc("Profile heap allocations for synthetic parsing and rendering.")]
dhat:
    cargo run --release -p aozora-bench --example dhat_parse

[doc("Measure small-document parse and render latency percentiles.")]
latency:
    cargo run --release -p aozora-bench --example latency_synthetic

# Region coverage supports the stable toolchain, while branch reporting requires nightly.
_COV_FLOOR := "84"
# Binary entrypoints are thin adapters covered through their libraries.
_COV_IGNORE := "(target/|/main\\.rs$)"

[doc("Check workspace region coverage against the configured floor.")]
coverage:
    # Fresh instrumentation avoids aggregating stale coverage maps after source files move.
    sh -c 'rm -rf "${CARGO_TARGET_DIR:-target}/llvm-cov-target"'
    cargo llvm-cov nextest \
        {{_ws}} \
        --ignore-filename-regex '{{_COV_IGNORE}}' \
        --fail-under-regions {{_COV_FLOOR}}

[doc("Run CLI and in-process LSP tests with all optional features.")]
test-internals:
    # Workspace-wide features would enable the Extism host smoke test without its WASM artifact.
    cargo nextest run -p aozora-cli --all-features

[doc("Generate an HTML coverage report without enforcing a threshold.")]
coverage-html:
    cargo llvm-cov nextest \
        {{_ws}} \
        --ignore-filename-regex '{{_COV_IGNORE}}' \
        --html --output-dir coverage/html

[doc("Report branch coverage with nightly Rust.")]
coverage-branch:
    cargo +{{_FUZZ_TOOLCHAIN}} llvm-cov nextest \
        --branch \
        {{_ws}} \
        --ignore-filename-regex '{{_COV_IGNORE}}'

[doc("Run mutation tests with the supplied cargo-mutants arguments.")]
mutants *ARGS:
    # Disable shell globbing so cargo-mutants receives exclusion patterns verbatim.
    set -f; CARGO_TARGET_DIR=target/mutants CARGO_INCREMENTAL=1 RUSTC_WRAPPER= \
      cargo mutants --config mutants.toml -j 1 {{ARGS}}

[doc("Run formatting, Clippy, spelling, and rustdoc checks.")]
lint: fmt-check clippy typos doc

[doc("Build workspace rustdoc with warnings treated as errors.")]
doc:
    # Rustdoc processes share output files, so serialization avoids concurrent writes.
    env RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features --jobs 1

[doc("Check Rust and TOML formatting without writing files.")]
fmt-check:
    cargo fmt --all -- --check
    taplo fmt --check

[doc("Format Rust and TOML files.")]
fmt:
    cargo fmt --all
    taplo fmt

[doc("Check repository license and copyright coverage.")]
reuse:
    reuse lint

[doc("Lint libraries, binaries, and tests except the benchmark harness.")]
clippy:
    # CLI lint-group flags would override workspace per-lint exceptions (ADR-0004).
    cargo clippy {{_ws}} --lib --bins --all-features -- -D warnings
    # Test targets use panics and output for failure diagnostics.
    cargo clippy {{_ws}} --tests --all-features -- \
      -D warnings -A clippy::unwrap_used -A clippy::expect_used \
      -A clippy::panic -A clippy::print_stdout -A clippy::print_stderr

[doc("Lint all workspace targets and optional features.")]
clippy-strict:
    cargo clippy --workspace --lib --bins --all-features -- -D warnings
    # Test, example, and benchmark targets use panics and output for diagnostics.
    cargo clippy --workspace --examples --tests --benches --all-features -- \
      -D warnings -A clippy::unwrap_used -A clippy::expect_used \
      -A clippy::panic -A clippy::print_stdout -A clippy::print_stderr

[doc("Lint WASM bindings and the Extism plugin on wasm32.")]
clippy-wasm:
    cargo clippy --target wasm32-unknown-unknown -p aozora-wasm -p aozora-extism -- -D warnings

[doc("Run formatting, all Clippy targets, spelling, and rustdoc checks.")]
lint-full: fmt-check clippy-strict clippy-wasm typos doc

[doc("Check spelling.")]
typos:
    typos

[doc("Check dependency licenses, advisories, bans, and sources.")]
deny:
    cargo deny check

[doc("Scan dependencies for RustSec advisories.")]
audit:
    cargo audit

[doc("Check for unused dependencies and unlinked source files.")]
shear:
    cargo shear

[doc("Check API compatibility against published crate baselines.")]
semver *ARGS:
    # tree-sitter-aozora has no published baseline for cargo-semver-checks.
    cargo semver-checks check-release --workspace \
        --exclude tree-sitter-aozora {{ARGS}}

_deps_marker := "target/.deps-check.timestamp"

[doc("List outdated direct workspace dependencies without failing.")]
outdated:
    cargo outdated --workspace --root-deps-only --depth 2 --exit-code 0

[doc("Update workspace dependencies within compatible version ranges.")]
upgrade:
    cargo upgrade --workspace --pinned --recursive
    cargo update --workspace
    @echo "Lockfile updated. Run 'just ci' before committing to verify."

[doc("Update workspace dependencies including incompatible versions.")]
upgrade-incompat:
    cargo upgrade --workspace --incompatible allow --recursive
    cargo update --workspace
    @echo "Lockfile updated WITH incompatible bumps. Review 'git diff Cargo.toml' before committing."

[doc("Check dependency health and record the last successful run.")]
deps-check:
    @mkdir -p target
    @echo "[deps-check] $(date -u +%FT%TZ) — outdated, audit, deny"
    just outdated
    just audit
    just deny
    @date -u +%FT%TZ > {{_deps_marker}}
    @echo "[deps-check] OK — marker written to {{_deps_marker}}"

[doc("Install the weekly dependency-check systemd user timer.")]
deps-timer-install:
    cargo run --release -p aozora-xtask -- deps install-timer

[doc("Show dependency timer status and recent journal entries.")]
deps-timer-status:
    cargo run --release -p aozora-xtask -- deps status

[doc("Remove the dependency timer while preserving its log.")]
deps-timer-uninstall:
    cargo run --release -p aozora-xtask -- deps uninstall-timer

[doc("Report dependency-check freshness and fail when older than seven days.")]
deps-status:
    @if [ ! -f {{_deps_marker}} ]; then \
        echo "[deps-status] never run; run 'just deps-check'"; \
        exit 1; \
    fi
    @ts="$(cat {{_deps_marker}})"; \
    age_secs=$(( $(date -u +%s) - $(date -u -d "$ts" +%s) )); \
    age_days=$(( age_secs / 86400 )); \
    if [ "$age_days" -gt 7 ]; then \
        echo "[deps-status] last check $age_days days ago ($ts) — STALE; run 'just deps-check'"; \
        exit 1; \
    else \
        echo "[deps-status] last check $age_days days ago ($ts) — fresh"; \
    fi

[doc("Build a profile-guided release binary using the configured corpus.")]
pgo:
    env AOZORA_CORPUS_ROOT='{{ env_var("AOZORA_CORPUS_ROOT") }}' bash scripts/pgo-build.sh

[doc("Build and run the C ABI smoke test.")]
smoke-ffi:
    bash crates/aozora-ffi/tests/c_smoke/run.sh

[doc("Build and optimize the portable Extism WASM plugin.")]
extism-build:
    env CARGO_PROFILE_RELEASE_OPT_LEVEL=z cargo build --release --target wasm32-unknown-unknown -p aozora-extism
    sh -c 'set -eu; command -v wasm-opt >/dev/null; \
        mkdir -p crates/aozora-extism/dist \
        && cp "${CARGO_TARGET_DIR:-target}/wasm32-unknown-unknown/release/aozora_extism.wasm" crates/aozora-extism/dist/aozora.wasm \
        && before=$(wc -c < crates/aozora-extism/dist/aozora.wasm) \
        && wasm-opt -Oz --strip-debug --strip-dwarf --vacuum \
            --enable-bulk-memory --enable-mutable-globals \
            --enable-nontrapping-float-to-int \
            crates/aozora-extism/dist/aozora.wasm -o crates/aozora-extism/dist/aozora.wasm \
        && after=$(wc -c < crates/aozora-extism/dist/aozora.wasm) \
        && test "$after" -lt "$before"'

[doc("Check Extism host exports against in-process wire results.")]
smoke-extism: extism-build
    cargo test -p aozora-extism --features host-smoke --test host_smoke -- --nocapture

[doc("Build the plugin and run Go formatting, vet, and SDK tests.")]
smoke-go: extism-build
    bash -c 'set -euo pipefail; \
        cp crates/aozora-extism/dist/aozora.wasm crates/aozora-go/aozora.wasm; \
        cd crates/aozora-go; \
        unformatted=$(gofmt -l .); \
        if [ -n "$unformatted" ]; then echo "gofmt needs: $unformatted"; exit 1; fi; \
        go vet ./...; \
        go test ./...'

[doc("Build and install a Python wheel, then run mypy and pytest.")]
smoke-py:
    env AOZORA_PY_VENV=target/venv-smoke-py bash scripts/smoke-py.sh

[doc("Check Node WASM outputs against committed conformance goldens.")]
parity-wasm:
    bash -euc 'CARGO_PROFILE_RELEASE_OPT_LEVEL=z wasm-pack build --target nodejs --release crates/aozora-wasm --out-dir pkg-nodejs \
        && node crates/aozora-wasm/tests/js/parity.mjs crates/aozora-wasm/pkg-nodejs'

[doc("Report GitHub Actions job and step timings for completed runs.")]
ci-profile *ARGS:
    cargo run -q --release -p aozora-xtask -- ci profile {{ARGS}}

[doc("Lint GitHub Actions workflows and embedded shell commands.")]
actionlint:
    actionlint -no-color -shellcheck=shellcheck -pyflakes=

[doc("Check GitHub Actions security offline for high-confidence, high-severity findings.")]
zizmor:
    zizmor --offline --no-progress --min-severity high --min-confidence high .

[doc("Run locked library tests on a native arm64 host.")]
test-aarch64:
    #!/usr/bin/env bash
    set -euo pipefail
    [[ "$(uname -m)" == "aarch64" ]] || { echo "test-aarch64 requires a native arm64 host" >&2; exit 2; }
    cargo test --locked -p aozora

[doc("Check that the library builds for WASI.")]
test-wasm:
    cargo build --locked --target wasm32-wasip1 -p aozora

[doc("Run Rust lint, build, test, coverage, conformance, and drift gates.")]
ci-rust:
    just lint-full
    just build
    just test
    just test-doc
    just test-doc-all
    just test-internals
    just prop
    just coverage
    just conformance
    just drift-gate

[doc("Run WASM, playground, browser, Lighthouse, and VS Code gates.")]
ci-web:
    just clippy-wasm
    just parity-wasm
    just playground-ci
    just playground-e2e
    cd playground && bun run lighthouse
    just vscode-ci

[doc("Run C ABI, Extism, and Go SDK smoke tests.")]
ci-bindings:
    just smoke-ffi
    just smoke-extism
    just smoke-go

[doc("Run repository spelling, dependency, license, workflow, and coordinate checks.")]
ci-repo:
    just typos
    just deny
    just audit
    just shear
    just reuse
    just actionlint
    just zizmor
    cargo run -p aozora-xtask -q -- lint coordinates

[doc("Run all required corpus correctness, allocation, and incremental speed gates.")]
ci-corpus:
    #!/usr/bin/env bash
    set -euo pipefail
    [[ -n "${AOZORA_CORPUS_ROOT:-}" && -d "$AOZORA_CORPUS_ROOT" ]] \
      || { echo "AOZORA_CORPUS_ROOT must name the pinned corpus checkout" >&2; exit 2; }
    just audit-gate
    just corpus-sweep
    just verbatim-gate
    just render-leak-gate
    just render-correctness-gate
    just alloc-gate
    just incremental-speedup-gate

[doc("Check native prerequisites and instruction-count performance.")]
ci-perf: native-prerequisites
    just perf-gate

[doc("Check publish contracts, API compatibility, crate archives, and artifact sizes.")]
ci-release:
    just publish-check
    just semver
    just artifact-crates
    just artifact-size-gate

[doc("Build every registered fuzz package with the pinned nightly.")]
ci-fuzz:
    #!/usr/bin/env bash
    set -euo pipefail
    for crate in pipeline render encoding; do
      (
        cd "crates/aozora/fuzz/$crate"
        cargo +{{_FUZZ_TOOLCHAIN}} fuzz build --fuzz-dir . --target {{_fuzz_target}}
      )
    done

[doc("Run native prerequisites and the Rust, WASI, web, bindings, repository, and performance suites.")]
ci: native-prerequisites
    just ci-rust
    just test-wasm
    just ci-web
    just ci-bindings
    just ci-repo
    just ci-perf

[doc("Start the bacon watcher with an optional job name.")]
watch JOB="":
    bacon {{JOB}}

[doc("Watch files and rerun Clippy.")]
watch-lint:
    bacon clippy

[doc("Watch files and rerun nextest.")]
watch-test:
    bacon test

[doc("Watch files with plain output and the selected bacon job.")]
watch-headless JOB="check":
    bacon --headless --job {{JOB}}

[doc("Install Git hooks with Lefthook.")]
hooks:
    lefthook install

[doc("Build and optimize the playground WASM package.")]
playground-wasm:
    env CARGO_PROFILE_RELEASE_OPT_LEVEL=z wasm-pack build --target web --release crates/aozora-wasm
    sh -c 'set -eu; wasm=crates/aozora-wasm/pkg/aozora_wasm_bg.wasm; \
        before=$(wc -c < "$wasm"); \
        wasm-opt -Oz --strip-debug --strip-dwarf --vacuum \
            --enable-bulk-memory --enable-mutable-globals \
            --enable-nontrapping-float-to-int "$wasm" -o "$wasm"; \
        after=$(wc -c < "$wasm"); \
        test "$after" -lt "$before"'

[doc("Ensure the playground WASM package and locked Bun dependencies exist.")]
_playground-ensure:
    [ -d crates/aozora-wasm/pkg ] || just playground-wasm
    cd playground && bun install --frozen-lockfile

[doc("Type-check playground TypeScript sources.")]
playground-typecheck: _playground-ensure
    cd playground && bun run typecheck

[doc("Run playground and shared-package tests with coverage.")]
playground-test: _playground-ensure
    cd playground && bun run test:coverage

[doc("Run playground type, coverage, lint, CSS, and legacy checks.")]
playground-ci: _playground-ensure
    cd playground && bun run typecheck
    cd playground && bun run test:coverage
    cd playground && bun run lint
    cd playground && bun run lint:css
    cd playground && bun run check:legacy

[doc("Build the playground and check its bundle.")]
playground-build: playground-wasm _playground-ensure
    cd playground && bun run build
    cd playground && bun run check:bundle

[doc("Run playground gates and export the production build.")]
playground-all: playground-ci playground-build
    bash -euc 'destination=target/release-ready-build/playground; rm -rf "$destination"; mkdir -p "$destination"; cp -R playground/dist/. "$destination/"; test -s "$destination/index.html"'

[doc("Build the playground and run Playwright browser tests.")]
playground-e2e: playground-build
    # Playwright browser dependencies must be installed by the host package manager.
    cd playground && bun run playwright install chromium firefox webkit
    cd playground && bun run playwright test

[doc("Build the playground and run Lighthouse audits.")]
playground-lighthouse: playground-build
    cd playground && bun run playwright install chromium
    cd playground && bun run lighthouse

[doc("Install the VS Code extension's locked Bun dependencies.")]
_vscode-ensure:
    bash -euc 'cd editors/vscode && bun install --frozen-lockfile'

[doc("Type-check the VS Code extension.")]
vscode-typecheck: _vscode-ensure
    bash -euc 'cd editors/vscode && bun run typecheck'

[doc("Lint, type-check, bundle, and test the VS Code extension.")]
vscode-ci: _vscode-ensure
    bash -euc 'cd editors/vscode && bun run check && bun run compile && bun run test'

[doc("Sample-profile one document relative to the configured corpus.")]
samply-doc DOC:
    cargo run --release -p aozora-xtask -- samply doc {{DOC}}

[doc("Sample-profile corpus parsing for the requested repeat count.")]
samply-corpus REPEAT="5":
    cargo run --release -p aozora-xtask -- samply corpus {{REPEAT}}

[doc("Sample-profile corpus HTML rendering for the requested repeat count.")]
samply-render REPEAT="5":
    cargo run --release -p aozora-xtask -- samply render {{REPEAT}}

[doc("Cache symbolication for a saved trace and binary.")]
trace-cache TRACE BIN="target/release/examples/throughput_by_class":
    cargo run --release -p aozora-xtask -- trace cache {{TRACE}} {{BIN}}

[doc("Show the hottest leaf frames in a saved trace.")]
trace-hot TRACE TOP="25":
    cargo run --release -p aozora-xtask -- trace hot {{TRACE}} --top {{TOP}}

[doc("Show the hottest frames including descendant samples.")]
trace-hot-inclusive TRACE TOP="25":
    cargo run --release -p aozora-xtask -- trace hot {{TRACE}} --top {{TOP}} --inclusive

[doc("Show trace sample counts by library.")]
trace-libs TRACE:
    cargo run --release -p aozora-xtask -- trace libs {{TRACE}}

[doc("Group trace samples by parser and corpus phase.")]
trace-rollup TRACE:
    cargo run --release -p aozora-xtask -- trace rollup {{TRACE}}

[doc("Show call stacks matching a regular expression.")]
trace-stacks TRACE PATTERN LIMIT="5":
    cargo run --release -p aozora-xtask -- trace stacks {{TRACE}} --pattern {{PATTERN}} --limit {{LIMIT}}

[doc("Compare hot frames between two saved traces.")]
trace-compare BEFORE AFTER TOP="25":
    cargo run --release -p aozora-xtask -- trace compare {{BEFORE}} {{AFTER}} --top {{TOP}}

[doc("Emit folded stacks for a flamegraph renderer.")]
trace-flame TRACE:
    cargo run --release -p aozora-xtask -- trace flame {{TRACE}}

[doc("Remove Lefthook Git hook stubs.")]
hooks-uninstall:
    lefthook uninstall

[doc("Remove workspace build artifacts.")]
clean:
    cargo clean --workspace
