// ADR-0014 watches the parent directory because editors save by renaming a temporary file over the target.

use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::i18n::{self as i18n, FluentArgs, LanguageIdentifier};
use anyhow::{Context, Result, bail};
use notify::{Event, RecursiveMode, Watcher};
use tracing::{debug, trace};

const DEBOUNCE: Duration = Duration::from_millis(75);

pub(crate) fn watch(
    path: &Path,
    lang: &LanguageIdentifier,
    once: impl Fn() -> Result<ExitCode>,
) -> Result<ExitCode> {
    rerun(&once);
    banner(path, lang);

    let parent = watched_dir(path);
    debug!(target = %path.display(), watch_dir = %parent.display(), "watch: monitoring parent directory for changes");
    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
        let _drop = tx.send(res);
    })
    .context("failed to start the file watcher")?;
    watcher
        .watch(parent, RecursiveMode::NonRecursive)
        .with_context(|| format!("failed to watch {}", parent.display()))?;

    loop {
        wait_for_change(&rx, path, DEBOUNCE)?;
        rerun(&once);
        banner(path, lang);
    }
}

fn wait_for_change(
    rx: &mpsc::Receiver<notify::Result<Event>>,
    path: &Path,
    debounce: Duration,
) -> Result<()> {
    loop {
        let event = match rx.recv() {
            Ok(Ok(event)) => event,
            Ok(Err(err)) => return Err(err).context("file watcher backend failed"),
            Err(_) => bail!("file watcher event channel disconnected"),
        };
        if should_skip(&event, path) {
            trace!(kind = ?event.kind, "watch: skipping fs event that does not touch the target");
            continue;
        }
        debug!(kind = ?event.kind, "watch: target changed; draining the debounce window");
        let mut deadline = debounce_deadline(Instant::now(), debounce);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(remaining) {
                Ok(Ok(event)) => {
                    let now = Instant::now();
                    if !should_skip(&event, path) {
                        deadline = debounce_deadline(now, debounce);
                    } else if now >= deadline {
                        return Ok(());
                    }
                }
                Ok(Err(err)) => return Err(err).context("file watcher backend failed"),
                Err(mpsc::RecvTimeoutError::Timeout) => return Ok(()),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    bail!("file watcher event channel disconnected")
                }
            }
        }
    }
}

fn debounce_deadline(now: Instant, debounce: Duration) -> Instant {
    now + debounce
}

fn watched_dir(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

fn touches(event: &Event, path: &Path) -> bool {
    let name = path.file_name();
    event.paths.iter().any(|p| p.file_name() == name)
}

fn should_skip(event: &Event, path: &Path) -> bool {
    !touches(event, path)
}

fn rerun(once: &impl Fn() -> Result<ExitCode>) {
    if let Err(err) = once() {
        let _drop = writeln!(io::stderr(), "aozora: {err:#}");
    }
}

fn banner(path: &Path, lang: &LanguageIdentifier) {
    let mut stderr = io::stderr().lock();
    let is_tty = stderr.is_terminal();
    let _drop = write_banner(&mut stderr, is_tty, path, lang);
}

fn write_banner(
    out: &mut impl Write,
    is_terminal: bool,
    path: &Path,
    lang: &LanguageIdentifier,
) -> io::Result<()> {
    if is_terminal {
        let mut args = FluentArgs::new();
        args.set("path", path.display().to_string());
        writeln!(out, "{}", i18n::tf(lang, "watch-banner", &args))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::thread;

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
        assert!(touches(&event, Path::new("file.txt")));
        assert!(touches(&event, Path::new("elsewhere/file.txt")));
        assert!(!touches(&event, Path::new("other.txt")));
    }

    #[test]
    fn should_skip_drops_unrelated_events() {
        let event = Event::new(EventKind::Any).add_path(PathBuf::from("/tmp/x/other.txt"));
        assert!(should_skip(&event, Path::new("file.txt")));
    }

    #[test]
    fn should_skip_keeps_matching_events() {
        let event = Event::new(EventKind::Any).add_path(PathBuf::from("/tmp/x/file.txt"));
        assert!(!should_skip(&event, Path::new("file.txt")));
    }

    #[test]
    fn wait_for_change_propagates_backend_errors() {
        let (tx, rx) = mpsc::channel();
        tx.send(Err(notify::Error::generic("backend stopped")))
            .expect("send error");
        let err = wait_for_change(&rx, Path::new("file.txt"), Duration::ZERO)
            .expect_err("backend error must stop watch");
        assert!(err.to_string().contains("file watcher backend failed"));
    }

    #[test]
    fn wait_for_change_propagates_disconnect() {
        let (tx, rx) = mpsc::channel();
        drop(tx);
        let err = wait_for_change(&rx, Path::new("file.txt"), Duration::ZERO)
            .expect_err("disconnect must stop watch");
        assert!(err.to_string().contains("disconnected"));
    }

    #[test]
    fn wait_for_change_skips_unrelated_events() {
        let (tx, rx) = mpsc::channel();
        tx.send(Ok(Event::new(EventKind::Any).add_path("other.txt".into())))
            .expect("send unrelated event");
        tx.send(Ok(Event::new(EventKind::Any).add_path("file.txt".into())))
            .expect("send matching event");
        wait_for_change(&rx, Path::new("file.txt"), Duration::ZERO).expect("matching change");
    }

    #[test]
    fn wait_for_change_propagates_an_error_during_debounce() {
        let (tx, rx) = mpsc::channel();
        tx.send(Ok(Event::new(EventKind::Any).add_path("file.txt".into())))
            .expect("send matching event");
        tx.send(Err(notify::Error::generic("backend stopped")))
            .expect("send error");
        let err = wait_for_change(&rx, Path::new("file.txt"), Duration::ZERO)
            .expect_err("debounce error must stop watch");
        assert!(err.to_string().contains("file watcher backend failed"));
    }

    #[test]
    fn unrelated_event_inside_debounce_does_not_finish_the_batch() {
        let (tx, rx) = mpsc::channel();
        tx.send(Ok(Event::new(EventKind::Any).add_path("file.txt".into())))
            .expect("send matching event");
        tx.send(Ok(Event::new(EventKind::Any).add_path("other.txt".into())))
            .expect("send unrelated event");
        drop(tx);

        let err = wait_for_change(&rx, Path::new("file.txt"), Duration::from_secs(1))
            .expect_err("an unrelated event must not hide a disconnected watcher");

        assert!(err.to_string().contains("disconnected"));
    }

    #[test]
    fn unrelated_events_do_not_extend_the_debounce_window() {
        let (tx, rx) = mpsc::channel();
        tx.send(Ok(Event::new(EventKind::Any).add_path("file.txt".into())))
            .expect("send matching event");
        let sender = thread::spawn(move || {
            for _ in 0..20 {
                thread::sleep(Duration::from_millis(2));
                if tx
                    .send(Ok(Event::new(EventKind::Any).add_path("other.txt".into())))
                    .is_err()
                {
                    break;
                }
            }
        });
        let started = Instant::now();

        wait_for_change(&rx, Path::new("file.txt"), Duration::from_millis(10))
            .expect("debounce completes");

        assert!(started.elapsed() < Duration::from_millis(30));
        sender.join().expect("sender");
    }

    #[test]
    fn debounce_deadline_advances_by_the_full_window() {
        let now = Instant::now();
        let debounce = Duration::from_millis(75);

        assert_eq!(
            debounce_deadline(now, debounce).duration_since(now),
            debounce
        );
    }

    #[test]
    fn rerun_invokes_the_command_exactly_once() {
        use std::cell::Cell;
        let calls = Cell::new(0);
        rerun(&|| {
            calls.set(calls.get() + 1);
            Ok(ExitCode::SUCCESS)
        });
        assert_eq!(calls.get(), 1);
    }

    fn lang(tag: &str) -> LanguageIdentifier {
        tag.parse().expect("test locale tag parses")
    }

    fn banner_line(tag: &str) -> String {
        let mut out = Vec::new();
        write_banner(&mut out, true, Path::new("doc.txt"), &lang(tag)).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn write_banner_emits_the_path_on_a_terminal_in_english_by_default() {
        assert_eq!(
            banner_line("en"),
            "── watching doc.txt (Ctrl-C to stop) ──\n"
        );
    }

    #[test]
    fn write_banner_localizes_the_path_line() {
        assert_eq!(banner_line("ja"), "── 監視中 doc.txt（Ctrl-C で終了）──\n");
        assert_eq!(banner_line("zh"), "── 正在监视 doc.txt（Ctrl-C 停止）──\n");
    }

    #[test]
    fn write_banner_is_silent_off_a_terminal() {
        let mut out = Vec::new();
        write_banner(&mut out, false, Path::new("doc.txt"), &lang("en")).unwrap();
        assert_eq!(out, Vec::<u8>::new());
    }
}
