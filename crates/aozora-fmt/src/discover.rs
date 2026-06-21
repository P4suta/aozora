//! Resolve the positional path arguments into a concrete, ordered,
//! de-duplicated list of files — or stdin.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use walkdir::{DirEntry, WalkDir};

use crate::cli::is_stdin;

/// The resolved input source.
///
/// Re-exported from the crate root (with [`resolve`]) so the `aozora` CLI's
/// `lint` subcommand reuses the formatter's exact path-discovery rules.
#[derive(Debug)]
pub enum Input {
    /// Read a single document from stdin.
    Stdin,
    /// A set of files discovered from the path arguments.
    Files(Resolved),
}

/// Files to process plus any non-fatal discovery errors (a `walkdir`
/// traversal error, say) accumulated rather than aborting the whole run.
#[derive(Debug, Default)]
pub struct Resolved {
    /// The discovered source files, sorted and de-duplicated.
    pub files: Vec<PathBuf>,
    /// Non-fatal discovery errors (e.g. a traversal error), to be reported
    /// without aborting the run.
    pub errors: Vec<String>,
}

/// Filename suffixes recognised as aozora sources during directory
/// recursion (matched case-insensitively).
const EXTENSIONS: &[&str] = &[".afm", ".aozora", ".aozora.txt"];

/// Classify the path arguments. Returns [`Input::Stdin`] when no paths are
/// given or the sole argument is `-`; mixing `-` with real paths is an error.
///
/// Recurses directories for `*.afm`, `*.aozora`, and `*.aozora.txt` files,
/// skipping `target/` and dotted entries, then sorts and de-duplicates.
///
/// # Errors
///
/// Returns an error only when `-` (stdin) is mixed with real path arguments.
/// Per-file traversal errors are accumulated in [`Resolved::errors`] instead.
pub fn resolve(paths: &[PathBuf]) -> Result<Input> {
    if paths.is_empty() {
        return Ok(Input::Stdin);
    }
    if paths.iter().any(|p| is_stdin(p)) {
        if paths.len() > 1 {
            bail!("cannot mix stdin (`-`) with file paths");
        }
        return Ok(Input::Stdin);
    }

    let mut resolved = Resolved::default();
    for path in paths {
        collect(path, &mut resolved);
    }
    resolved.files.sort();
    resolved.files.dedup();
    Ok(Input::Files(resolved))
}

/// Add one argument's files to `resolved`: recurse directories, take other
/// paths verbatim. A missing path is pushed as-is so it surfaces as a read
/// error during processing — keeping all per-file error accounting in one
/// place rather than splitting it across discovery and processing.
#[allow(
    clippy::filetype_is_file,
    reason = "recursion intentionally processes regular files only; symlinks are skipped to match WalkDir's follow_links(false)"
)]
fn collect(path: &Path, resolved: &mut Resolved) {
    if !path.is_dir() {
        resolved.files.push(path.to_path_buf());
        return;
    }
    let walk = WalkDir::new(path)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| !is_ignored(e));
    for entry in walk {
        match entry {
            Ok(entry) if entry.file_type().is_file() && is_aozora_source(entry.path()) => {
                resolved.files.push(entry.into_path());
            }
            Ok(_) => {}
            Err(err) => resolved.errors.push(err.to_string()),
        }
    }
}

/// Skip `target` and dotted entries (e.g. `.git`) below the explicitly
/// passed root; the root itself (depth 0) is never skipped.
fn is_ignored(entry: &DirEntry) -> bool {
    if entry.depth() == 0 {
        return false;
    }
    let name = entry.file_name().to_string_lossy();
    name == "target" || name.starts_with('.')
}

/// True for filenames ending in a recognised aozora source extension,
/// e.g. `chapter.afm`, `本文.aozora`, `note.aozora.txt`.
fn is_aozora_source(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::env;
    use std::fs;
    use std::process;
    use std::slice;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A fresh, empty scratch directory under the OS temp dir, unique per
    /// call so parallel test threads don't clobber each other.
    fn scratch_dir(name: &str) -> PathBuf {
        static SEQ: AtomicU32 = AtomicU32::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let mut dir = env::temp_dir();
        dir.push(format!("aozora-fmt-discover-{}-{n}-{name}", process::id()));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    fn touch(dir: &Path, rel: &str) -> PathBuf {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(&path, "x").expect("write file");
        path
    }

    fn files_of(input: Input) -> Vec<PathBuf> {
        match input {
            Input::Files(resolved) => resolved.files,
            Input::Stdin => panic!("expected Input::Files, got Stdin"),
        }
    }

    #[test]
    fn no_paths_resolve_to_stdin() {
        assert!(matches!(resolve(&[]).unwrap(), Input::Stdin));
    }

    #[test]
    fn lone_dash_resolves_to_stdin() {
        let paths = [PathBuf::from("-")];
        assert!(matches!(resolve(&paths).unwrap(), Input::Stdin));
    }

    #[test]
    fn dash_mixed_with_a_path_is_an_error() {
        let paths = [PathBuf::from("-"), PathBuf::from("a.afm")];
        let err = resolve(&paths).expect_err("mixing stdin and paths must error");
        assert!(err.to_string().contains("cannot mix"), "{err}");
    }

    #[test]
    fn extension_match_is_case_insensitive_and_suffix_aware() {
        assert!(is_aozora_source(Path::new("chapter.afm")));
        assert!(is_aozora_source(Path::new("本文.aozora")));
        assert!(is_aozora_source(Path::new("note.aozora.txt")));
        assert!(is_aozora_source(Path::new("LOUD.AFM")));
        assert!(!is_aozora_source(Path::new("plain.txt")));
        assert!(!is_aozora_source(Path::new("readme.md")));
        assert!(!is_aozora_source(Path::new("noext")));
    }

    #[test]
    fn missing_path_is_passed_through_verbatim() {
        // A non-existent, non-directory argument is kept as-is so it
        // surfaces as a read error later — not dropped during discovery.
        let paths = [PathBuf::from("/definitely/not/here.afm")];
        let files = files_of(resolve(&paths).unwrap());
        assert_eq!(files, vec![PathBuf::from("/definitely/not/here.afm")]);
    }

    #[test]
    fn duplicate_path_arguments_are_deduplicated() {
        let dir = scratch_dir("dedup");
        let f = touch(&dir, "a.afm");
        let paths = [f.clone(), f.clone()];
        let files = files_of(resolve(&paths).unwrap());
        assert_eq!(files, vec![f], "duplicate args must collapse to one");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn directory_recursion_filters_sorts_and_skips_ignored() {
        let dir = scratch_dir("walk");
        let a = touch(&dir, "a.afm");
        let b = touch(&dir, "b.aozora");
        touch(&dir, "c.txt"); // wrong extension → skipped
        let nested = touch(&dir, "sub/d.aozora.txt");
        touch(&dir, "target/skip.afm"); // build dir → skipped
        touch(&dir, ".hidden/skip.afm"); // dotted dir → skipped

        let files = files_of(resolve(slice::from_ref(&dir)).unwrap());
        assert_eq!(
            files,
            vec![a, b, nested],
            "only sources outside target/dotfiles, sorted",
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_skipped_during_recursion() {
        use std::os::unix::fs::symlink;
        let dir = scratch_dir("symlink");
        let real = touch(&dir, "real.afm");
        symlink(&real, dir.join("link.afm")).expect("create symlink");

        let files = files_of(resolve(slice::from_ref(&dir)).unwrap());
        assert_eq!(
            files,
            vec![real],
            "the symlink entry is not a regular file, so it's skipped",
        );
        fs::remove_dir_all(&dir).ok();
    }
}
