use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use walkdir::{DirEntry, WalkDir};

use crate::fmt::cli::is_stdin;

#[derive(Debug)]
pub(crate) enum Input {
    Stdin,
    Files(Resolved),
}

#[derive(Debug, Default)]
pub(crate) struct Resolved {
    pub files: Vec<PathBuf>,
    pub errors: Vec<String>,
}

const EXTENSIONS: &[&str] = &[".afm", ".aozora", ".aozora.txt"];

pub(crate) fn resolve(paths: &[PathBuf]) -> Result<Input> {
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

#[expect(
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

fn is_ignored(entry: &DirEntry) -> bool {
    if entry.depth() == 0 {
        return false;
    }
    let name = entry.file_name().to_string_lossy();
    name == "target" || name.starts_with('.')
}

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
        touch(&dir, "c.txt");
        let nested = touch(&dir, "sub/d.aozora.txt");
        touch(&dir, "target/skip.afm");
        touch(&dir, ".hidden/skip.afm");

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
