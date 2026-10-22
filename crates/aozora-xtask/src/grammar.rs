//! tree-sitter grammar regeneration drift gate.
//!
//! `crates/tree-sitter-aozora/grammar.js` is the source of truth for the
//! reference grammar.
//! `tree-sitter generate` compiles it into the committed `src/parser.c` (+ `src/grammar.json` / `src/node-types.json`), which `build.rs` turns into the static parser used by the conformance harness.
//! Those generated files are checked in so downstream consumers need only a C
//! toolchain — but that also means an edit to `grammar.js` without a matching
//! `tree-sitter generate` leaves the committed parser stale and silently
//! wrong.
//!
//! `xtask conformance grammar --update` regenerates the artefacts from
//! `grammar.js` via the pinned `tree-sitter` CLI; `--check` (the default,
//! wired into `drift-gate`) exits non-zero when the on-disk artefacts have
//! drifted from a fresh generate. mise pins the CLI to the same
//! version as the `tree-sitter` runtime crate so the gate is reproducible.
//!
//! Mirrors the [`crate::schema`] / [`crate::types`] drift gates: `--update`
//! writes the artefact, `--check` fails on drift and points back at the
//! regenerate step. Unlike those, regeneration shells out to the external
//! `tree-sitter` binary, so it runs into a throwaway scratch tree
//! ([`ScratchDir`]) and `--check` never mutates the working copy.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{self, Command};

/// The reference-grammar crate, relative to the workspace root.
const GRAMMAR_CRATE_REL: &str = "crates/tree-sitter-aozora";

/// Generated artefacts under the grammar crate that `tree-sitter generate`
/// derives from `grammar.js`. These three encode the grammar itself — the
/// parse tables (`parser.c`) plus the JSON grammar / node-type manifests. The
/// `src/tree_sitter/*.h` runtime headers the same command vendors are build
/// scaffolding (part of the tree-sitter runtime ABI, not grammar-derived), so
/// they are intentionally outside the drift set.
const ARTEFACTS: &[&str] = &["src/parser.c", "src/grammar.json", "src/node-types.json"];

pub(crate) fn dispatch(update: bool) -> Result<(), String> {
    if update { regenerate() } else { check() }
}

fn workspace_root() -> Result<PathBuf, String> {
    // <workspace>/crates/aozora-xtask/Cargo.toml → workspace root is two up.
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
    fn drop(&mut self) {
        if let Err(err) = fs::remove_dir_all(&self.path) {
            eprintln!("remove scratch directory {}: {err}", self.path.display());
        }
    }
}

/// Run `tree-sitter generate` on a throwaway copy of `grammar.js` inside a
/// fresh [`ScratchDir`]. The generated files land under `<scratch>/src/`.
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
        // Guards against a path typo in ARTEFACTS / GRAMMAR_CRATE_REL: the
        // gate can only be meaningful if these are the files that are
        // actually checked in. Runs against the live workspace checkout.
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
