//! JSON Schema artefact dump / drift gate.
//!
//! Bridges `aozora::wire::schema_*` → `crates/aozora-book/src/wire/schema-*.json`.
//! `xtask schema dump` regenerates the four schema files; `xtask
//! schema check` exits non-zero when the on-disk artefact has
//! drifted from the live wire types.
//!
//! The artefact lives in the handbook source tree so external
//! consumers (downstream filter / plugin authors) can fetch the
//! schema from a stable URL once GitHub Pages publishes the handbook.

use std::fs;
use std::path::{Path, PathBuf};

use aozora::wire;

use crate::SchemaArgs;
use crate::SchemaOp;

type SchemaGen = fn() -> serde_json::Value;

/// Schema file relative paths under workspace root, paired with the
/// generator function that produces the live schema. Order matches
/// the wire endpoints (`serialize_diagnostics` →
/// `serialize_container_pairs`).
const SCHEMA_FILES: &[(&str, SchemaGen)] = &[
    (
        "crates/aozora-book/src/wire/schema-diagnostics.json",
        wire::schema_diagnostics,
    ),
    (
        "crates/aozora-book/src/wire/schema-nodes.json",
        wire::schema_nodes,
    ),
    (
        "crates/aozora-book/src/wire/schema-pairs.json",
        wire::schema_pairs,
    ),
    (
        "crates/aozora-book/src/wire/schema-container-pairs.json",
        wire::schema_container_pairs,
    ),
];

pub(crate) fn dispatch(args: &SchemaArgs) -> Result<(), String> {
    match args.op {
        SchemaOp::Dump => dump(),
        SchemaOp::Check => check(),
    }
}

fn workspace_root() -> Result<PathBuf, String> {
    // The xtask binary lives under <workspace>/crates/aozora-xtask;
    // resolve the workspace root by stripping two directory levels
    // from CARGO_MANIFEST_DIR so this works from any cwd.
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let root = Path::new(manifest_dir)
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| {
            format!("could not derive workspace root from CARGO_MANIFEST_DIR={manifest_dir:?}")
        })?;
    Ok(root.to_path_buf())
}

fn render(value: &serde_json::Value) -> String {
    // Pretty-print with trailing newline so the on-disk file follows
    // the standard text-file convention. `to_string_pretty` uses
    // 2-space indent.
    let mut s = serde_json::to_string_pretty(value).expect("serde_json pretty print");
    s.push('\n');
    s
}

fn dump() -> Result<(), String> {
    let root = workspace_root()?;
    if let Some(parent) = root.join(SCHEMA_FILES[0].0).parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("create_dir_all {}: {err}", parent.display()))?;
    }
    for (rel, make_schema) in SCHEMA_FILES {
        let path = root.join(rel);
        let text = render(&make_schema());
        fs::write(&path, &text)
            .map_err(|err| format!("write schema artefact {}: {err}", path.display()))?;
        eprintln!("xtask schema dump: wrote {}", path.display());
    }
    Ok(())
}

fn check() -> Result<(), String> {
    let root = workspace_root()?;
    let mut drift = Vec::new();
    for (rel, make_schema) in SCHEMA_FILES {
        let path = root.join(rel);
        let actual = render(&make_schema());
        let stored = fs::read_to_string(&path)
            .map_err(|err| format!("read schema artefact {}: {err}", path.display()))?;
        if actual != stored {
            drift.push(rel.to_string());
        }
    }
    if drift.is_empty() {
        eprintln!("xtask schema check: 4/4 schema artefacts up to date");
        Ok(())
    } else {
        Err(format!(
            "schema drift detected in {} file(s):\n  {}\n\
             run `xtask schema dump` to regenerate, then commit",
            drift.len(),
            drift.join("\n  "),
        ))
    }
}
