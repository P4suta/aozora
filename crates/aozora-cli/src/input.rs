//! Input-source guards for the document subcommands.
//!
//! A document subcommand with no path argument reads stdin. On a pipe or a
//! redirect that is exactly right; but on a *bare interactive terminal* it
//! silently blocks forever waiting for a human to type — the single worst
//! first-run papercut of the CLI. [`guard_stdin`] detects that case up front
//! and turns it into an actionable usage error (exit 2) instead of a hang.
//!
//! Only an interactive TTY trips the guard. Piped or redirected stdin
//! (`cat f | aozora check`, `aozora check < f`, an empty pipe) is not a
//! terminal, so the guard stands aside and the normal stdin path runs
//! unchanged — empty piped input still parses to an empty document (exit 0).

use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::process::ExitCode;

/// The pure decision behind [`stdin_is_interactive`], split out so both
/// outcomes are unit-testable without a real TTY: it is `true` only when
/// `path` is the stdin sentinel `-` *and* `is_terminal` says stdin is a
/// terminal. `stdin_is_interactive` is the thin wrapper that supplies the
/// live terminal probe.
fn is_interactive_decision(path: &Path, is_terminal: bool) -> bool {
    path.as_os_str() == "-" && is_terminal
}

/// True when `path` is the stdin sentinel `-` *and* stdin is an interactive
/// terminal — i.e. reading it would block on human input. Piped or
/// redirected stdin is not a TTY, so this is `false` there.
fn stdin_is_interactive(path: &Path) -> bool {
    is_interactive_decision(path, io::stdin().is_terminal())
}

/// Write the four-line "empty stdin on a terminal" hint for `cmd` to `w`.
///
/// `cmd` is the subcommand tag shown in the copy-pasteable examples, e.g.
/// `check` or `inspect nodes`. The alignment is tuned for fullwidth CJK
/// glyphs in a monospace terminal; keep it in sync with the unit test.
fn write_stdin_hint(w: &mut impl Write, cmd: &str) -> io::Result<()> {
    writeln!(w, "error: 標準入力が空です (端末から実行中)")?;
    writeln!(w, "  ヒント: ファイルを →  aozora {cmd} <FILE>")?;
    writeln!(w, "          パイプで   →  cat f.txt | aozora {cmd}")?;
    writeln!(w, "  全機能:  aozora --help")
}

/// Guard a document subcommand against hanging on an interactive terminal
/// with no input.
///
/// Returns `Some(ExitCode::from(2))` — a usage error, mirroring the
/// `--watch`-on-stdin sibling — after printing [`write_stdin_hint`] to
/// stderr when stdin is an interactive TTY; otherwise `None`, meaning
/// "proceed, the input source is a file or a pipe".
///
/// The exit code must be returned through the caller's normal `Ok(..)` path
/// (not `?`), so it stays `2` rather than collapsing to the generic `1` the
/// `main` error handler assigns.
pub(crate) fn guard_stdin(path: &Path, cmd: &str) -> Option<ExitCode> {
    if !stdin_is_interactive(path) {
        return None;
    }
    let mut stderr = io::stderr().lock();
    let _drop = write_stdin_hint(&mut stderr, cmd);
    Some(ExitCode::from(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdin_hint_bytes_are_exact() {
        let mut buf = Vec::new();
        write_stdin_hint(&mut buf, "check").expect("writing to a Vec cannot fail");
        let expected = concat!(
            "error: 標準入力が空です (端末から実行中)\n",
            "  ヒント: ファイルを →  aozora check <FILE>\n",
            "          パイプで   →  cat f.txt | aozora check\n",
            "  全機能:  aozora --help\n",
        );
        assert_eq!(String::from_utf8(buf).unwrap(), expected);
    }

    #[test]
    fn decision_is_true_only_for_dash_sentinel_on_a_terminal() {
        // The `-` sentinel on a terminal is the one interactive case: this is
        // the outcome a real TTY would produce and the only value that kills a
        // `-> false` whole-body mutation of the decision.
        assert!(
            is_interactive_decision(Path::new("-"), true),
            "`-` on a terminal must be interactive"
        );
    }

    #[test]
    fn decision_is_false_off_a_terminal_even_for_the_sentinel() {
        // Same `-` sentinel, non-terminal stdin: the `&& is_terminal` conjunct
        // must gate the result to false (a pipe or redirect).
        assert!(
            !is_interactive_decision(Path::new("-"), false),
            "`-` off a terminal must not be interactive"
        );
    }

    #[test]
    fn decision_is_false_for_a_file_path_on_a_terminal() {
        // A non-`-` path even on a terminal is false. With `==` mutated to
        // `!=`, `"file.txt" != "-"` would be true and, combined under `&&`
        // with the terminal flag, would wrongly report interactive — so this
        // pins the `==`.
        assert!(
            !is_interactive_decision(Path::new("some/file.txt"), true),
            "a file path is never the stdin sentinel"
        );
    }

    #[test]
    fn a_real_path_is_never_interactive() {
        // A file path is not the `-` sentinel, so the guard must always
        // stand aside regardless of whether the test harness's stdin is a
        // terminal (under `cargo test` it never is).
        assert!(
            !stdin_is_interactive(Path::new("some/file.txt")),
            "a file path must not be treated as interactive stdin"
        );
        assert!(
            guard_stdin(Path::new("some/file.txt"), "check").is_none(),
            "a file path must never trip the stdin guard"
        );
    }

    #[test]
    fn piped_stdin_sentinel_is_not_interactive() {
        // The harness runs with a non-terminal stdin, so even the `-`
        // sentinel resolves to non-interactive here — mirroring a pipe or a
        // redirect, where the guard must stand aside.
        assert!(
            !stdin_is_interactive(Path::new("-")),
            "non-terminal stdin must not be interactive"
        );
        assert!(
            guard_stdin(Path::new("-"), "check").is_none(),
            "non-terminal `-` must not trip the guard"
        );
    }
}
