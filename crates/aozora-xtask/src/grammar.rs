//! Checks and regenerates tree-sitter parser artifacts using a temporary copy of `grammar.js`.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{self, Command};

const GRAMMAR_CRATE_REL: &str = "crates/tree-sitter-aozora";

const ARTEFACTS: &[&str] = &["src/parser.c", "src/grammar.json", "src/node-types.json"];

pub(crate) fn dispatch(update: bool) -> Result<(), String> {
    if update { regenerate() } else { check() }
}

fn workspace_root() -> Result<PathBuf, String> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let root = Path::new(manifest_dir)
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| {
            format!("could not derive workspace root from CARGO_MANIFEST_DIR={manifest_dir:?}")
        })?;
    Ok(root.to_path_buf())
}

#[derive(Debug)]
struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    fn create() -> Result<Self, String> {
        let path = env::temp_dir().join(format!("aozora-grammar-regen-{}", process::id()));
        Self::create_at(path)
    }

    fn create_at(path: PathBuf) -> Result<Self, String> {
        if let Err(err) = fs::remove_dir_all(&path)
            && err.kind() != io::ErrorKind::NotFound
        {
            return Err(format!("clear stale {}: {err}", path.display()));
        }
        fs::create_dir_all(&path).map_err(|err| format!("create {}: {err}", path.display()))?;
        Ok(Self { path })
    }
}

impl Drop for ScratchDir {
    #[expect(
        clippy::print_stderr,
        reason = "report grammar restoration failures during cleanup"
    )]
    fn drop(&mut self) {
        if let Err(err) = fs::remove_dir_all(&self.path) {
            eprintln!("remove scratch directory {}: {err}", self.path.display());
        }
    }
}

fn scratch_generate(grammar_crate: &Path) -> Result<ScratchDir, String> {
    let scratch = ScratchDir::create()?;
    let grammar_js = grammar_crate.join("grammar.js");
    fs::copy(&grammar_js, scratch.path.join("grammar.js"))
        .map_err(|err| format!("copy {}: {err}", grammar_js.display()))?;

    let output = Command::new("tree-sitter")
        .arg("generate")
        .current_dir(&scratch.path)
        .output()
        .map_err(|err| {
            format!(
                "spawn `tree-sitter generate` failed: {err}\n\
                 (the tree-sitter CLI must be on PATH; run `just setup`, then \
                 use `just drift-gate` / `just grammar`)"
            )
        })?;
    if output.status.success() {
        Ok(scratch)
    } else {
        Err(format!(
            "`tree-sitter generate` failed ({}):\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

#[expect(
    clippy::print_stderr,
    reason = "the command reports results and diagnostics to developers"
)]
fn check() -> Result<(), String> {
    let root = workspace_root()?;
    let grammar_crate = root.join(GRAMMAR_CRATE_REL);
    let scratch = scratch_generate(&grammar_crate)?;

    let mut drift = Vec::new();
    for rel in ARTEFACTS {
        let fresh = fs::read(scratch.path.join(rel))
            .map_err(|err| format!("read regenerated {rel}: {err}"))?;
        let stored_path = grammar_crate.join(rel);
        let stored = fs::read(&stored_path)
            .map_err(|err| format!("read committed {}: {err}", stored_path.display()))?;
        if fresh != stored {
            drift.push(*rel);
        }
    }

    if drift.is_empty() {
        eprintln!(
            "xtask conformance grammar check: {}/{} artefacts up to date",
            ARTEFACTS.len(),
            ARTEFACTS.len(),
        );
        Ok(())
    } else {
        Err(format!(
            "grammar drift detected in {} artefact(s):\n  {}\n\
             `grammar.js` changed without regenerating the committed parser. \
             Run `just grammar` (`xtask conformance grammar --update`), then commit.",
            drift.len(),
            drift.join("\n  "),
        ))
    }
}

#[expect(
    clippy::print_stderr,
    reason = "the command reports results and diagnostics to developers"
)]
fn regenerate() -> Result<(), String> {
    let root = workspace_root()?;
    let grammar_crate = root.join(GRAMMAR_CRATE_REL);
    let scratch = scratch_generate(&grammar_crate)?;

    for rel in ARTEFACTS {
        let dst = grammar_crate.join(rel);
        fs::copy(scratch.path.join(rel), &dst)
            .map_err(|err| format!("write {}: {err}", dst.display()))?;
        eprintln!("xtask conformance grammar update: wrote {}", dst.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scratch_recreates_stale_directory() {
        let root = tempfile::tempdir().expect("temporary root");
        let path = root.path().join("scratch");
        fs::create_dir_all(path.join("stale")).expect("stale directory");
        fs::write(path.join("stale/grammar.js"), "stale grammar").expect("stale file");

        let scratch = ScratchDir::create_at(path.clone()).expect("recreated scratch directory");

        assert_eq!(scratch.path, path);
        assert!(path.is_dir());
        assert!(
            fs::read_dir(&path)
                .expect("scratch contents")
                .next()
                .is_none()
        );
    }

    #[test]
    fn scratch_creates_absent_directory() {
        let root = tempfile::tempdir().expect("temporary root");
        let path = root.path().join("scratch");

        let scratch = ScratchDir::create_at(path.clone()).expect("new scratch directory");

        assert_eq!(scratch.path, path);
        assert!(path.is_dir());
        drop(scratch);
        assert!(!path.exists());
    }

    #[test]
    fn scratch_reports_stale_clear_error_for_regular_file() {
        let root = tempfile::tempdir().expect("temporary root");
        let path = root.path().join("scratch");
        fs::write(&path, "existing file").expect("regular file");

        let error = ScratchDir::create_at(path.clone()).expect_err("regular file must fail");

        assert!(
            error.starts_with(&format!("clear stale {}:", path.display())),
            "unexpected error: {error}"
        );
        assert_eq!(
            fs::read_to_string(path).expect("preserved file"),
            "existing file"
        );
    }

    #[test]
    fn artefacts_are_the_parser_and_two_manifests() {
        assert_eq!(
            ARTEFACTS.len(),
            3,
            "parser.c + grammar.json + node-types.json"
        );
        assert!(ARTEFACTS.contains(&"src/parser.c"));
        assert!(ARTEFACTS.contains(&"src/grammar.json"));
        assert!(ARTEFACTS.contains(&"src/node-types.json"));
    }

    #[test]
    fn artefacts_live_under_the_grammar_crate_src() {
        for rel in ARTEFACTS {
            assert!(
                Path::new(rel).starts_with("src"),
                "generated artefact lives under src/: {rel}"
            );
        }
    }

    #[test]
    fn committed_artefacts_exist_on_disk() {
        let grammar_crate = workspace_root()
            .expect("workspace root")
            .join(GRAMMAR_CRATE_REL);
        assert!(
            grammar_crate.join("grammar.js").is_file(),
            "grammar.js source of truth must exist at {}",
            grammar_crate.display()
        );
        for rel in ARTEFACTS {
            let path = grammar_crate.join(rel);
            assert!(
                path.is_file(),
                "committed artefact missing: {}",
                path.display()
            );
        }
    }
}
