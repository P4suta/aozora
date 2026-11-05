//! Exercises portable developer commands through real files and Git repositories.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn developer(root: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("developer")
        .args(arguments)
        .arg("--root")
        .arg(root)
        .output()
        .expect("developer command starts")
}

fn portable_recipe(root: &Path, arguments: &[&str]) -> Output {
    if !cfg!(unix) {
        return developer(root, arguments);
    }
    Command::new("just")
        .args(["--shell", "/bin/bash", "--justfile"])
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Justfile"))
        .args(arguments)
        .arg("--root")
        .arg(root)
        .output()
        .expect("portable recipe starts")
}

#[test]
fn freshness_checks_use_the_recorded_instant_and_refuse_invalid_or_future_markers() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let marker = directory.path().join("target/.deps-check.timestamp");
    fs::create_dir_all(marker.parent().expect("marker parent")).expect("target directory");
    for (recorded, success) in [
        ("2026-10-24T00:00:00Z\n", true),
        ("2026-10-17T00:00:00Z\n", true),
        ("2026-10-16T23:59:59Z\n", false),
        ("2026-10-24T00:00:01Z\n", false),
        ("2026-02-30T00:00:00Z\n", false),
        ("2026-10-24T00:00:00+00:00\n", false),
        ("2026-10-24T00:00:00.1Z\n", false),
        ("unreadable timestamp\n", false),
    ] {
        fs::write(&marker, recorded).expect("fixture timestamp");
        let output = developer(
            directory.path(),
            &["deps-status", "--now", "2026-10-24T00:00:00Z"],
        );
        assert_eq!(
            output.status.success(),
            success,
            "marker {recorded:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fs::remove_file(&marker).expect("remove fixture marker");
    let absent = developer(directory.path(), &["deps-status"]);
    assert!(!absent.status.success());
    assert!(String::from_utf8_lossy(&absent.stderr).contains("never run"));
}

#[test]
fn decision_creation_preserves_title_arguments_and_refuses_overwriting_or_empty_slugs() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let records = directory.path().join("docs/adr");
    fs::create_dir_all(&records).expect("decision directory");
    let original = "# 0001. Existing\n\n- Status: accepted\n";
    fs::write(records.join("0001-existing.md"), original).expect("existing decision");
    let title = "Preserve spaces & quoted \"arguments\"";
    let created = portable_recipe(directory.path(), &["new-adr", title]);
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let path = records.join("preserve-spaces-quoted-arguments.md");
    let body = fs::read_to_string(&path).expect("descriptive decision filename");
    assert!(body.starts_with("---\ndate: "));
    assert!(body.contains(&format!("# {title}\n")));
    assert!(body.contains("Status: proposed"));
    assert!(
        !portable_recipe(directory.path(), &["new-adr", title])
            .status
            .success()
    );
    assert!(
        !developer(directory.path(), &["new-adr", "../"])
            .status
            .success()
    );
    assert_eq!(fs::read_to_string(&path).expect("created decision"), body);
    assert_eq!(
        fs::read_to_string(records.join("0001-existing.md")).expect("accepted decision"),
        original
    );
}

#[test]
fn hook_setup_preserves_a_configured_dispatcher_in_a_real_repository() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let root = directory.path().join("repository with spaces");
    let hooks = directory.path().join("owner hooks");
    fs::create_dir_all(&root).expect("repository directory");
    fs::create_dir_all(&hooks).expect("dispatcher directory");
    let sentinel = "owner dispatcher stays intact\n";
    fs::write(hooks.join("pre-commit"), sentinel).expect("owner dispatcher");
    assert!(
        Command::new("git")
            .arg("init")
            .arg("--quiet")
            .arg(&root)
            .status()
            .expect("git init")
            .success()
    );
    assert!(
        Command::new("git")
            .current_dir(&root)
            .args(["config", "--local", "core.hooksPath"])
            .arg(&hooks)
            .status()
            .expect("configure dispatcher")
            .success()
    );
    fs::write(
        root.join("lefthook.yml"),
        "pre-commit:\n  commands:\n    fixture:\n      run: git --version\n",
    )
    .expect("project hook configuration");
    for operation in ["hooks", "hooks-uninstall"] {
        let output = portable_recipe(&root, &[operation]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            fs::read_to_string(hooks.join("pre-commit")).expect("owner dispatcher"),
            sentinel
        );
        assert!(!root.join(".git/hooks/pre-commit").exists());
    }
}
