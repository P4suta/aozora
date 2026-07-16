//! Per-file work: read, format (panic-guarded), and the in-place write with
//! the idempotency guard.

use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind, set_hook, take_hook};
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use aozora::render::SerializeOptions;

use crate::encoding::{self, Encoding};
use crate::format_source_with;
use crate::source;

/// A formatted file: the original decoded source and the canonical form.
#[derive(Debug)]
pub struct Formatted {
    /// The decoded source, before canonicalisation.
    pub old: String,
    /// The canonical form.
    pub new: String,
}

impl Formatted {
    /// True when canonicalisation changed the source.
    #[must_use]
    pub fn changed(&self) -> bool {
        self.old != self.new
    }
}

/// Read `path` under `encoding` and canonicalise it (panic-guarded).
///
/// Shared verbatim with the `aozora` CLI's `lint --fix`, so its source rewrite
/// is byte-identical to `fmt --fix --write`.
///
/// # Errors
///
/// Returns an error if `path` cannot be read, its bytes do not decode under
/// `encoding`, or the formatter panics on the input.
pub fn read_and_format(
    path: &Path,
    opts: SerializeOptions,
    encoding: Encoding,
) -> Result<Formatted> {
    let raw = source::read_file(path)?;
    let old =
        encoding::decode(&raw, encoding).with_context(|| format!("decoding {}", path.display()))?;
    let new = format_guarded(&old, opts)?;
    Ok(Formatted { old, new })
}

/// Rewrite `path` with its canonical form if it changed, upholding the
/// formatter's idempotency contract: refuse to write when a second pass
/// differs rather than corrupt the file.
///
/// Shared with the `aozora` CLI's `lint --fix` (see [`read_and_format`]).
///
/// # Errors
///
/// Returns an error (leaving the file untouched) if the canonical form is not
/// idempotent — a second pass differs — or if the write fails.
pub fn write_back(path: &Path, fmt: &Formatted, opts: SerializeOptions) -> Result<()> {
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
    fs::write(path, &fmt.new).with_context(|| format!("writing {}", path.display()))
}

/// Marker error returned by [`guard`] when the wrapped closure panicked.
///
/// Callers turn this into a domain-specific message (the formatter says "no
/// files were modified"; the CLI's `render` says "no output was produced").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Panicked;

/// Run `f`, turning an upstream `aozora` parser panic into a typed [`Panicked`].
///
/// `catch_unwind` only intercepts the panic under `panic = "unwind"` — the
/// dev/test default and `aozora-lsp`'s dist profile. The shipping release binary
/// sets `panic = "abort"`, so there a parser panic aborts the process before the
/// catch runs, exactly as the sibling `source` module documents. The default panic hook is
/// silenced for the duration so a caught panic doesn't also print "thread 'main'
/// panicked …"; the caller reports it instead.
///
/// # Errors
///
/// Returns [`Panicked`] if `f` unwinds from a panic.
pub fn guard<T>(f: impl FnOnce() -> T) -> Result<T, Panicked> {
    let prev_hook = take_hook();
    set_hook(Box::new(|_| {}));
    let result = catch_unwind(AssertUnwindSafe(f));
    set_hook(prev_hook);
    result.map_err(|_| Panicked)
}

/// Format `source` under [`guard`]. In `--write` mode a caught panic surfaces
/// as an error before any write, so no file is touched (where [`guard`] can
/// catch — see its note on `panic = "unwind"` vs `"abort"`).
pub(crate) fn format_guarded(source: &str, opts: SerializeOptions) -> Result<String> {
    guard(|| format_source_with(source, opts)).map_err(|_| {
        anyhow!(
            "the formatter panicked while processing this input; no files were \
             modified. This is a bug — please report it at \
             https://github.com/P4suta/aozora/issues"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::env;
    use std::path::PathBuf;
    use std::process;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A unique scratch path under the OS temp dir. A per-test counter
    /// keeps parallel test threads from colliding on the same file.
    fn scratch(name: &str) -> PathBuf {
        static SEQ: AtomicU32 = AtomicU32::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let mut path = env::temp_dir();
        path.push(format!("aozora-fmt-process-{}-{n}-{name}", process::id()));
        path
    }

    #[test]
    fn format_guarded_canonicalises_ruby() {
        // A redundant explicit ｜ canonicalises to the bare ruby form.
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
    fn formatted_changed_reflects_difference() {
        let same = Formatted {
            old: "x".to_owned(),
            new: "x".to_owned(),
        };
        let diff = Formatted {
            old: "x".to_owned(),
            new: "y".to_owned(),
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
    fn write_back_noop_leaves_file_untouched_and_uncreated() {
        let path = scratch("noop.afm");
        let fmt = Formatted {
            old: "same".to_owned(),
            new: "same".to_owned(),
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

    /// The idempotency guard is the formatter's anti-corruption seatbelt:
    /// if a (hypothetically non-idempotent) canonical form does not survive
    /// a second pass, `write_back` must refuse to write rather than persist
    /// a form it can't reproduce. We simulate that by handing it a
    /// `Formatted` whose `new` is deliberately *not* canonical, so the
    /// second pass differs.
    #[test]
    fn write_back_refuses_non_idempotent_output_and_preserves_file() {
        let path = scratch("guard.afm");
        fs::write(&path, "original").expect("seed file");
        let fmt = Formatted {
            old: "original".to_owned(),
            // Non-canonical on purpose: a redundant ｜ canonicalises away,
            // so format_guarded(new) != new.
            new: "｜日本《にほん》".to_owned(),
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
