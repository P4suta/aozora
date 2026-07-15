//! A hand-rolled `which`: locate an executable on `PATH` (or in a single
//! directory) without pulling in a dependency for one small probe.
//!
//! Two consumers share it: `doctor` reports whether `pandoc` / `aozora-lsp`
//! are reachable, and `lsp` resolves the `aozora-lsp` daemon it delegates to
//! (on `PATH`, else beside the running executable). Keeping the search in one
//! place means both agree on what "executable" means — the exec-bit check on
//! Unix and the `PATHEXT` expansion on Windows.

use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Find `program` on the process `PATH`, returning the first executable
/// candidate — the convenience wrapper over [`find_on_path`] that reads the
/// live `PATH`.
// mutants::skip — a thin adapter over the live process `PATH`: it only reads
// `PATH` and hands off to [`find_on_path`], which carries the swept search
// assertions. A live-`PATH` read cannot be exercised deterministically without
// mutating the global process environment — ruled out here by
// `#![forbid(unsafe_code)]` (env mutation is `unsafe` on edition 2024).
#[cfg_attr(test, mutants::skip)]
pub(crate) fn which(program: &str) -> Option<PathBuf> {
    find_on_path(&env::var_os("PATH")?, program)
}

/// The pure `PATH` search behind [`which`], taking the raw `PATH` value so it
/// is unit-testable without mutating the process environment. Uses
/// [`env::split_paths`] for the platform's separator, skips empty entries, and
/// delegates each directory to [`find_in_dir`].
pub(crate) fn find_on_path(paths: &OsStr, program: &str) -> Option<PathBuf> {
    env::split_paths(paths)
        .filter(|dir| !dir.as_os_str().is_empty())
        .find_map(|dir| find_in_dir(&dir, program))
}

/// Find `program` inside a single `dir`, returning the first executable
/// candidate. Split out so a caller with a specific directory in hand (the
/// `lsp` delegate's "next to this binary" fallback) can search it directly,
/// with the same exec-bit / `PATHEXT` semantics the `PATH` walk uses.
pub(crate) fn find_in_dir(dir: &Path, program: &str) -> Option<PathBuf> {
    executable_candidates(dir, program)
        .into_iter()
        .find(|candidate| is_executable_file(candidate))
}

// mutants::skip — the Windows PATHEXT expansion is cfg-dead on the Linux
// sweep host, so cargo-mutants cannot exercise it here; the non-Windows
// counterpart below carries the swept assertions. Reinforcing this variant
// would need a separate Windows mutation pass.
#[cfg_attr(test, mutants::skip)]
#[cfg(windows)]
fn executable_candidates(dir: &Path, program: &str) -> Vec<PathBuf> {
    // The bare name (for an already-suffixed program), then each PATHEXT entry.
    let mut candidates = vec![dir.join(program)];
    let pathext = env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned());
    for ext in pathext.split(';').filter(|ext| !ext.is_empty()) {
        candidates.push(dir.join(format!("{program}{ext}")));
    }
    candidates
}

#[cfg(not(windows))]
fn executable_candidates(dir: &Path, program: &str) -> Vec<PathBuf> {
    vec![dir.join(program)]
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    path.metadata()
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

// mutants::skip — the non-unix fallback is cfg-dead on the Linux sweep host;
// the `#[cfg(unix)]` variant above carries the swept assertions. Reinforcing
// this variant would need a separate non-unix mutation pass.
#[cfg_attr(test, mutants::skip)]
#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(windows))]
    #[test]
    fn executable_candidates_is_the_bare_join_off_windows() {
        let got = executable_candidates(Path::new("/bin"), "pandoc");
        assert_eq!(
            got,
            vec![PathBuf::from("/bin/pandoc")],
            "one bare candidate"
        );
    }

    #[cfg(unix)]
    #[test]
    fn find_on_path_finds_the_first_executable_and_skips_the_rest() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt as _;

        let empty = tempfile::TempDir::new().expect("empty dir");
        let with_tool = tempfile::TempDir::new().expect("tool dir");
        let tool = with_tool.path().join("mytool");
        fs::write(&tool, b"#!/bin/sh\n").expect("write tool");
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).expect("chmod +x");
        // A non-executable file of the same name in an earlier dir must be
        // skipped, so the search reaches the real executable.
        let shadow = empty.path().join("mytool");
        fs::write(&shadow, b"not runnable").expect("write shadow");
        fs::set_permissions(&shadow, fs::Permissions::from_mode(0o644)).expect("chmod -x");

        // PATH = "<empty-dir>::<tool-dir>" — note the empty middle entry, which
        // must be skipped rather than joined into "/mytool".
        let path =
            env::join_paths([empty.path(), Path::new(""), with_tool.path()]).expect("join PATH");
        assert_eq!(
            find_on_path(&path, "mytool").as_deref(),
            Some(tool.as_path()),
            "finds the executable past the non-executable shadow and the empty entry"
        );
        assert_eq!(
            find_on_path(&path, "nope"),
            None,
            "an absent program is None"
        );
    }

    #[cfg(unix)]
    #[test]
    fn find_in_dir_locates_an_executable_and_ignores_a_plain_file() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::TempDir::new().expect("dir");
        // No such program yet: None.
        assert_eq!(find_in_dir(dir.path(), "daemon"), None, "absent -> None");

        // A plain (non-executable) file is not a match.
        let plain = dir.path().join("daemon");
        fs::write(&plain, b"").expect("write");
        fs::set_permissions(&plain, fs::Permissions::from_mode(0o644)).expect("chmod -x");
        assert_eq!(
            find_in_dir(dir.path(), "daemon"),
            None,
            "a non-executable file is not a candidate"
        );

        // Flip the exec bit and it resolves.
        fs::set_permissions(&plain, fs::Permissions::from_mode(0o755)).expect("chmod +x");
        assert_eq!(
            find_in_dir(dir.path(), "daemon").as_deref(),
            Some(plain.as_path()),
            "an executable file in the dir resolves"
        );
    }

    #[cfg(unix)]
    #[test]
    fn is_executable_file_needs_a_file_with_an_exec_bit() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::TempDir::new().expect("dir");
        let exec = dir.path().join("run");
        fs::write(&exec, b"").expect("write");
        fs::set_permissions(&exec, fs::Permissions::from_mode(0o755)).expect("chmod");
        assert!(is_executable_file(&exec), "0o755 file is executable");

        let plain = dir.path().join("data");
        fs::write(&plain, b"").expect("write");
        fs::set_permissions(&plain, fs::Permissions::from_mode(0o644)).expect("chmod");
        assert!(!is_executable_file(&plain), "0o644 file is not executable");

        // A directory is never an executable *file*, even with the exec bit.
        assert!(!is_executable_file(dir.path()), "a directory is not a file");
    }
}
