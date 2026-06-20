//! JSON Schema artefact dump / drift gate.
//!
//! Bridges `aozora::json::schema_*` → `crates/aozora-book/src/wire/schema-*.json`.
//! `xtask schema dump` regenerates the four schema files; `xtask
//! schema check` exits non-zero when the on-disk artefact has
//! drifted from the live wire types.
//!
//! The artefact lives in the handbook source tree so external
//! consumers (downstream filter / plugin authors) can fetch the
//! schema from a stable URL once GitHub Pages publishes the handbook.

use std::fs;
use std::path::{Path, PathBuf};

use aozora::json;

use crate::SchemaArgs;
use crate::SchemaOp;

pub(crate) type SchemaGen = fn() -> serde_json::Value;

/// Schema file relative paths under workspace root, paired with the
/// generator function that produces the live schema. Order matches
/// the wire endpoints (`diagnostics` →
/// `container_pairs`).
///
/// `pub(crate)` so the `types` module can read the same committed schema
/// artefacts as the single source of truth for `quicktype` codegen.
pub(crate) const SCHEMA_FILES: &[(&str, SchemaGen)] = &[
    (
        "crates/aozora-book/src/wire/schema-diagnostics.json",
        json::schema_diagnostics,
    ),
    (
        "crates/aozora-book/src/wire/schema-nodes.json",
        json::schema_nodes,
    ),
    (
        "crates/aozora-book/src/wire/schema-pairs.json",
        json::schema_pairs,
    ),
    (
        "crates/aozora-book/src/wire/schema-container-pairs.json",
        json::schema_container_pairs,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_pretty_prints_with_two_space_indent() {
        let value = serde_json::json!({ "a": 1, "b": [2, 3] });
        let out = render(&value);
        // 2-space indent for nested keys.
        assert!(out.contains("  \"a\": 1"), "2-space indent: {out}");
        assert!(
            out.contains("  \"b\": ["),
            "2-space indent for array: {out}"
        );
    }

    #[test]
    fn render_appends_trailing_newline() {
        let out = render(&serde_json::json!({}));
        assert!(out.ends_with('\n'), "must end with newline: {out:?}");
        assert!(
            !out.ends_with("\n\n"),
            "exactly one trailing newline: {out:?}"
        );
    }

    #[test]
    fn render_round_trips_to_equal_json_value() {
        let value = serde_json::json!({ "nested": { "x": true }, "list": [1, 2] });
        let out = render(&value);
        let reparsed: serde_json::Value =
            serde_json::from_str(&out).expect("render output must be valid JSON");
        assert_eq!(reparsed, value, "render must preserve the value");
    }

    #[test]
    fn schema_files_cover_all_four_wire_endpoints() {
        assert_eq!(SCHEMA_FILES.len(), 4, "exactly four wire schema files");
        for (rel, _) in SCHEMA_FILES {
            assert!(
                Path::new(rel)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("json")),
                "schema artefact path must be a .json file: {rel}"
            );
            assert!(
                rel.contains("wire/schema-"),
                "schema artefact lives under wire/: {rel}"
            );
        }
    }

    #[test]
    fn schema_generators_produce_object_schemas() {
        // Each generator must yield a JSON object with a `title`; this is
        // pure (no I/O) and exercises the live wire schema codegen.
        for (rel, make_schema) in SCHEMA_FILES {
            let schema = make_schema();
            let obj = schema
                .as_object()
                .unwrap_or_else(|| panic!("{rel}: schema root must be an object"));
            assert!(obj.contains_key("title"), "{rel}: schema must have a title");
        }
    }
}
