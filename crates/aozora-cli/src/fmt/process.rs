//! Reads, formats, and replaces files with panic and idempotency guards.

use std::cell::Cell;
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind, set_hook, take_hook};
use std::path::{Path, PathBuf};
use std::sync::{Once, OnceLock};

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

use anyhow::{Context, Result, anyhow, bail};
use aozora::SerializeOptions;
use rayon::prelude::*;
use rayon::{ThreadPool, ThreadPoolBuilder};

use crate::atomic_write;
use crate::fmt::encoding::{self, Encoding, FileEncoding};
use crate::fmt::source;
use aozora::fmt::format_source_with;

const MAX_BATCH_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
const PARALLEL_FILE_COUNT_THRESHOLD: usize = 4;
const FORMAT_PANIC_MESSAGE: &str = "the formatter panicked while processing this input; no files were \
    modified. This is a bug — please report it at \
    https://github.com/P4suta/aozora/issues";

#[derive(Debug)]
pub(crate) struct Formatted {
    pub old: String,
    pub new: String,
    pub diagnostics: Vec<aozora::Diagnostic>,
    encoding: FileEncoding,
}

impl Formatted {
    #[must_use]
    pub(crate) fn changed(&self) -> bool {
        self.old != self.new
    }
}

pub(crate) fn read_and_format(
    path: &Path,
    opts: SerializeOptions,
    encoding: Encoding,
) -> Result<Formatted> {
    let raw = source::read_file(path)?;
    let decoded = encoding::decode_with_encoding(&raw, encoding)
        .with_context(|| format!("decoding {}", path.display()))?;
    let (new, diagnostics) = format_with_diagnostics(&decoded.text, opts)?;
    Ok(Formatted {
        old: decoded.text,
        new,
        diagnostics,
        encoding: decoded.encoding,
    })
}

pub(crate) fn read_and_format_batch(
    paths: &[PathBuf],
    opts: SerializeOptions,
    encoding: Encoding,
) -> Vec<Result<Formatted>> {
    collect_batch(paths, |path| read_and_format(path, opts, encoding))
}

fn collect_batch<T: Send>(paths: &[PathBuf], process: impl Fn(&Path) -> T + Send + Sync) -> Vec<T> {
    if paths.len() < PARALLEL_FILE_COUNT_THRESHOLD {
        return paths.iter().map(|path| process(path.as_path())).collect();
    }
    let Some(pool) = batch_pool() else {
        return paths.iter().map(|path| process(path.as_path())).collect();
    };
    pool.install(|| {
        paths
            .par_iter()
            .map(|path| process(path.as_path()))
            .collect()
    })
}

fn batch_width() -> usize {
    batch_pool()
        .map_or(1, ThreadPool::current_num_threads)
        .saturating_mul(2)
}

pub(crate) fn batch_len(paths: &[PathBuf]) -> usize {
    let mut bytes = 0_u64;
    let mut len = 0;
    for path in paths.iter().take(batch_width()) {
        let source_bytes = fs::metadata(path).map_or(0, |metadata| metadata.len());
        if len != 0 && bytes.saturating_add(source_bytes) > MAX_BATCH_SOURCE_BYTES {
            break;
        }
        bytes = bytes.saturating_add(source_bytes);
        len += 1;
    }
    len
}

fn batch_pool() -> Option<&'static ThreadPool> {
    static POOL: OnceLock<Option<ThreadPool>> = OnceLock::new();
    POOL.get_or_init(|| {
        ThreadPoolBuilder::new()
            .num_threads(num_cpus::get_physical().max(1))
            .thread_name(|index| format!("aozora-fmt-{index}"))
            .build()
            .ok()
    })
    .as_ref()
}

/// Refuses a non-idempotent result before replacing the file.
pub(crate) fn write_back(path: &Path, fmt: &Formatted, opts: SerializeOptions) -> Result<()> {
    if !fmt.changed() {
        return Ok(());
    }
    let reformatted = format_guarded(&fmt.new, opts)?;
    if reformatted != fmt.new {
        bail!(
            "refusing to overwrite {}: formatting is not idempotent for this \
             input (a second pass changes the output). This is a bug — please \
             report it. The file was left unchanged.",
            path.display()
        );
    }
    let encoded = encoding::encode(&fmt.new, fmt.encoding)
        .with_context(|| format!("encoding {}", path.display()))?;
    atomic_write::replace(path, &encoded).with_context(|| format!("writing {}", path.display()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Panicked;

/// Catches unwinding panics and silences their hook output, but aborting panics still stop the process.
pub(super) fn guard<T>(f: impl FnOnce() -> T) -> Result<T, Panicked> {
    install_panic_hook();
    SUPPRESS_PANIC_HOOK.with(|depth| depth.set(depth.get().saturating_add(1)));
    let result = catch_unwind(AssertUnwindSafe(f));
    SUPPRESS_PANIC_HOOK.with(|depth| depth.set(depth.get().saturating_sub(1)));
    result.map_err(|_| Panicked)
}

thread_local! {
    static SUPPRESS_PANIC_HOOK: Cell<u32> = const { Cell::new(0) };
}

#[cfg(test)]
static SUPPRESSED_PANIC_HOOKS: AtomicUsize = AtomicUsize::new(0);

fn install_panic_hook() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let previous = take_hook();
        set_hook(Box::new(move |info| {
            let suppressed = SUPPRESS_PANIC_HOOK
                .try_with(|depth| depth.get() != 0)
                .unwrap_or(false);
            if suppressed {
                #[cfg(test)]
                SUPPRESSED_PANIC_HOOKS.fetch_add(1, AtomicOrdering::Relaxed);
            } else {
                previous(info);
            }
        }));
    });
}

pub(crate) fn format_guarded(source: &str, opts: SerializeOptions) -> Result<String> {
    guard(|| format_source_with(source, opts)).map_err(|_| anyhow!(FORMAT_PANIC_MESSAGE))
}

pub(crate) fn format_with_diagnostics(
    source: &str,
    opts: SerializeOptions,
) -> Result<(String, Vec<aozora::Diagnostic>)> {
    guard(|| {
        let document = aozora::parse(source.to_owned())?;
        let snapshot = document.snapshot();
        let diagnostics = snapshot.diagnostics().to_vec();
        Ok::<_, aozora::ParseError>((snapshot.to_source_with(opts), diagnostics))
    })
    .map_err(|_| anyhow!(FORMAT_PANIC_MESSAGE))?
    .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::env;
    use std::path::PathBuf;
    use std::process;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::thread;

    fn scratch(name: &str) -> PathBuf {
        static SEQ: AtomicU32 = AtomicU32::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let mut path = env::temp_dir();
        path.push(format!("aozora-fmt-process-{}-{n}-{name}", process::id()));
        path
    }

    #[test]
    fn format_guarded_canonicalises_ruby() {
        let out =
            format_guarded("｜日本《にほん》", SerializeOptions::default()).expect("format ok");
        assert_eq!(out, "日本《にほん》", "bare canonical expected: {out:?}");
    }

    #[test]
    fn guard_returns_value_for_non_panicking_closure() {
        assert_eq!(guard(|| 2 + 2).expect("no panic"), 4);
    }

    #[test]
    fn guard_converts_panic_into_marker_error() {
        let result = guard(|| -> i32 { panic!("boom") });
        assert_eq!(result, Err(Panicked));
    }

    #[test]
    fn guard_suppresses_the_panic_hook() {
        let before = SUPPRESSED_PANIC_HOOKS.load(AtomicOrdering::Relaxed);
        assert_eq!(guard(|| panic!("expected")), Err(Panicked));
        assert!(SUPPRESSED_PANIC_HOOKS.load(AtomicOrdering::Relaxed) > before);
    }

    #[test]
    fn formatted_changed_reflects_difference() {
        let same = Formatted {
            old: "x".to_owned(),
            new: "x".to_owned(),
            diagnostics: Vec::new(),
            encoding: FileEncoding::Utf8,
        };
        let diff = Formatted {
            old: "x".to_owned(),
            new: "y".to_owned(),
            diagnostics: Vec::new(),
            encoding: FileEncoding::Utf8,
        };
        assert!(!same.changed());
        assert!(diff.changed());
    }

    #[test]
    fn read_and_format_reads_then_canonicalises() {
        let path = scratch("read.afm");
        fs::write(&path, "｜日本《にほん》").expect("seed file");
        let fmt = read_and_format(&path, SerializeOptions::default(), Encoding::Auto)
            .expect("read+format");
        assert_eq!(fmt.old, "｜日本《にほん》");
        assert_eq!(fmt.new, "日本《にほん》");
        assert!(fmt.changed());
        fs::remove_file(&path).ok();
    }

    #[test]
    fn read_and_format_errors_on_missing_file() {
        let path = scratch("missing.afm");
        let err = read_and_format(&path, SerializeOptions::default(), Encoding::Auto)
            .expect_err("missing file must error");
        assert!(
            err.to_string().contains("reading"),
            "error should name the read step: {err:#}",
        );
    }

    #[test]
    fn batch_format_preserves_input_order() {
        let paths = (0..8)
            .map(|index| {
                let path = scratch(&format!("batch-{index}.aozora"));
                fs::write(&path, format!("body-{index}")).expect("seed file");
                path
            })
            .collect::<Vec<_>>();
        let results = read_and_format_batch(&paths, SerializeOptions::default(), Encoding::Auto);
        assert_eq!(results.len(), paths.len());
        for (index, result) in results.into_iter().enumerate() {
            let formatted = result.expect("batch item");
            assert_eq!(formatted.old, format!("body-{index}"));
            fs::remove_file(&paths[index]).ok();
        }
    }

    #[test]
    fn batch_parallelism_starts_at_four_inputs() {
        let paths = (0..4).map(|_| PathBuf::new()).collect::<Vec<_>>();
        let worker_name = |_: &Path| thread::current().name().unwrap_or_default().to_owned();
        let serial = collect_batch(&paths[..3], worker_name);
        assert!(serial.iter().all(|name| !name.starts_with("aozora-fmt-")));
        let parallel = collect_batch(&paths, worker_name);
        assert!(parallel.iter().all(|name| name.starts_with("aozora-fmt-")));
    }

    #[test]
    fn batch_width_keeps_two_items_per_worker() {
        assert_eq!(
            batch_width(),
            batch_pool()
                .map_or(1, ThreadPool::current_num_threads)
                .saturating_mul(2)
        );
        assert!(batch_width() >= 2);
    }

    #[test]
    fn batch_len_respects_count_and_source_byte_limits() {
        assert_eq!(MAX_BATCH_SOURCE_BYTES, 67_108_864);

        let small = (0..=batch_width())
            .map(|index| scratch(&format!("missing-{index}.aozora")))
            .collect::<Vec<_>>();
        assert_eq!(batch_len(&small), batch_width());

        let exact = [scratch("exact-a.aozora"), scratch("exact-b.aozora")];
        for path in &exact {
            fs::File::create(path)
                .and_then(|file| file.set_len(MAX_BATCH_SOURCE_BYTES / 2))
                .expect("create sparse exact-size file");
        }
        assert_eq!(batch_len(&exact), 2);

        let over = [scratch("over-a.aozora"), scratch("over-b.aozora")];
        for path in &over {
            fs::File::create(path)
                .and_then(|file| file.set_len(MAX_BATCH_SOURCE_BYTES / 2 + 1))
                .expect("create sparse over-size file");
        }
        assert_eq!(batch_len(&over), 1);

        for path in exact.iter().chain(&over) {
            fs::remove_file(path).ok();
        }
    }

    #[test]
    fn guard_is_thread_local() {
        let threads = (0..8)
            .map(|_| thread::spawn(|| guard(|| panic!("expected"))))
            .collect::<Vec<_>>();
        for thread in threads {
            assert_eq!(thread.join().expect("thread"), Err(Panicked));
        }
    }

    #[test]
    fn write_back_noop_leaves_file_untouched_and_uncreated() {
        let path = scratch("noop.afm");
        let fmt = Formatted {
            old: "same".to_owned(),
            new: "same".to_owned(),
            diagnostics: Vec::new(),
            encoding: FileEncoding::Utf8,
        };
        write_back(&path, &fmt, SerializeOptions::default()).expect("noop write_back");
        assert!(
            !path.exists(),
            "unchanged formatting must not create or touch the file",
        );
    }

    #[test]
    fn write_back_rewrites_when_changed() {
        let path = scratch("write.afm");
        fs::write(&path, "｜日本《にほん》").expect("seed file");
        let fmt = read_and_format(&path, SerializeOptions::default(), Encoding::Auto)
            .expect("read+format");
        write_back(&path, &fmt, SerializeOptions::default()).expect("write_back");
        let written = fs::read_to_string(&path).expect("read back");
        assert_eq!(written, fmt.new);
        assert_eq!(written, "日本《にほん》");
        fs::remove_file(&path).ok();
    }

    #[test]
    fn write_back_preserves_shift_jis_for_detected_and_explicit_input() {
        let (raw, _, had_errors) = encoding_rs::SHIFT_JIS.encode("｜日本《にほん》");
        assert!(!had_errors);
        let (expected, _, had_errors) = encoding_rs::SHIFT_JIS.encode("日本《にほん》");
        assert!(!had_errors);

        for selected in [Encoding::Auto, Encoding::Sjis] {
            let path = scratch("write-sjis.afm");
            fs::write(&path, raw.as_ref()).expect("seed file");
            let fmt =
                read_and_format(&path, SerializeOptions::default(), selected).expect("read+format");
            write_back(&path, &fmt, SerializeOptions::default()).expect("write_back");
            assert_eq!(fs::read(&path).expect("read back"), expected.as_ref());
            fs::remove_file(&path).ok();
        }
    }

    #[test]
    fn write_back_rejects_unrepresentable_shift_jis_before_writing() {
        let path = scratch("unrepresentable-sjis.afm");
        fs::write(&path, "original").expect("seed file");
        let fmt = Formatted {
            old: "original".to_owned(),
            new: "\u{1f980}".to_owned(),
            diagnostics: Vec::new(),
            encoding: FileEncoding::ShiftJis,
        };
        let err = write_back(&path, &fmt, SerializeOptions::default()).unwrap_err();
        assert!(
            err.to_string().contains(&path.display().to_string()),
            "encoding error must identify its input: {err:#}",
        );
        assert_eq!(fs::read(&path).expect("read back"), b"original");
        fs::remove_file(&path).ok();
    }

    #[test]
    fn write_back_refuses_non_idempotent_output_and_preserves_file() {
        let path = scratch("guard.afm");
        fs::write(&path, "original").expect("seed file");
        let fmt = Formatted {
            old: "original".to_owned(),
            new: "｜日本《にほん》".to_owned(),
            diagnostics: Vec::new(),
            encoding: FileEncoding::Utf8,
        };
        let err = write_back(&path, &fmt, SerializeOptions::default())
            .expect_err("non-idempotent output must be refused");
        assert!(
            err.to_string().contains("idempotent"),
            "guard message should mention idempotency: {err:#}",
        );
        assert_eq!(
            fs::read_to_string(&path).expect("read back"),
            "original",
            "the original file must be left byte-for-byte intact",
        );
        fs::remove_file(&path).ok();
    }
}
