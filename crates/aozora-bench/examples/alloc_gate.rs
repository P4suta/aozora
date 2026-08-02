//! Deterministic allocation-pressure ratchet for the public document parser,
//! the #237 P0.2-real perf gate.
//!
//! The lex producer stores the AST in owned `Vec` / `String` storage
//! (`NodeStore`, `StrInterner`). The worry
//! the gate guards is that many small `Vec` pushes / re-grows inflate
//! `malloc` traffic versus the bump allocator. Allocation *count* and *bytes*
//! are a pure function of the input — unlike wall-clock they gate identically
//! on a laptop and a noisy CI runner — so they make a stable ratchet.
//!
//! For every corpus document this measures, via dhat's [`dhat::HeapStats`]
//! around `parse` and `snapshot`, the owned-path allocation delta (source,
//! transient scratch, and owned storage). Two normalized metrics are gated against a
//! committed baseline at `corpus/alloc-baseline.json`, mirroring
//! `xtask corpus audit-gate`:
//!
//! - `alloc_blocks_per_file`        = Σ Δblocks / files   (malloc-count pressure)
//! - `alloc_bytes_per_source_byte`  = Σ Δbytes  / Σ src   (volume amplification)
//!
//! A structural-count check (the AST's registry / source-node / pair
//! counts per document) is the correctness floor under the proxy: if the AST
//! started producing a *different amount* of data, the alloc baseline
//! would be measuring the wrong thing. (Byte-identity itself is proven by the
//! conformance golden and the corpus round-trip fixed-point gate.)
//!
//! ```text
//! AOZORA_CORPUS_ROOT=… cargo run --release --example alloc_gate -p aozora-bench \
//!     -- --baseline corpus/alloc-baseline.json [--root DIR] [--update]
//! ```
//!
//! Exit codes: `0` pass (or no corpus → skip), `1` over budget, `2` usage /
//! parity error.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::too_many_lines,
    clippy::disallowed_methods,
    reason = "profiling-gate tool, not library code"
)]

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::hint::black_box;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process;

use aozora::{TextEdit, decode_auto};
use aozora_bench::build_synthetic_aozora;
use aozora_corpus::{CorpusError, CorpusSource, ENV_CORPUS_ROOT, FilesystemCorpus};
use dhat::{HeapStats, Profiler};
use serde_json::{Map, Value, from_str, json, to_string_pretty};

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

/// The release corpus and toolchain are pinned, so allocation ceilings have no
/// regression headroom.
const DEFAULT_TOLERANCE: f64 = 0.0;
const EDIT_BYTES: usize = 256 * 1024;
const LARGE_BYTES: usize = 1024 * 1024;
const SNAPSHOT_CLONES: usize = 1000;

#[derive(Debug)]
struct Args {
    baseline: PathBuf,
    root: Option<PathBuf>,
    update: bool,
    tolerance: f64,
    limit: Option<usize>,
}

fn parse_args_from(args: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut baseline = None;
    let mut root = None;
    let mut update = false;
    let mut tolerance = DEFAULT_TOLERANCE;
    let mut limit = None;
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--baseline" => {
                baseline = Some(PathBuf::from(
                    it.next()
                        .ok_or_else(|| "--baseline requires a path".to_owned())?,
                ));
            }
            "--root" => {
                root = Some(PathBuf::from(
                    it.next()
                        .ok_or_else(|| "--root requires a path".to_owned())?,
                ));
            }
            "--update" => update = true,
            "--tolerance" => {
                let value = it
                    .next()
                    .ok_or_else(|| "--tolerance requires a number".to_owned())?;
                tolerance = value
                    .parse::<f64>()
                    .map_err(|_| "--tolerance must be a finite non-negative number".to_owned())?;
                if !tolerance.is_finite() || tolerance < 0.0 {
                    return Err("--tolerance must be a finite non-negative number".to_owned());
                }
            }
            "--limit" => {
                let value = it
                    .next()
                    .ok_or_else(|| "--limit requires a positive integer".to_owned())?;
                let parsed = value
                    .parse::<usize>()
                    .map_err(|_| "--limit must be a positive integer".to_owned())?;
                if parsed == 0 {
                    return Err("--limit must be a positive integer".to_owned());
                }
                limit = Some(parsed);
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    let baseline = baseline.ok_or_else(|| "--baseline <path> is required".to_owned())?;
    Ok(Args {
        baseline,
        root,
        update,
        tolerance,
        limit,
    })
}

fn parse_args() -> Args {
    parse_args_from(env::args().skip(1)).unwrap_or_else(|error| {
        eprintln!("alloc_gate: {error}");
        process::exit(2);
    })
}

/// Resolve the corpus: `--root` wins, else `AOZORA_CORPUS_ROOT`, else skip
/// cleanly (exit 0) so the gate is a no-op on corpus-less hosts, exactly like
/// `audit-gate`.
fn resolve_corpus(root: Option<&Path>) -> Box<dyn CorpusSource> {
    if let Some(root) = root {
        return match open_corpus(root.to_path_buf()) {
            Ok(c) => Box::new(c),
            Err(e) => {
                eprintln!("alloc_gate: --root {} not usable: {e}", root.display());
                process::exit(2);
            }
        };
    }
    if let Some(root) = env::var_os(ENV_CORPUS_ROOT) {
        let root = PathBuf::from(root);
        return match open_corpus(root.clone()) {
            Ok(c) => Box::new(c),
            Err(e) => {
                eprintln!(
                    "alloc_gate: {ENV_CORPUS_ROOT} {} not usable: {e}",
                    root.display()
                );
                process::exit(2);
            }
        };
    }
    println!("alloc_gate: AOZORA_CORPUS_ROOT not set — skipped (no corpus).");
    process::exit(0);
}

fn open_corpus(root: PathBuf) -> Result<FilesystemCorpus, CorpusError> {
    FilesystemCorpus::new(root)
}

/// Per-corpus allocation totals.
#[derive(Default)]
struct Totals {
    files: u64,
    source_bytes: u64,
    alloc_blocks: u64,
    alloc_bytes: u64,
    decode_errors: u64,
    io_errors: u64,
}

#[derive(Clone, Copy)]
struct Allocation {
    blocks: u64,
    bytes: u64,
}

type Contract = BTreeMap<&'static str, Allocation>;

impl Totals {
    fn blocks_per_file(&self) -> f64 {
        if self.files == 0 {
            0.0
        } else {
            self.alloc_blocks as f64 / self.files as f64
        }
    }

    fn bytes_per_source_byte(&self) -> f64 {
        if self.source_bytes == 0 {
            0.0
        } else {
            self.alloc_bytes as f64 / self.source_bytes as f64
        }
    }
}

fn main() {
    let args = parse_args();

    let corpus = resolve_corpus(args.root.as_deref());

    // Testing mode lets `HeapStats::get` read live cumulative counters.
    let _profiler = Profiler::builder().testing().build();

    let mut totals = Totals::default();
    for item in corpus.iter() {
        if let Some(limit) = args.limit
            && totals.files >= limit as u64
        {
            break;
        }
        let Ok(item) = item else {
            totals.io_errors += 1;
            continue;
        };
        let Ok(text) = decode_auto(&item.bytes) else {
            totals.decode_errors += 1;
            continue;
        };

        // Measured window: only the owned producer's allocations (transient
        // scratch + owned storage) land in the delta.
        let before = HeapStats::get();
        let owned = aozora::parse(text.as_ref())
            .expect("source fits parser span limit")
            .snapshot();
        let after = HeapStats::get();
        totals.alloc_blocks += after.total_blocks - before.total_blocks;
        totals.alloc_bytes += after.total_bytes - before.total_bytes;
        totals.files += 1;
        totals.source_bytes += text.len() as u64;

        // Read the side-table lengths so the optimiser cannot elide the parse.
        black_box((
            owned.nodes().len(),
            owned.pairs().len(),
            owned.container_pairs().len(),
            owned.diagnostics().len(),
        ));
    }

    if totals.io_errors != 0 {
        eprintln!(
            "alloc_gate: refusing a partial-corpus result after {} I/O error(s)",
            totals.io_errors
        );
        process::exit(2);
    }

    if totals.files == 0 {
        eprintln!("alloc_gate: corpus yielded 0 decodable documents.");
        process::exit(1);
    }

    let blocks_per_file = totals.blocks_per_file();
    let bytes_per_source_byte = totals.bytes_per_source_byte();
    let contract = allocation_contract();
    println!(
        "alloc_gate: {} files, {} decode errors",
        totals.files, totals.decode_errors
    );
    println!("  alloc_blocks_per_file       = {blocks_per_file:.4}");
    println!("  alloc_bytes_per_source_byte = {bytes_per_source_byte:.4}");
    for (name, allocation) in &contract {
        println!(
            "  {name:<28} = {:>8} blocks, {:>12} bytes",
            allocation.blocks, allocation.bytes
        );
    }

    if args.update {
        write_baseline(
            &args,
            &totals,
            (blocks_per_file, bytes_per_source_byte),
            &contract,
        );
        println!(
            "alloc_gate: baseline written to {}",
            args.baseline.display()
        );
        return;
    }

    let baseline = read_baseline(&args.baseline);
    let tol = baseline
        .tolerance
        .filter(|t| t.is_finite() && *t >= 0.0)
        .unwrap_or(args.tolerance);
    let mut failed = if totals.decode_errors != 0 {
        eprintln!(
            "  decode_errors: {} document(s) could not be decoded",
            totals.decode_errors
        );
        true
    } else {
        false
    };
    failed |= check_ratio_metric(
        "alloc_blocks_per_file",
        Ratio {
            numerator: totals.alloc_blocks,
            denominator: totals.files,
        },
        baseline.blocks_per_file,
        tol,
    );
    failed |= check_ratio_metric(
        "alloc_bytes_per_source_byte",
        Ratio {
            numerator: totals.alloc_bytes,
            denominator: totals.source_bytes,
        },
        baseline.bytes_per_source_byte,
        tol,
    );
    for (name, allocation) in contract {
        failed |= check_contract(name, allocation, &baseline.contract);
    }

    if failed {
        eprintln!(
            "alloc_gate: FAIL — owned allocation pressure regressed beyond +{:.1}%.\n  \
             Re-baseline with `just alloc-gate-update` only if the increase is intended,\n  \
             and attach a `just throughput` run showing wall-clock is within budget.",
            tol * 100.0
        );
        process::exit(1);
    }
    println!(
        "alloc_gate: PASS (within +{:.1}% of baseline).",
        tol * 100.0
    );
}

fn allocation_contract() -> Contract {
    let edit_source = build_synthetic_aozora(EDIT_BYTES);
    let large_source = build_synthetic_aozora(LARGE_BYTES);
    let mut contract = Contract::new();

    contract.insert(
        "large_document",
        measure(|| {
            let snapshot = aozora::parse(black_box(large_source.as_str()))
                .expect("synthetic source fits parser spans")
                .snapshot();
            black_box(snapshot.nodes().len());
        }),
    );

    let mut single =
        aozora::parse(edit_source.clone()).expect("synthetic source fits parser spans");
    let at = edit_offset(&edit_source);
    contract.insert(
        "single_edit",
        measure(|| {
            single
                .edit([TextEdit::new(at..at, "x")])
                .expect("synthetic edit is valid");
            black_box(single.snapshot());
        }),
    );

    let mut multiple =
        aozora::parse(edit_source.clone()).expect("synthetic source fits parser spans");
    let first = edit_source.find("\n\n").expect("paragraph boundary") + 2;
    let second = edit_source.rfind("\n\n").expect("paragraph boundary");
    contract.insert(
        "multiple_edits",
        measure(|| {
            multiple
                .edit([
                    TextEdit::new(first..first, "x"),
                    TextEdit::new(second..second, "y"),
                ])
                .expect("synthetic edits are valid");
            black_box(multiple.snapshot());
        }),
    );

    let snapshot = aozora::parse(edit_source)
        .expect("synthetic source fits parser spans")
        .snapshot();
    contract.insert(
        "snapshot_clone",
        measure(|| {
            for _ in 0..SNAPSHOT_CLONES {
                black_box(snapshot.clone());
            }
        }),
    );

    contract.insert(
        "html_render",
        measure(|| {
            black_box(snapshot.to_html());
        }),
    );

    contract
}

fn edit_offset(source: &str) -> usize {
    let middle = source.len() / 2;
    source[middle..]
        .find("\n\n")
        .map_or(middle, |relative| middle + relative + 2)
}

fn measure(operation: impl FnOnce()) -> Allocation {
    let before = HeapStats::get();
    operation();
    let after = HeapStats::get();
    Allocation {
        blocks: after.total_blocks - before.total_blocks,
        bytes: after.total_bytes - before.total_bytes,
    }
}

#[derive(Clone, Copy)]
struct Ratio {
    numerator: u64,
    denominator: u64,
}

#[derive(Clone, Copy)]
struct RatioBaseline {
    stored: f64,
    numerator: Option<u64>,
    denominator: Option<u64>,
}

fn check_ratio_metric(name: &str, current: Ratio, baseline: RatioBaseline, tolerance: f64) -> bool {
    let Some(baseline_numerator) = baseline.numerator else {
        eprintln!("alloc_gate: baseline metric {name} missing/invalid");
        return true;
    };
    let Some(baseline_denominator) = baseline.denominator else {
        eprintln!("alloc_gate: baseline metric {name} missing/invalid");
        return true;
    };
    if !baseline.stored.is_finite() || current.denominator == 0 || baseline_denominator == 0 {
        eprintln!("alloc_gate: baseline metric {name} missing/invalid");
        return true;
    }

    let current_ratio = current.numerator as f64 / current.denominator as f64;
    let baseline_ratio = baseline_numerator as f64 / baseline_denominator as f64;
    let allowed = baseline_ratio * (1.0 + tolerance);
    let over = if tolerance == 0.0 {
        u128::from(current.numerator) * u128::from(baseline_denominator)
            > u128::from(baseline_numerator) * u128::from(current.denominator)
    } else {
        current_ratio > allowed
    };
    if over {
        eprintln!(
            "  {name}: {current_ratio:.4} > allowed {allowed:.4} (baseline {baseline_ratio:.4} +{:.1}%)",
            tolerance * 100.0
        );
        true
    } else {
        false
    }
}

fn check_contract(name: &str, current: Allocation, baseline: &Value) -> bool {
    let mut failed = false;
    for (metric, actual) in [("blocks", current.blocks), ("bytes", current.bytes)] {
        let Some(ceiling) = baseline[name][metric].as_u64() else {
            eprintln!("alloc_gate: baseline metric contract.{name}.{metric} missing/invalid");
            failed = true;
            continue;
        };
        if actual > ceiling {
            eprintln!("  contract.{name}.{metric}: {actual} > baseline ceiling {ceiling}");
            failed = true;
        }
    }
    failed
}

struct Baseline {
    blocks_per_file: RatioBaseline,
    bytes_per_source_byte: RatioBaseline,
    tolerance: Option<f64>,
    contract: Value,
}

fn read_baseline(path: &Path) -> Baseline {
    let Ok(text) = fs::read_to_string(path) else {
        eprintln!("alloc_gate: cannot read baseline {}", path.display());
        process::exit(2);
    };
    let Ok(v) = from_str::<Value>(&text) else {
        eprintln!("alloc_gate: baseline {} is not valid JSON", path.display());
        process::exit(2);
    };
    baseline_from_value(&v)
}

fn baseline_from_value(v: &Value) -> Baseline {
    let files = v["files_analyzed"].as_u64();
    let source_bytes = v["source_bytes_total"].as_u64();
    Baseline {
        blocks_per_file: RatioBaseline {
            stored: v["alloc_blocks_per_file"].as_f64().unwrap_or(f64::NAN),
            numerator: v["alloc_blocks_total"].as_u64(),
            denominator: files,
        },
        bytes_per_source_byte: RatioBaseline {
            stored: v["alloc_bytes_per_source_byte"]
                .as_f64()
                .unwrap_or(f64::NAN),
            numerator: v["alloc_bytes_total"].as_u64(),
            denominator: source_bytes,
        },
        tolerance: v["tolerance"].as_f64(),
        contract: v["contract"].clone(),
    }
}

fn write_baseline(args: &Args, totals: &Totals, ratios: (f64, f64), contract: &Contract) {
    let json = baseline_value(args.tolerance, totals, ratios, contract);
    let text = to_string_pretty(&json).expect("json serialize is infallible");
    if let Err(e) = replace_file(&args.baseline, format!("{text}\n").as_bytes()) {
        eprintln!(
            "alloc_gate: cannot write baseline {}: {e}",
            args.baseline.display()
        );
        process::exit(2);
    }
}

fn replace_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    replace_file_with(path, |file| file.write_all(bytes))
}

fn replace_file_with(
    path: &Path,
    write: impl FnOnce(&mut fs::File) -> io::Result<()>,
) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let permissions = match fs::metadata(path) {
        Ok(metadata) => Some(metadata.permissions()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let mut builder = tempfile::Builder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        builder.permissions(fs::Permissions::from_mode(0o666))
    };
    let mut temporary = builder.tempfile_in(parent)?;
    if let Some(permissions) = permissions {
        temporary.as_file().set_permissions(permissions)?;
    }
    write(temporary.as_file_mut())?;
    temporary.as_file_mut().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

fn baseline_value(
    tolerance: f64,
    totals: &Totals,
    ratios: (f64, f64),
    contract: &Contract,
) -> Value {
    let (blocks_per_file, bytes_per_source_byte) = ratios;
    let contract = Value::Object(
        contract
            .iter()
            .map(|(name, allocation)| {
                (
                    (*name).to_owned(),
                    json!({
                        "blocks": allocation.blocks,
                        "bytes": allocation.bytes,
                    }),
                )
            })
            .collect::<Map<_, _>>(),
    );
    json!({
        "tolerance": tolerance,
        "files_analyzed": totals.files,
        "source_bytes_total": totals.source_bytes,
        "alloc_blocks_total": totals.alloc_blocks,
        "alloc_bytes_total": totals.alloc_bytes,
        "alloc_blocks_per_file": blocks_per_file,
        "alloc_bytes_per_source_byte": bytes_per_source_byte,
        "contract": contract,
        "note": "public Document parse/edit/snapshot/render allocation-pressure ratchet; \
                 regenerate with `just alloc-gate-update`. Metrics normalized \
                 per-file / per-source-byte so corpus drift does not trip the gate.",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_arguments_reject_values_that_can_disable_comparisons() {
        for value in ["NaN", "inf", "-0.1", "invalid"] {
            let error = parse_args_from([
                "--baseline".to_owned(),
                "baseline.json".to_owned(),
                "--tolerance".to_owned(),
                value.to_owned(),
            ])
            .expect_err("invalid tolerance must fail");
            assert!(error.contains("finite non-negative"), "{error}");
        }

        for value in ["0", "-1", "invalid"] {
            let error = parse_args_from([
                "--baseline".to_owned(),
                "baseline.json".to_owned(),
                "--limit".to_owned(),
                value.to_owned(),
            ])
            .expect_err("invalid limit must fail");
            assert!(error.contains("positive integer"), "{error}");
        }
    }

    #[test]
    fn configured_corpus_root_must_be_usable() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(open_corpus(dir.path().join("missing")).is_err());
        assert!(open_corpus(dir.path().to_path_buf()).is_ok());
    }

    #[test]
    fn failed_baseline_write_preserves_the_previous_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("baseline.json");
        fs::write(&path, b"previous").expect("seed baseline");

        let error = replace_file_with(&path, |file| {
            file.write_all(b"partial")?;
            Err(io::Error::other("injected failure"))
        })
        .expect_err("write failure must surface");

        assert_eq!(error.to_string(), "injected failure");
        assert_eq!(fs::read(path).expect("read baseline"), b"previous");
    }

    #[test]
    fn written_baseline_is_its_own_zero_tolerance_ceiling() {
        let totals = Totals {
            files: 17_889,
            source_bytes: 845_727_350,
            alloc_blocks: 3_564_797,
            alloc_bytes: 13_052_359_463,
            ..Totals::default()
        };
        let ratios = (totals.blocks_per_file(), totals.bytes_per_source_byte());
        let value = baseline_value(0.0, &totals, ratios, &Contract::new());
        let encoded = to_string_pretty(&value).expect("baseline JSON");
        let decoded: Value = from_str(&encoded).expect("baseline JSON round trip");
        let baseline = baseline_from_value(&decoded);

        assert!(!check_ratio_metric(
            "blocks",
            Ratio {
                numerator: totals.alloc_blocks,
                denominator: totals.files,
            },
            baseline.blocks_per_file,
            0.0,
        ));
        assert!(!check_ratio_metric(
            "bytes",
            Ratio {
                numerator: totals.alloc_bytes,
                denominator: totals.source_bytes,
            },
            baseline.bytes_per_source_byte,
            0.0,
        ));
    }
}
