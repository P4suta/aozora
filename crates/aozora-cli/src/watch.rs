//! `--watch`: re-run a document subcommand when its input file changes.
//!
//! Foreground and non-daemon (ADR-0014). The loop runs the command once
//! up front, then again on every change to the input, debounced. Each
//! run's exit code is swallowed — a diagnostic or `fmt --check` mismatch
//! prints but does not stop the watch — so the loop ends only on Ctrl-C
//! (default SIGINT termination).
//!
//! The input's *parent directory* is watched, not the file itself:
//! editors commonly save by writing a temp file and renaming it over the
//! target, which a direct file watch misses. Events are filtered to the
//! target by file name.

use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result};
use notify::{Event, RecursiveMode, Watcher};

/// Coalesce a burst of save events (an editor's temp-write + rename, or a
/// formatter's own write-back) into a single re-run.
const DEBOUNCE: Duration = Duration::from_millis(75);

/// Run `once` immediately, then re-run it on every change to `path`
/// until interrupted. Always returns `ExitCode::SUCCESS`: per-run exit
/// codes are reported but not propagated — watching is the point.
pub(crate) fn watch(path: &Path, once: impl Fn() -> Result<ExitCode>) -> Result<ExitCode> {
    rerun(&once);
    banner(path);

    let parent = watched_dir(path);
    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
        if let Ok(event) = res {
            // The receiver lives as long as the watcher; a send error
            // only happens during shutdown, which we ignore.
            let _drop = tx.send(event);
        }
    })
    .context("failed to start the file watcher")?;
    watcher
        .watch(parent, RecursiveMode::NonRecursive)
        .with_context(|| format!("failed to watch {}", parent.display()))?;

    while let Ok(event) = rx.recv() {
        if !touches(&event, path) {
            continue;
        }
        // Drain the debounce window so a rename+write burst is one re-run.
        while rx.recv_timeout(DEBOUNCE).is_ok() {}
        rerun(&once);
        banner(path);
    }
    Ok(ExitCode::SUCCESS)
}

/// The directory to watch: the file's parent, or `.` for a bare filename.
fn watched_dir(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

/// Does `event` concern the target file? Matched by file name so an
/// atomic save (rename of a temp over the target) still counts.
fn touches(event: &Event, path: &Path) -> bool {
    let name = path.file_name();
    event.paths.iter().any(|p| p.file_name() == name)
}

/// Run the command once, printing any error but swallowing the exit code
/// so the watch keeps going.
fn rerun(once: &impl Fn() -> Result<ExitCode>) {
    if let Err(err) = once() {
        let _drop = writeln!(io::stderr(), "aozora: {err:#}");
    }
}

/// A between-runs banner, TTY only so piped output stays clean.
fn banner(path: &Path) {
    let mut stderr = io::stderr().lock();
    if stderr.is_terminal() {
        let _drop = writeln!(stderr, "── watching {} (Ctrl-C to stop) ──", path.display());
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use notify::EventKind;

    use super::*;

    #[test]
    fn bare_filename_watches_cwd() {
        assert_eq!(watched_dir(Path::new("file.txt")), Path::new("."));
    }

    #[test]
    fn nested_path_watches_its_parent() {
        assert_eq!(watched_dir(Path::new("a/b/file.txt")), Path::new("a/b"));
    }

    #[test]
    fn touches_matches_by_file_name() {
        let event = Event::new(EventKind::Any).add_path(PathBuf::from("/tmp/x/file.txt"));
        // Same name in any directory counts — atomic saves rename a temp
        // over the target, so directory equality would miss them.
        assert!(touches(&event, Path::new("file.txt")));
        assert!(touches(&event, Path::new("elsewhere/file.txt")));
        // A different file in the watched directory does not.
        assert!(!touches(&event, Path::new("other.txt")));
    }
}
