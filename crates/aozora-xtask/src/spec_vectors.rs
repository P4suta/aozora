use std::collections::BTreeSet;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::{SpecVectorsArgs, SpecVectorsOp};

const DEST_REL: &str = "crates/aozora-conformance/spec-vectors";
const SPEC_CONFORMANCE_REL: &str = "conformance";
const DEFAULT_SPEC_REL: &str = "../aozora-notation-spec";
const SPEC_REPO_ENV: &str = "AOZORA_SPEC_REPO";

const VECTORS_SUBDIR: &str = "vectors";
const SCHEMA_REL: &str = "schema/vector.schema.json";
const RUNNER_REL: &str = "RUNNER.md";

pub(crate) fn dispatch(args: &SpecVectorsArgs) -> Result<(), String> {
    match args.op {
        SpecVectorsOp::Check => check(),
        SpecVectorsOp::Sync => sync(),
    }
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

fn spec_repo(root: &Path) -> PathBuf {
    resolve_spec_repo(root, env::var_os(SPEC_REPO_ENV))
}

fn resolve_spec_repo(root: &Path, override_var: Option<OsString>) -> PathBuf {
    override_var.map_or_else(
        || root.join(DEFAULT_SPEC_REL),
        |v| {
            let p = PathBuf::from(v);
            if p.is_absolute() { p } else { root.join(p) }
        },
    )
}

#[expect(
    clippy::print_stderr,
    reason = "the command reports results and diagnostics to developers"
)]
fn check() -> Result<(), String> {
    let root = workspace_root()?;
    let spec = spec_repo(&root);
    let spec_conf = spec.join(SPEC_CONFORMANCE_REL);
    let spec_vectors = spec_conf.join(VECTORS_SUBDIR);

    if !spec_vectors.is_dir() {
        return Err(format!(
            "sibling spec repo not found at {} (set {SPEC_REPO_ENV}, or check out \
             P4suta/aozora-notation-spec).",
            spec.display()
        ));
    }

    let dest = root.join(DEST_REL);
    let mut drift = Vec::new();
    diff_tree(&spec_vectors, &dest.join(VECTORS_SUBDIR), &mut drift)?;
    diff_file(
        SCHEMA_REL,
        &spec_conf.join(SCHEMA_REL),
        &dest.join(SCHEMA_REL),
        &mut drift,
    )?;
    diff_file(
        RUNNER_REL,
        &spec_conf.join(RUNNER_REL),
        &dest.join(RUNNER_REL),
        &mut drift,
    )?;

    if drift.is_empty() {
        eprintln!(
            "spec-vectors check: vendored copy matches {} ✔",
            spec.display()
        );
        Ok(())
    } else {
        Err(format!(
            "vendored spec vectors drift from {} in {} path(s):\n  {}\n\
             run `just sync-spec-vectors` and commit the diff — the spec repo is \
             the source of truth for the conformance corpus.",
            spec.display(),
            drift.len(),
            drift.join("\n  "),
        ))
    }
}

#[expect(
    clippy::print_stderr,
    reason = "the command reports results and diagnostics to developers"
)]
fn sync() -> Result<(), String> {
    let root = workspace_root()?;
    let spec = spec_repo(&root);
    let spec_conf = spec.join(SPEC_CONFORMANCE_REL);
    let spec_vectors = spec_conf.join(VECTORS_SUBDIR);

    if !spec_vectors.is_dir() {
        return Err(format!(
            "sibling spec repo not found at {} (set {SPEC_REPO_ENV}). `sync` needs \
             the spec checked out — it is the source of truth for the corpus.",
            spec.display()
        ));
    }

    let dest = root.join(DEST_REL);
    let dest_vectors = dest.join(VECTORS_SUBDIR);
    let dest_schema = dest.join(SCHEMA_REL);
    let dest_schema_dir = dest_schema
        .parent()
        .ok_or_else(|| format!("schema path has no parent: {}", dest_schema.display()))?;

    remove_dir_if_present(&dest_vectors)?;
    remove_dir_if_present(dest_schema_dir)?;
    fs::create_dir_all(&dest_vectors)
        .map_err(|e| format!("create {}: {e}", dest_vectors.display()))?;
    fs::create_dir_all(dest_schema_dir)
        .map_err(|e| format!("create {}: {e}", dest_schema_dir.display()))?;

    copy_tree(&spec_vectors, &dest_vectors)?;
    copy_file(&spec_conf.join(SCHEMA_REL), &dest_schema)?;
    copy_file(&spec_conf.join(RUNNER_REL), &dest.join(RUNNER_REL))?;

    let count = collect_files(&dest_vectors)?
        .iter()
        .filter(|rel| rel.file_name().is_some_and(|n| n == "vector.json"))
        .count();
    eprintln!(
        "spec-vectors sync: vendored {count} vector(s) from {} → {}",
        spec.display(),
        dest.display()
    );
    Ok(())
}

fn collect_files(root: &Path) -> Result<BTreeSet<PathBuf>, String> {
    let mut out = BTreeSet::new();
    if !root.exists() {
        return Ok(out);
    }
    for entry in WalkDir::new(root).sort_by_file_name() {
        let entry = entry.map_err(|e| format!("walk {}: {e}", root.display()))?;
        if !entry.file_type().is_dir() {
            let rel = entry
                .path()
                .strip_prefix(root)
                .map_err(|e| format!("strip_prefix {}: {e}", entry.path().display()))?
                .to_path_buf();
            out.insert(rel);
        }
    }
    Ok(out)
}

fn diff_tree(spec_dir: &Path, dest_dir: &Path, drift: &mut Vec<String>) -> Result<(), String> {
    let spec_files = collect_files(spec_dir)?;
    let dest_files = collect_files(dest_dir)?;

    let mut all: BTreeSet<&PathBuf> = BTreeSet::new();
    all.extend(spec_files.iter());
    all.extend(dest_files.iter());

    for rel in all {
        let label = format!("{VECTORS_SUBDIR}/{}", rel.display());
        match (spec_files.contains(rel), dest_files.contains(rel)) {
            (true, false) => drift.push(format!("missing from vendored: {label}")),
            (false, true) => drift.push(format!("stale in vendored: {label}")),
            (true, true) => {
                let a = fs::read(spec_dir.join(rel))
                    .map_err(|e| format!("read {}: {e}", spec_dir.join(rel).display()))?;
                let b = fs::read(dest_dir.join(rel))
                    .map_err(|e| format!("read {}: {e}", dest_dir.join(rel).display()))?;
                if a != b {
                    drift.push(format!("differs: {label}"));
                }
            }
            (false, false) => {}
        }
    }
    Ok(())
}

fn diff_file(
    label: &str,
    spec_file: &Path,
    dest_file: &Path,
    drift: &mut Vec<String>,
) -> Result<(), String> {
    let spec_bytes =
        fs::read(spec_file).map_err(|e| format!("read spec {}: {e}", spec_file.display()))?;
    match fs::read(dest_file) {
        Ok(dest_bytes) => {
            if dest_bytes != spec_bytes {
                drift.push(format!("differs: {label}"));
            }
        }
        Err(_) => drift.push(format!("missing from vendored: {label}")),
    }
    Ok(())
}

fn remove_dir_if_present(dir: &Path) -> Result<(), String> {
    match fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("remove {}: {e}", dir.display())),
    }
}

fn copy_tree(src: &Path, dst: &Path) -> Result<(), String> {
    for entry in fs::read_dir(src).map_err(|e| format!("read_dir {}: {e}", src.display()))? {
        let entry = entry.map_err(|e| format!("read_dir entry in {}: {e}", src.display()))?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let ft = entry
            .file_type()
            .map_err(|e| format!("file_type {}: {e}", from.display()))?;
        if ft.is_dir() {
            fs::create_dir_all(&to).map_err(|e| format!("create {}: {e}", to.display()))?;
            copy_tree(&from, &to)?;
        } else {
            copy_file(&from, &to)?;
        }
    }
    Ok(())
}

fn copy_file(from: &Path, to: &Path) -> Result<(), String> {
    fs::copy(from, to)
        .map(|_| ())
        .map_err(|e| format!("copy {} → {}: {e}", from.display(), to.display()))
}

#[cfg(test)]
mod tests {
    use std::process;

    use super::*;

    struct Scratch {
        path: PathBuf,
    }

    impl Scratch {
        fn new(tag: &str) -> Self {
            let path =
                env::temp_dir().join(format!("aozora-spec-vectors-test-{}-{tag}", process::id()));
            fs::remove_dir_all(&path).ok();
            fs::create_dir_all(&path).expect("create scratch");
            Self { path }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.path).ok();
        }
    }

    fn write(path: &Path, bytes: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(path, bytes).expect("write");
    }

    #[test]
    fn copy_then_diff_is_clean() {
        let s = Scratch::new("clean");
        let src = s.path.join("src");
        let dst = s.path.join("dst");
        write(&src.join("a/vector.json"), "one");
        write(&src.join("b/c/vector.json"), "two");
        fs::create_dir_all(&dst).unwrap();

        copy_tree(&src, &dst).expect("copy_tree");
        let mut drift = Vec::new();
        diff_tree(&src, &dst, &mut drift).expect("diff_tree");
        assert!(drift.is_empty(), "round-trip must be drift-free: {drift:?}");
    }

    #[test]
    fn diff_detects_content_stale_and_missing() {
        let s = Scratch::new("drift");
        let src = s.path.join("src");
        let dst = s.path.join("dst");
        write(&src.join("shared/vector.json"), "spec");
        write(&dst.join("shared/vector.json"), "vendored");
        write(&src.join("only_spec/vector.json"), "x");
        write(&dst.join("only_dst/vector.json"), "y");

        let mut drift = Vec::new();
        diff_tree(&src, &dst, &mut drift).expect("diff_tree");
        assert_eq!(drift.len(), 3, "three divergences: {drift:?}");
        assert!(drift.iter().any(|d| d.contains("differs: vectors/shared")));
        assert!(
            drift
                .iter()
                .any(|d| d.contains("missing from vendored: vectors/only_spec"))
        );
        assert!(
            drift
                .iter()
                .any(|d| d.contains("stale in vendored: vectors/only_dst"))
        );
    }

    #[test]
    fn diff_file_flags_content_and_absence() {
        let s = Scratch::new("file");
        let spec = s.path.join("spec.md");
        write(&spec, "canonical");

        let same = s.path.join("same.md");
        write(&same, "canonical");
        let mut drift = Vec::new();
        diff_file("RUNNER.md", &spec, &same, &mut drift).expect("diff_file");
        assert!(drift.is_empty(), "identical file is clean: {drift:?}");

        let other = s.path.join("other.md");
        write(&other, "edited");
        diff_file("RUNNER.md", &spec, &other, &mut drift).expect("diff_file");
        assert_eq!(drift, vec!["differs: RUNNER.md"]);

        drift.clear();
        diff_file("RUNNER.md", &spec, &s.path.join("nope.md"), &mut drift).expect("diff_file");
        assert_eq!(drift, vec!["missing from vendored: RUNNER.md"]);
    }

    #[test]
    fn spec_repo_honours_absolute_and_relative_override() {
        let root = Path::new("/ws");
        assert_eq!(
            resolve_spec_repo(root, Some(OsString::from("/elsewhere/spec"))),
            PathBuf::from("/elsewhere/spec")
        );
        assert_eq!(
            resolve_spec_repo(root, Some(OsString::from("sibling/spec"))),
            PathBuf::from("/ws/sibling/spec")
        );
        assert_eq!(
            resolve_spec_repo(root, None),
            PathBuf::from("/ws/../aozora-notation-spec")
        );
    }
}
