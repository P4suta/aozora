use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn workspace_root() -> Result<PathBuf, String> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    dir.ancestors()
        .find(|p| p.join("Cargo.lock").is_file())
        .map(Path::to_path_buf)
        .ok_or_else(|| "workspace root not found".to_owned())
}

pub(crate) fn tracked_rs_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let output = Command::new("git")
        .arg("ls-files")
        .arg("--")
        .arg("*.rs")
        .current_dir(root)
        .output()
        .map_err(|e| format!("spawn `git ls-files`: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "`git ls-files` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let listing = String::from_utf8(output.stdout)
        .map_err(|e| format!("`git ls-files` output was not UTF-8: {e}"))?;
    let files: Vec<PathBuf> = listing
        .lines()
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .filter(|path| root.join(path).is_file())
        .collect();
    if files.is_empty() {
        return Err("`git ls-files '*.rs'` returned nothing — scope is empty, \
                    so this gate would pass silently"
            .to_owned());
    }
    Ok(files)
}
