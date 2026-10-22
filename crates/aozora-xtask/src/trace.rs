use std::path::{Path, PathBuf};
use std::time::Instant;

use clap::{Args, Subcommand, ValueEnum};

use aozora_trace::{
    Categorizer, RollupConfig, SymbolCache, Symbolicator, TableRenderable, Trace, analysis,
};

#[derive(Args)]
pub(crate) struct TraceArgs {
    /// Report format for commands other than `cache`.
    #[arg(long, value_enum, global = true, default_value_t = TraceFormat::Human)]
    pub(crate) format: TraceFormat,
    #[command(subcommand)]
    pub(crate) cmd: TraceCmd,
}

#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub(crate) enum TraceFormat {
    /// Text tables, or folded stacks for `flame`.
    #[default]
    Human,
    /// Pretty-printed JSON of the typed report.
    Json,
}

#[derive(Subcommand)]
pub(crate) enum TraceCmd {
    /// Resolve trace symbols with a binary and write a sidecar cache.
    Cache {
        trace: PathBuf,
        /// Binary containing debug information for address resolution.
        binary: PathBuf,
        /// Library name in the trace's `libs` array that the binary corresponds to.
        /// Defaults to the binary file stem.
        #[arg(long)]
        lib_name: Option<String>,
    },
    /// Show inclusive and leaf percentages for hot frames.
    Hot {
        trace: PathBuf,
        /// Number of rows to print.
        #[arg(long, default_value_t = 25)]
        top: usize,
        /// Count distinct frames throughout each stack instead of leaves alone.
        #[arg(long)]
        inclusive: bool,
        /// Optional binary for fresh symbolication when the cache is missing or stale.
        #[arg(long)]
        binary: Option<PathBuf>,
    },
    /// Library distribution.
    Libs { trace: PathBuf },
    /// Rollup function names into named categories.
    Rollup {
        trace: PathBuf,
        /// Category definitions in TOML, defaulting to built-in categories.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Optional binary for fresh symbolication when the cache is missing.
        #[arg(long)]
        binary: Option<PathBuf>,
    },
    /// Print top-K call stacks where any frame matches `pattern`.
    Stacks {
        trace: PathBuf,
        /// Regular expression to match frame labels.
        #[arg(long)]
        pattern: String,
        /// Limit on the number of distinct stacks to print.
        #[arg(long, default_value_t = 5)]
        limit: usize,
        #[arg(long)]
        binary: Option<PathBuf>,
    },
    /// Compare two traces, surfacing functions that grew, shrank, appeared, or disappeared.
    Compare {
        before: PathBuf,
        after: PathBuf,
        #[arg(long, default_value_t = 25)]
        top: usize,
        /// Binary shared by both recorded traces.
        #[arg(long)]
        binary: Option<PathBuf>,
    },
    /// Emit folded stacks for `flamegraph.pl`.
    Flame {
        trace: PathBuf,
        #[arg(long)]
        binary: Option<PathBuf>,
    },
}

#[expect(
    clippy::print_stdout,
    reason = "the command reports results and diagnostics to developers"
)]
pub(crate) fn dispatch(args: TraceArgs) -> Result<(), String> {
    let format = args.format;
    match args.cmd {
        TraceCmd::Cache {
            trace,
            binary,
            lib_name,
        } => cmd_cache(&trace, &binary, lib_name.as_deref()),
        TraceCmd::Hot {
            trace,
            top,
            inclusive,
            binary,
        } => emit(&cmd_hot(&trace, top, inclusive, binary.as_deref())?, format),
        TraceCmd::Libs { trace } => emit(&cmd_libs(&trace)?, format),
        TraceCmd::Rollup {
            trace,
            config,
            binary,
        } => emit(
            &cmd_rollup(&trace, config.as_deref(), binary.as_deref())?,
            format,
        ),
        TraceCmd::Stacks {
            trace,
            pattern,
            limit,
            binary,
        } => emit(
            &cmd_stacks(&trace, &pattern, limit, binary.as_deref())?,
            format,
        ),
        TraceCmd::Compare {
            before,
            after,
            top,
            binary,
        } => emit(
            &cmd_compare(&before, &after, top, binary.as_deref())?,
            format,
        ),
        TraceCmd::Flame { trace, binary } => {
            let folded = cmd_flame(&trace, binary.as_deref())?;
            match format {
                TraceFormat::Human => print!("{}", analysis::render_folded(&folded)),
                TraceFormat::Json => {
                    let json = serde_json::to_string_pretty(&folded).map_err(|e| e.to_string())?;
                    println!("{json}");
                }
            }
            Ok(())
        }
    }
}

#[expect(
    clippy::print_stderr,
    reason = "the command reports results and diagnostics to developers"
)]
fn cmd_cache(trace_path: &Path, binary: &Path, lib_name: Option<&str>) -> Result<(), String> {
    let started = Instant::now();
    let mut trace = Trace::load(trace_path).map_err(|e| e.to_string())?;
    let lib_name = lib_name.map_or_else(|| derive_lib_name(binary), str::to_owned);

    let mut sym = Symbolicator::new();
    sym.add_binary(&lib_name, binary)
        .map_err(|e| e.to_string())?;
    sym.verify_against(&trace).map_err(|e| e.to_string())?;

    for lib in &trace.libs {
        if lib.name == lib_name {
            continue;
        }
        let path = Path::new(&lib.path);
        if path.exists() {
            match sym.add_binary_dynamic_only(&lib.name, path) {
                Ok(n) if n > 0 => {
                    eprintln!(
                        "[symbolicator] {}: {n} dynamic symbols loaded (.dynsym fallback)",
                        lib.name
                    );
                }
                _ => {}
            }
        }
    }

    let mut cache = SymbolCache::default();
    let (resolved, attempted) = sym.resolve_into(&mut trace, &mut cache);
    let cache_path = SymbolCache::sidecar_path_for(trace_path);
    cache.write(&cache_path).map_err(|e| e.to_string())?;
    eprintln!(
        "resolved {resolved}/{attempted} frames; wrote sidecar {} in {:?}",
        cache_path.display(),
        started.elapsed()
    );
    Ok(())
}

fn cmd_hot(
    trace_path: &Path,
    top: usize,
    inclusive: bool,
    binary: Option<&Path>,
) -> Result<analysis::HotReport, String> {
    let trace = load_with_symbols(trace_path, binary)?;
    Ok(if inclusive {
        analysis::hot_inclusive(&trace, top)
    } else {
        analysis::hot_leaves(&trace, top)
    })
}

fn cmd_libs(trace_path: &Path) -> Result<analysis::LibraryReport, String> {
    let trace = Trace::load(trace_path).map_err(|e| e.to_string())?;
    Ok(analysis::library_distribution(&trace))
}

fn cmd_rollup(
    trace_path: &Path,
    config: Option<&Path>,
    binary: Option<&Path>,
) -> Result<analysis::RollupReport, String> {
    let trace = load_with_symbols(trace_path, binary)?;
    let cfg = match config {
        Some(p) => RollupConfig::from_toml_file(p).map_err(|e| e.to_string())?,
        None => RollupConfig::aozora_defaults(),
    };
    let categorizer: Categorizer = cfg.compile().map_err(|e| e.to_string())?;
    Ok(analysis::rollup(&trace, &categorizer))
}

fn cmd_stacks(
    trace_path: &Path,
    pattern: &str,
    limit: usize,
    binary: Option<&Path>,
) -> Result<analysis::MatchedStacksReport, String> {
    let trace = load_with_symbols(trace_path, binary)?;
    let regex = regex::Regex::new(pattern).map_err(|e| format!("bad regex: {e}"))?;
    Ok(analysis::matching_stacks(&trace, &regex, limit))
}

fn cmd_compare(
    before: &Path,
    after: &Path,
    top: usize,
    binary: Option<&Path>,
) -> Result<analysis::ComparisonReport, String> {
    let b = load_with_symbols(before, binary)?;
    let a = load_with_symbols(after, binary)?;
    Ok(analysis::compare(&b, &a, top))
}

fn cmd_flame(
    trace_path: &Path,
    binary: Option<&Path>,
) -> Result<Vec<analysis::FoldedStack>, String> {
    let trace = load_with_symbols(trace_path, binary)?;
    Ok(analysis::folded_stacks(&trace))
}

#[expect(
    clippy::print_stdout,
    reason = "the command reports results and diagnostics to developers"
)]
fn emit<R: TableRenderable + serde::Serialize>(
    report: &R,
    format: TraceFormat,
) -> Result<(), String> {
    match format {
        TraceFormat::Human => println!("{}", report.render_table()),
        TraceFormat::Json => {
            let json = serde_json::to_string_pretty(report).map_err(|e| e.to_string())?;
            println!("{json}");
        }
    }
    Ok(())
}

#[expect(
    clippy::print_stderr,
    reason = "the command reports results and diagnostics to developers"
)]
fn load_with_symbols(trace_path: &Path, binary: Option<&Path>) -> Result<Trace, String> {
    let mut trace = Trace::load(trace_path).map_err(|e| e.to_string())?;
    let cache_path = SymbolCache::sidecar_path_for(trace_path);
    let cached = SymbolCache::load(&cache_path).map_err(|e| e.to_string())?;
    let mut applied = 0;
    if let Some(c) = cached {
        applied = c.apply(&mut trace);
        eprintln!(
            "[cache] applied {applied} resolved labels from {}",
            cache_path.display()
        );
    }
    let unresolved = trace
        .threads
        .iter()
        .map(|t| t.resolved.iter().filter(|r| r.is_none()).count())
        .sum::<usize>();
    if unresolved > 0 {
        if let Some(b) = binary {
            let lib_name = derive_lib_name(b);
            let mut sym = Symbolicator::new();
            sym.add_binary(&lib_name, b).map_err(|e| e.to_string())?;
            sym.verify_against(&trace).map_err(|e| e.to_string())?;
            let mut cache = SymbolCache::default();
            let (resolved, _) = sym.resolve_into(&mut trace, &mut cache);
            if let Some(mut on_disk) = SymbolCache::load(&cache_path).map_err(|e| e.to_string())? {
                for (name, lib) in cache.libs {
                    let entry = on_disk.libs.entry(name).or_default();
                    for (k, v) in lib.by_address {
                        entry.by_address.insert(k, v);
                    }
                    entry.debug_id = lib.debug_id;
                }
                on_disk.write(&cache_path).map_err(|e| e.to_string())?;
            } else {
                cache.write(&cache_path).map_err(|e| e.to_string())?;
            }
            eprintln!(
                "[symbol] resolved {resolved} additional frames via DWARF, updated {}",
                cache_path.display()
            );
        } else if applied == 0 {
            eprintln!(
                "[warn] no symbol cache and no --binary specified; report will show hex addresses for {unresolved} frames"
            );
        }
    }
    Ok(trace)
}

fn derive_lib_name(binary: &Path) -> String {
    binary
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("binary")
        .to_owned()
}
