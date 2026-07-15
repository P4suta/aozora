//! Host-side dev tooling for the aozora workspace.
//!
//! Today: `xtask samply <doc|corpus> …` wrappers around the
//! [`samply`](https://github.com/mstange/samply) sampling profiler.
//!
//! ## Why not a shell script
//!
//! An earlier attempt sat in `scripts/samply-doc.sh` /
//! `scripts/samply-corpus.sh`. It was rewritten in Rust because:
//! - the rest of the project is Rust 2024 — keeping tooling in the
//!   same language means one toolchain, one set of types, one set of
//!   error messages
//! - shell scripts add a parallel shell-quoting / `set -euo pipefail`
//!   surface that is fundamentally harder to reason about
//! - portability: bash idioms break on Windows / non-bash shells; an
//!   `xtask` binary works wherever `cargo run` does
//!
//! ## Why on the host (not Docker)
//!
//! `samply` opens `perf_event_open(2)` directly against the kernel.
//! Docker's default seccomp profile blocks it; even with
//! `--privileged --pid=host` the kernel's `/proc/sys/kernel/perf_event_paranoid`
//! is read inside the container's PID namespace, which doesn't
//! match what the host's perf-events subsystem will allow.
//! Bottom line: profiling needs to be on the host, period.
//!
//! ## Why a separate crate (and not part of `aozora-bench`)
//!
//! `aozora-bench` is a library + examples crate consumed by `cargo
//! bench` and `cargo run --example`. Adding a binary target to it
//! would tie the bench compile to a binary that nobody benchmarking
//! actually wants. The `xtask` pattern keeps developer tooling in a
//! dedicated crate that is not built by `just build`'s default path.
//! The crate is `publish = false` since it's not a library
//! consumers depend on.

#![allow(
    clippy::disallowed_methods,
    reason = "xtask binary uses std::process::exit / std::env::set_var to wire up the spawned `cargo` and `samply` invocations; both are appropriate here, in the dev-tooling crate, but disallowed elsewhere"
)]
#![allow(
    missing_docs,
    reason = "host dev-tool binary — not a public API surface"
)]

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command, ExitStatus};

use clap::{Args, Parser, Subcommand, ValueEnum};

mod ci;
mod conformance;
mod corpus;
mod deps;
mod grammar;
mod msrv;
mod publish;
mod schema;
mod spec_vectors;
mod trace;
mod types;
mod version;

pub(crate) use ci::CiArgs;
pub(crate) use corpus::CorpusArgs;
pub(crate) use deps::DepsArgs;
pub(crate) use trace::TraceArgs;
pub(crate) use version::VersionArgs;

const PERF_PARANOID_PATH: &str = "/proc/sys/kernel/perf_event_paranoid";
const PERF_PARANOID_MAX: i32 = 1;
const SAMPLY_RATE_HZ: u32 = 4000;
const DEFAULT_CORPUS_REPEAT: usize = 5;
const DEFAULT_RENDER_REPEAT: usize = 5;

#[derive(Parser)]
#[command(name = "xtask", about = "aozora developer tooling", version)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Sample-profile a target via `samply`.
    Samply(SamplyArgs),
    /// Analyse a saved samply `.json.gz` trace via `aozora-trace`.
    Trace(TraceArgs),
    /// Local-only dependency-follow-up tooling — install / inspect /
    /// remove the systemd user timer that runs `just deps-check`
    /// weekly. Replaces the dependabot / GitHub-Actions pattern with
    /// a host-side pure-Rust mechanism.
    Deps(DepsArgs),
    /// Build / inspect aozora-corpus binary archives. Replaces the
    /// directory-of-17-k-small-files load shape with a single packed
    /// file that can be raw SJIS, pre-decoded UTF-8, and/or zstd-
    /// compressed. The pack step is incremental — entries whose source
    /// `mtime` and `blake3` hash match the previous archive are copied
    /// verbatim.
    Corpus(CorpusArgs),
    /// CI / GitHub Actions instrumentation: profile a finished run,
    /// run every CI job locally before pushing, or replay a workflow
    /// job through `nektos/act`.
    Ci(CiArgs),
    /// JSON Schema artefact dump / drift gate for the `aozora::json`
    /// envelopes. Generates schema files under
    /// `crates/aozora-conformance/json/schema-*.json` and CI-checks
    /// that they stay in sync with the live wire shape.
    Schema(SchemaArgs),
    /// TypeScript types artefact dump / drift gate. Generates
    /// `crates/aozora-wasm/pkg/aozora_types.d.ts` from the live
    /// enums (`NodeKind` / `PairKind` / `Severity` /
    /// `DiagnosticSource` / `InternalCheckCode`) and wire structs.
    /// Drift-gated like `xtask schema`.
    Types(TypesArgs),
    /// WPT-style conformance suite runner. Walks every
    /// fixture under `crates/aozora-conformance/fixtures/render/`,
    /// runs the parser, and reports pass/fail counts per
    /// `(feature, level)` pair declared in each fixture's
    /// `meta.toml`. Exits non-zero on any `must`-tier failure.
    Conformance(ConformanceArgs),
    /// Print the channel-aware build-version string (the `AOZORA_BUILD_VERSION`
    /// format) — the single source of the `dev` / `nightly` / `stable` version
    /// identity. `nightly.yml` / `release.yml` call this so the binaries are
    /// stamped from one place. `nightly` needs `--date YYYYMMDD`.
    Version(VersionArgs),
    /// Vendor / drift-check the conformance vectors against the sibling
    /// `aozora-notation-spec` repo (the source of truth). `sync` copies the
    /// vectors + schema + RUNNER.md into
    /// `crates/aozora-conformance/spec-vectors/`; `check` fails the build
    /// when the vendored copy has drifted (`--allow-missing` skips where the
    /// sibling isn't checked out, i.e. the dev container / cloud CI).
    SpecVectors(SpecVectorsArgs),
    /// crates.io publish-path ledger drift gate. Offline — never contacts
    /// a registry; it reads manifests only. Cross-checks the workspace's
    /// publishable members against `release-plz.toml`'s `changelog_include`
    /// (a crate added to the workspace but not to that list drops out of
    /// the aggregated CHANGELOG), and enforces the manifest hygiene rules
    /// the root `Cargo.toml` already states in prose: path-only internal
    /// dev-deps, and no registry `version` on a `publish = false` member.
    Publish(PublishArgs),
    /// MSRV / toolchain pin coherence gate. `rust-toolchain.toml`'s
    /// channel (the DEV toolchain, tracking latest stable) and
    /// `Cargo.toml`'s `rust-version` (the PUBLIC CONTRACT, a measured
    /// floor) are two authorities holding deliberately different numbers
    /// (ADR-0034). Checks that every other pin follows the right one, that
    /// the handbook names a Rust version in exactly one page, that the
    /// READMEs derive the MSRV badge rather than writing it down, and that
    /// the contract stays at least six months behind the channel.
    Msrv(MsrvArgs),
}

#[derive(Args)]
struct MsrvArgs {
    #[command(subcommand)]
    op: MsrvOp,
}

#[derive(Subcommand)]
enum MsrvOp {
    /// Fail when a version pin follows the wrong authority, or when the
    /// MSRV drifts within six months of the dev channel. Wired into
    /// `drift-gate`.
    Check,
}

#[derive(Args)]
struct PublishArgs {
    #[command(subcommand)]
    op: PublishOp,
}

#[derive(Subcommand)]
enum PublishOp {
    /// Fail when the publish ledger has drifted — `release-plz.toml`
    /// disagreeing with the workspace's publish set, or a manifest
    /// breaking the publish-path hygiene rules. Wired into `drift-gate`.
    Check,
}

#[derive(Args)]
struct SpecVectorsArgs {
    #[command(subcommand)]
    op: SpecVectorsOp,
}

#[derive(Subcommand)]
enum SpecVectorsOp {
    /// Fail when the vendored `spec-vectors/` has drifted from the sibling
    /// spec's `conformance/` subtree (vectors + schema + RUNNER.md).
    Check {
        /// Treat an absent sibling checkout as a skip (exit 0) instead of an
        /// error. The vendored copy is authoritative where the spec isn't
        /// checked out (dev container / cloud CI).
        #[arg(long)]
        allow_missing: bool,
    },
    /// Copy the vectors + schema + RUNNER.md out of the sibling spec into the
    /// vendored copy, replacing them wholesale so spec-side deletions
    /// propagate. Commit the diff.
    Sync,
}

#[derive(Args)]
struct SchemaArgs {
    #[command(subcommand)]
    op: SchemaOp,
}

#[derive(Subcommand)]
enum SchemaOp {
    /// Generate the four wire-format schemas and write them to
    /// `crates/aozora-conformance/json/schema-*.json`. Overwrites
    /// existing files; commit the diff.
    Dump,
    /// Compare on-disk schemas against freshly-generated ones; exit
    /// non-zero on drift. Used as a CI gate so renamed fields /
    /// added variants force the artefact regeneration step.
    Check,
}

#[derive(Args)]
struct ConformanceArgs {
    #[command(subcommand)]
    op: ConformanceOp,
}

#[derive(Args)]
struct RunArgs {
    /// Which implementation to measure conformance for.
    #[arg(long, value_enum, default_value_t = Implementation::Rust)]
    implementation: Implementation,
    /// Regenerate the `tree-sitter` S-expression snapshots
    /// (`expected.tree-sitter.txt`) and the results artefact. Use after
    /// an intentional grammar change; no effect on the `rust`
    /// implementation.
    #[arg(long)]
    update: bool,
}

#[derive(Args)]
struct VectorsArgs {
    /// Which implementation to run the specification vectors through.
    #[arg(long, value_enum, default_value_t = Implementation::Rust)]
    implementation: Implementation,
    /// Regenerate the `tree-sitter` spec-vector S-expression snapshot
    /// (`spec-vectors/tree-sitter-snapshot.json`). Use after an
    /// intentional grammar change; no effect on the `rust` implementation.
    #[arg(long)]
    update: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum Implementation {
    /// The canonical Rust parser (`crates/aozora-pipeline`).
    Rust,
    /// The reference tree-sitter grammar (`crates/tree-sitter-aozora`).
    #[value(name = "tree-sitter")]
    TreeSitter,
}

#[derive(Subcommand)]
enum ConformanceOp {
    /// Run every fixture against the chosen `--implementation`.
    ///
    /// `rust` (default) is the canonical parser: it compares each
    /// fixture's `to_html()` / `to_source()` to the committed goldens,
    /// writes a per-case results.json under
    /// `crates/aozora-conformance/conformance-results.json`, and exits
    /// non-zero on any `must`-tier failure.
    ///
    /// `tree-sitter` runs the reference grammar
    /// (`crates/tree-sitter-aozora`): it reports the per-tier pass rate
    /// (a fixture "passes" when the grammar parses it without ERROR
    /// nodes) and gates on a per-fixture S-expression snapshot
    /// (`expected.tree-sitter.txt`). Any snapshot drift exits non-zero,
    /// tier-independent; pass `--update` to regenerate the snapshots
    /// after an intentional grammar change.
    Run(RunArgs),
    /// Run the vendored specification conformance vectors
    /// (`crates/aozora-conformance/spec-vectors/`) against the chosen
    /// `--implementation`.
    ///
    /// `rust` (default) holds the parser to each vector's `expected`
    /// projections (`serialize` / `nodes` / `pairs` / `diagnostics`) per
    /// its `meta.level`: `must` mismatches exit non-zero; `should` / `may`
    /// warn. The `html` channel is informative (spec §8) and only warns.
    ///
    /// `tree-sitter` runs the reference grammar over each vector's
    /// `source`: it reports the per-tier pass rate (no ERROR nodes) and
    /// gates on the `spec-vectors/tree-sitter-snapshot.json` S-expression
    /// snapshot. Any drift exits non-zero; `--update` regenerates it.
    Vectors(VectorsArgs),
    /// Regenerate-drift gate for the committed tree-sitter parser.
    ///
    /// `crates/tree-sitter-aozora/grammar.js` is the source of truth; the
    /// checked-in `src/parser.c` (+ `grammar.json` / `node-types.json`) is
    /// `tree-sitter generate`'s output, compiled by `build.rs` into the
    /// static parser `aozora-lsp` links against. `--check` (default,
    /// wired into `drift-gate`) exits non-zero when those artefacts have
    /// drifted from a fresh generate; `--update` regenerates them in place
    /// via the pinned `tree-sitter` CLI. Commit the diff.
    Grammar(GrammarArgs),
}

#[derive(Args)]
struct GrammarArgs {
    /// Regenerate the committed grammar artefacts
    /// (`crates/tree-sitter-aozora/src/{parser.c,grammar.json,node-types.json}`)
    /// in place from `grammar.js` via the pinned `tree-sitter` CLI, then
    /// exit. Use after an intentional grammar edit; commit the diff.
    #[arg(long)]
    update: bool,
    /// Verify the committed artefacts still match a fresh `tree-sitter
    /// generate`; exit non-zero on drift. This is the default when neither
    /// flag is given, and the form the `drift-gate` CI job runs.
    #[arg(long, conflicts_with = "update")]
    check: bool,
}

#[derive(Args)]
struct TypesArgs {
    #[command(subcommand)]
    op: TypesOp,
}

#[derive(Subcommand)]
enum TypesOp {
    /// Generate `aozora_types.d.ts` from the live enums + wire
    /// structs and write it under `crates/aozora-wasm/pkg/`.
    /// Overwrites the existing file; commit the diff.
    Ts,
    /// Compare on-disk `aozora_types.d.ts` against fresh codegen;
    /// exit non-zero on drift. CI gate.
    Check,
    /// Generate native wire types for every host-SDK language from the
    /// committed JSON Schema via `quicktype` (one generator, all
    /// languages). Writes one file per language (e.g.
    /// `crates/aozora-go/json_gen.go`); overwrites it, commit
    /// the diff.
    Langs,
    /// Compare each on-disk per-language wire types file against fresh
    /// `quicktype` codegen; exit non-zero on drift. CI gate (extends
    /// `drift-gate`).
    LangsCheck,
}

#[derive(Args)]
struct SamplyArgs {
    #[command(subcommand)]
    target: SamplyTarget,
}

#[derive(Subcommand)]
enum SamplyTarget {
    /// Profile a single corpus document via the `pathological_probe` example.
    ///
    /// The probe runs `lex` 100 times on the doc, so a 232 KB
    /// outlier doc gives samply ~170 ms of parser-bound wall time at 4 kHz
    /// = ~700 samples. Larger docs (e.g. 3 MB doc 50685) give richer
    /// traces (~10 k samples).
    Doc {
        /// Corpus-relative path under `AOZORA_CORPUS_ROOT`.
        ///
        /// e.g. `001529/files/50685_ruby_67979/50685_ruby_67979.txt`.
        relative_path: String,

        /// Output basename (the `.json.gz` is appended). Defaults to
        /// `aozora-doc-<file-stem>` so multiple runs on different docs
        /// don't clobber each other.
        #[arg(long)]
        out_name: Option<String>,
    },
    /// Profile the parser hot path across the full corpus via the
    /// `throughput_by_class` example.
    ///
    /// `repeat` controls how many times the parse pass is replayed
    /// after the corpus is loaded — higher values give samply more
    /// parser-bound wall time to attach to (the corpus load happens
    /// once and contributes mostly Shift-JIS decode + filesystem
    /// syscalls, which would otherwise dominate the trace).
    Corpus {
        /// Number of parse passes after the one-time corpus load.
        #[arg(default_value_t = DEFAULT_CORPUS_REPEAT)]
        repeat: usize,
    },
    /// Profile the **HTML render** hot path across the full corpus via
    /// the `render_hot_path` example.
    ///
    /// `repeat` controls the per-doc render loop count. Default 5 so
    /// render-bound stack frames dominate the trace; the per-doc parse
    /// (untimed in the probe report but still on the wall) drops to a
    /// minority of samples at this multiplier.
    Render {
        /// Number of `render_to_string` calls per document.
        #[arg(default_value_t = DEFAULT_RENDER_REPEAT)]
        repeat: usize,
    },
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Cmd::Samply(args) => match args.target {
            SamplyTarget::Doc {
                relative_path,
                out_name,
            } => samply_doc(&relative_path, out_name.as_deref()),
            SamplyTarget::Corpus { repeat } => samply_corpus(repeat),
            SamplyTarget::Render { repeat } => samply_render(repeat),
        },
        Cmd::Trace(args) => trace::dispatch(args),
        Cmd::Deps(args) => deps::dispatch(&args),
        Cmd::Corpus(args) => corpus::dispatch(&args),
        Cmd::Ci(args) => ci::run(&args),
        Cmd::Schema(args) => schema::dispatch(&args),
        Cmd::Types(args) => types::dispatch(&args),
        Cmd::Conformance(args) => conformance::dispatch(&args),
        Cmd::Version(args) => version::dispatch(&args),
        Cmd::SpecVectors(args) => spec_vectors::dispatch(&args),
        Cmd::Publish(args) => publish::dispatch(&args),
        Cmd::Msrv(args) => msrv::dispatch(&args),
    };
    if let Err(err) = result {
        eprintln!("xtask: {err}");
        process::exit(1);
    }
}

/// Sample-profile a single corpus document via `pathological_probe`.
fn samply_doc(relative_path: &str, out_name: Option<&str>) -> Result<(), String> {
    let corpus_root = require_env("AOZORA_CORPUS_ROOT")?;
    let doc_full = Path::new(&corpus_root).join(relative_path);
    if !doc_full.is_file() {
        return Err(format!("doc not found at {}", doc_full.display()));
    }
    require_perf_paranoid()?;

    let basename = out_name.map_or_else(|| derive_basename(relative_path), str::to_owned);
    let out = PathBuf::from("/tmp").join(format!("aozora-doc-{basename}.json.gz"));

    rebuild_with_debug("pathological_probe")?;
    let bin = bench_example_path("pathological_probe")?;

    eprintln!(
        ">>> samply: doc={relative_path}\n           out={}",
        out.display()
    );
    let status = Command::new("samply")
        .arg("record")
        .arg("--save-only")
        .arg("--no-open")
        .arg("-o")
        .arg(&out)
        .arg("-r")
        .arg(SAMPLY_RATE_HZ.to_string())
        .arg("--")
        .arg(bin)
        .env("AOZORA_PROBE_DOC", relative_path)
        .status()
        .map_err(|e| format!("failed to spawn samply: {e}"))?;
    expect_status(status, "samply record")?;

    eprintln!();
    eprintln!(">>> done. inspect with:");
    eprintln!(
        "    samply load {}        # opens local Firefox-Profiler UI",
        out.display()
    );
    Ok(())
}

/// Sample-profile the corpus parser hot path via `throughput_by_class`.
fn samply_corpus(repeat: usize) -> Result<(), String> {
    require_env("AOZORA_CORPUS_ROOT")?;
    require_perf_paranoid()?;

    let timestamp = current_yyyymmdd_hhmmss();
    let out = PathBuf::from("/tmp").join(format!("aozora-corpus-{timestamp}.json.gz"));

    rebuild_with_debug("throughput_by_class")?;
    let bin = bench_example_path("throughput_by_class")?;

    eprintln!(
        ">>> samply: repeat={repeat}\n           out={}",
        out.display()
    );
    let status = Command::new("samply")
        .arg("record")
        .arg("--save-only")
        .arg("--no-open")
        .arg("-o")
        .arg(&out)
        .arg("-r")
        .arg(SAMPLY_RATE_HZ.to_string())
        .arg("--")
        .arg(bin)
        .env("AOZORA_PROFILE_REPEAT", repeat.to_string())
        .status()
        .map_err(|e| format!("failed to spawn samply: {e}"))?;
    expect_status(status, "samply record")?;

    eprintln!();
    eprintln!(">>> done. inspect with:");
    eprintln!(
        "    samply load {}        # opens local Firefox-Profiler UI",
        out.display()
    );
    Ok(())
}

/// Sample-profile the HTML render hot path via `render_hot_path`.
/// `repeat` controls per-doc render-loop iterations so render frames
/// dominate the trace over the per-doc parse warmup.
fn samply_render(repeat: usize) -> Result<(), String> {
    require_env("AOZORA_CORPUS_ROOT")?;
    require_perf_paranoid()?;

    let timestamp = current_yyyymmdd_hhmmss();
    let out = PathBuf::from("/tmp").join(format!("aozora-render-{timestamp}.json.gz"));

    rebuild_with_debug("render_hot_path")?;
    let bin = bench_example_path("render_hot_path")?;

    eprintln!(
        ">>> samply: repeat={repeat}\n           out={}",
        out.display()
    );
    let status = Command::new("samply")
        .arg("record")
        .arg("--save-only")
        .arg("--no-open")
        .arg("-o")
        .arg(&out)
        .arg("-r")
        .arg(SAMPLY_RATE_HZ.to_string())
        .arg("--")
        .arg(bin)
        .env("AOZORA_RENDER_REPEAT", repeat.to_string())
        .status()
        .map_err(|e| format!("failed to spawn samply: {e}"))?;
    expect_status(status, "samply record")?;

    eprintln!();
    eprintln!(">>> done. inspect with:");
    eprintln!(
        "    samply load {}        # opens local Firefox-Profiler UI",
        out.display()
    );
    Ok(())
}

fn require_env(key: &str) -> Result<OsString, String> {
    env::var_os(key).ok_or_else(|| format!("{key} not set"))
}

/// Refuse to launch samply when `perf_event_paranoid` is too high.
///
/// Samply uses `perf_event_open(2)` to sample the CPU. The Linux
/// kernel hides that syscall behind `kernel.perf_event_paranoid`,
/// which on most distros defaults to `2` ("block all unprivileged
/// perf access") — samply will spawn but record zero samples.
///
/// We catch this *before* spawning samply because samply itself
/// fails late and silently (a half-empty trace looks like "your
/// program ran too fast"). The error message gives the user a
/// one-shot fix, a permanent fix, and a "why this is needed"
/// explanation in 12 lines or less.
fn require_perf_paranoid() -> Result<(), String> {
    let raw = match fs::read_to_string(PERF_PARANOID_PATH) {
        Ok(s) => s,
        Err(e) => {
            return Err(format!(
                "\n\
                 ╭─────────────────────────────────────────────────────────────────╮\n\
                 │  ❌  Cannot read {PERF_PARANOID_PATH:46}  │\n\
                 │      ({e:60}) │\n\
                 │                                                                 │\n\
                 │      Samply needs perf_event_open(2). Without this file we      │\n\
                 │      can't tell whether the kernel will allow it. Bailing now.  │\n\
                 ╰─────────────────────────────────────────────────────────────────╯"
            ));
        }
    };
    let level: i32 = raw
        .trim()
        .parse()
        .map_err(|e| format!("failed to parse {PERF_PARANOID_PATH}={raw:?}: {e}"))?;
    if level > PERF_PARANOID_MAX {
        return Err(format_paranoid_blocked(level));
    }
    Ok(())
}

/// Format the user-facing message shown when `perf_event_paranoid`
/// is too high. Extracted from [`require_perf_paranoid`] for testing
/// + so the layout is reviewable in isolation.
fn format_paranoid_blocked(level: i32) -> String {
    format!(
        "\n\
         ╭──────────────────────────────────────────────────────────────────────╮\n\
         │  🔒  perf_event_paranoid = {level} — samply CANNOT collect samples here. │\n\
         ╰──────────────────────────────────────────────────────────────────────╯\n\
         \n\
         ▸ One-shot fix (resets at next reboot):\n     \
             echo {PERF_PARANOID_MAX} | sudo tee {PERF_PARANOID_PATH}\n\
         \n\
         ▸ Permanent fix (survives reboots):\n     \
             echo 'kernel.perf_event_paranoid = {PERF_PARANOID_MAX}' | sudo tee /etc/sysctl.d/99-perf.conf\n     \
             sudo sysctl --system\n\
         \n\
         ▸ Why this is required:\n     \
             samply uses perf_event_open(2) to sample the CPU at {SAMPLY_RATE_HZ}Hz.\n     \
             The kernel guards that syscall behind perf_event_paranoid; the\n     \
             default of 2 blocks all unprivileged use. samply would otherwise\n     \
             spawn but record zero samples, which looks like 'your program ran\n     \
             too fast' — much harder to diagnose than this message.\n\
         \n\
         ▸ Security note:\n     \
             Setting paranoid=1 lets unprivileged processes profile their own\n     \
             children. Lower than the default 2 but still safer than 0 (which\n     \
             would expose kernel internals). For a single-user dev workstation\n     \
             this is the standard recommendation.\n"
    )
}

/// Rebuild a bench example with debug info preserved so samply can
/// symbolicate the resulting binary. `--profile=bench` inherits from
/// `release` but overrides `strip = "none"` and `debug = 1`.
///
/// We deliberately do NOT use `cargo run --release` here — that
/// invocation strips debug info and clobbers any prior bench-profile
/// build of the same binary, which is the foot-gun the original
/// shell script existed to avoid.
fn rebuild_with_debug(example: &str) -> Result<(), String> {
    eprintln!(">>> rebuilding {example} with debug info (--profile=bench)");
    let status = Command::new(env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo")))
        .arg("build")
        .arg("--profile=bench")
        .arg("--example")
        .arg(example)
        .arg("-p")
        .arg("aozora-bench")
        .status()
        .map_err(|e| format!("failed to spawn cargo: {e}"))?;
    expect_status(status, "cargo build --profile=bench")
}

/// Resolve the on-disk path of the bench-profile example binary.
/// Cargo writes profile=bench output to `target/release/examples/`
/// (the `release` directory is shared with `--release`; bench layers
/// on debug info via `[profile.bench] strip = "none"; debug = 1`).
fn bench_example_path(example: &str) -> Result<PathBuf, String> {
    let workspace = workspace_root()?;
    let path = workspace
        .join("target")
        .join("release")
        .join("examples")
        .join(example);
    if !path.is_file() {
        return Err(format!(
            "expected bench example at {} (build skipped or failed?)",
            path.display()
        ));
    }
    Ok(path)
}

/// Walk up from the binary's invocation directory to find the
/// workspace `Cargo.toml`. Cargo sets `CARGO_MANIFEST_DIR` for the
/// xtask crate, so the workspace root is the parent of the
/// `crates/` directory above us.
fn workspace_root() -> Result<PathBuf, String> {
    let manifest_dir = env::var_os("CARGO_MANIFEST_DIR").ok_or_else(|| {
        "CARGO_MANIFEST_DIR not set (xtask must be run via `cargo run -p aozora-xtask`)".to_owned()
    })?;
    let manifest_dir = PathBuf::from(manifest_dir);
    // `crates/aozora-xtask/Cargo.toml` → workspace root is two up.
    manifest_dir
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            format!(
                "cannot derive workspace root from CARGO_MANIFEST_DIR={}",
                manifest_dir.display()
            )
        })
}

fn expect_status(status: ExitStatus, label: &str) -> Result<(), String> {
    if status.success() {
        Ok(())
    } else {
        Err(format!("{label} failed: {status}"))
    }
}

fn derive_basename(relative_path: &str) -> String {
    // Strip the directory + `.txt` suffix → just the file stem.
    Path::new(relative_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("doc")
        .to_owned()
}

/// `YYYYMMDD-HHMMSS` derived from `SystemTime` without pulling in
/// `chrono` for a single invocation. Day-precision is enough to
/// disambiguate per-session profile runs in `/tmp`.
fn current_yyyymmdd_hhmmss() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    // Convert to UTC broken-down time using the gregorian-day algorithm.
    // Good enough for filenames; not for actual datetime work.
    let (year, month, day, hour, minute, second) = secs_to_utc(secs);
    format!("{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}")
}

/// Tiny epoch-seconds → (Y, M, D, h, m, s) for filename timestamps.
/// Doesn't handle leap seconds or pre-1970 inputs (irrelevant here).
#[allow(
    clippy::cast_possible_truncation,
    reason = "epoch sub-day quantities and the day index fit in u32; explicit `as u32` is the simplest expression for this throwaway date-format helper"
)]
fn secs_to_utc(mut secs: u64) -> (u32, u32, u32, u32, u32, u32) {
    let second = (secs % 60) as u32;
    secs /= 60;
    let minute = (secs % 60) as u32;
    secs /= 60;
    let hour = (secs % 24) as u32;
    secs /= 24;
    // `secs` is now days since 1970-01-01 (Thursday).
    let mut days = secs as u32;
    let mut year: u32 = 1970;
    loop {
        let len = days_in_year(year);
        if days < len {
            break;
        }
        days -= len;
        year += 1;
    }
    let mut month: u32 = 1;
    loop {
        let len = days_in_month(year, month);
        if days < len {
            break;
        }
        days -= len;
        month += 1;
    }
    let day = days + 1;
    (year, month, day, hour, minute, second)
}

fn is_leap_year(y: u32) -> bool {
    (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400)
}

fn days_in_year(y: u32) -> u32 {
    if is_leap_year(y) { 366 } else { 365 }
}

fn days_in_month(y: u32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(y) {
                29
            } else {
                28
            }
        }
        _ => unreachable!("month out of range"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_basename_strips_directory_and_extension() {
        assert_eq!(
            derive_basename("001529/files/50685_ruby_67979/50685_ruby_67979.txt"),
            "50685_ruby_67979"
        );
    }

    #[test]
    fn derive_basename_handles_no_extension() {
        assert_eq!(derive_basename("foo/bar"), "bar");
    }

    #[test]
    fn secs_to_utc_unix_epoch_is_1970_01_01() {
        assert_eq!(secs_to_utc(0), (1970, 1, 1, 0, 0, 0));
    }

    #[test]
    fn secs_to_utc_handles_leap_year_crossing() {
        // 2020-02-29 12:00:00 UTC = 1582977600
        assert_eq!(secs_to_utc(1_582_977_600), (2020, 2, 29, 12, 0, 0));
    }

    #[test]
    fn secs_to_utc_handles_recent_date() {
        // 2026-01-01 00:00:00 UTC = 1767225600
        assert_eq!(secs_to_utc(1_767_225_600), (2026, 1, 1, 0, 0, 0));
    }

    #[test]
    fn paranoid_blocked_message_lists_three_remedies() {
        let msg = format_paranoid_blocked(2);
        // The message MUST tell the user what the problem is and
        // give them at least a one-shot fix + a permanent fix +
        // an explanation of *why* this is needed.
        assert!(
            msg.contains("perf_event_paranoid = 2"),
            "missing observed value: {msg}"
        );
        assert!(msg.contains("One-shot fix"));
        assert!(msg.contains("Permanent fix"));
        assert!(msg.contains("/etc/sysctl.d/99-perf.conf"));
        assert!(msg.contains("Why this is required"));
        assert!(msg.contains("perf_event_open(2)"));
        assert!(msg.contains("Security note"));
    }
}
