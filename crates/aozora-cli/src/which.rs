//! A hand-rolled `which`: locate an executable on `PATH` (or in a single
//! directory) without pulling in a dependency for one small probe.
//!
//! `doctor` uses it to report whether `pandoc` is reachable on `PATH` — the
//! exec-bit check on Unix and the `PATHEXT` expansion on Windows.

use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Find `program` on the process `PATH`, returning the first executable
/// candidate — the convenience wrapper over [`find_on_path`] that reads the
/// live `PATH`.
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

fn executable_candidates(dir: &Path, program: &str) -> Vec<PathBuf> {
    #[cfg(windows)]
    let extensions = Some(env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned()));
    #[cfg(not(windows))]
    let extensions: Option<String> = None;

    executable_candidates_with_extensions(dir, program, extensions.as_deref())
}

fn executable_candidates_with_extensions(
    dir: &Path,
    program: &str,
    extensions: Option<&str>,
) -> Vec<PathBuf> {
    let mut candidates = vec![dir.join(program)];
    if let Some(extensions) = extensions {
        for extension in extensions
            .split(';')
            .filter(|extension| !extension.is_empty())
        {
            candidates.push(dir.join(format!("{program}{extension}")));
        }
    }
    candidates
}

fn is_executable_file(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        path.metadata()
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_path_finds_the_active_cargo_toolchain() {
        assert!(which("cargo").is_some());
    }

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

    #[test]
    fn executable_candidates_expand_windows_extensions() {
        let got = executable_candidates_with_extensions(
            Path::new("tools"),
            "pandoc",
            Some(".COM;.EXE;;.CMD"),
        );
        assert_eq!(
            got,
            vec![
                PathBuf::from("tools/pandoc"),
                PathBuf::from("tools/pandoc.COM"),
                PathBuf::from("tools/pandoc.EXE"),
                PathBuf::from("tools/pandoc.CMD"),
            ]
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
