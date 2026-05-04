//! Golden-fixture infrastructure for the aozora parser conformance
//! suite.
//!
//! # Fixture layout
//!
//! Each fixture lives under `crates/aozora-conformance/fixtures/<group>/<case>/`
//! and contains:
//!
//! - `source.txt` — input bytes (UTF-8) for the parser
//! - `expected.html` — golden output from `tree.to_html()`
//! - `expected.serialize.txt` — golden output from `tree.serialize()`
//! - `expected.diagnostics.json` — golden output from
//!   `aozora::wire::serialize_diagnostics(tree.diagnostics())`
//! - `expected.nodes.json` — golden output from
//!   `aozora::wire::serialize_nodes(&tree)`
//! - `expected.pairs.json` — golden output from
//!   `aozora::wire::serialize_pairs(&tree)`
//! - `expected.container_pairs.json` — golden output from
//!   `aozora::wire::serialize_container_pairs(&tree)`
//!
//! The two source-text axes (`html`, `serialize`) anchor the
//! human-readable surface; the four wire-format axes pin the JSON
//! projections that drivers (FFI / WASM / `PyO3`) consume — every
//! cross-language wire byte is exercised against the same fixture
//! set, so a regression that survives the renderer gate but breaks
//! a wire client lights up here.
//!
//! Tests load a fixture, parse it, and assert each rendered output
//! matches its golden byte-for-byte. To regenerate goldens after an
//! intentional output change, run with `UPDATE_GOLDEN=1`:
//!
//! ```text
//! UPDATE_GOLDEN=1 cargo test -p aozora-conformance --test render_gate
//! ```
//!
//! This scaffolding backs the byte-identical render gate over the
//! representative `render` group as well as the hand-curated
//! reference corpus used by the WPT-style conformance runner.

#![forbid(unsafe_code)]

use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

/// In-memory representation of a render fixture.
#[derive(Debug, Clone)]
pub struct RenderFixture {
    /// Group + case identifier, used in test failure messages.
    pub name: String,
    /// Filesystem path to the fixture directory.
    pub dir: PathBuf,
    /// Source bytes from `source.txt`.
    pub source: String,
    /// Expected HTML from `expected.html`. `None` if the file does
    /// not exist (`UPDATE_GOLDEN` mode populates it on first run).
    pub expected_html: Option<String>,
    /// Expected serialize output from `expected.serialize.txt`.
    pub expected_serialize: Option<String>,
    /// Expected diagnostics-wire JSON from `expected.diagnostics.json`.
    pub expected_diagnostics: Option<String>,
    /// Expected nodes-wire JSON from `expected.nodes.json`.
    pub expected_nodes: Option<String>,
    /// Expected pairs-wire JSON from `expected.pairs.json`.
    pub expected_pairs: Option<String>,
    /// Expected container-pairs-wire JSON from
    /// `expected.container_pairs.json`.
    pub expected_container_pairs: Option<String>,
}

impl RenderFixture {
    /// Load every fixture under `fixtures_root/<group>/`.
    ///
    /// Returns fixtures sorted alphabetically by `<group>/<case>` so
    /// test failure output is reproducible.
    ///
    /// # Panics
    ///
    /// Panics if `fixtures_root` does not exist — the caller is
    /// expected to be a test that knows the layout.
    #[must_use]
    pub fn load_group(fixtures_root: &Path, group: &str) -> Vec<Self> {
        let group_dir = fixtures_root.join(group);
        assert!(
            group_dir.is_dir(),
            "fixtures group {group:?} missing under {}",
            fixtures_root.display()
        );
        let mut entries: Vec<_> = fs::read_dir(&group_dir)
            .expect("read_dir on fixtures group")
            .filter_map(Result::ok)
            .filter(|e| e.path().is_dir())
            .collect();
        entries.sort_by_key(fs::DirEntry::file_name);
        entries
            .into_iter()
            .map(|entry| Self::load_one(group, &entry.path()))
            .collect()
    }

    fn load_one(group: &str, dir: &Path) -> Self {
        let case = dir
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("?")
            .to_owned();
        let source_path = dir.join("source.txt");
        let source = fs::read_to_string(&source_path)
            .unwrap_or_else(|_| panic!("missing source.txt in fixture {group}/{case}"));
        let expected_html = fs::read_to_string(dir.join("expected.html")).ok();
        let expected_serialize = fs::read_to_string(dir.join("expected.serialize.txt")).ok();
        let expected_diagnostics = fs::read_to_string(dir.join("expected.diagnostics.json")).ok();
        let expected_nodes = fs::read_to_string(dir.join("expected.nodes.json")).ok();
        let expected_pairs = fs::read_to_string(dir.join("expected.pairs.json")).ok();
        let expected_container_pairs =
            fs::read_to_string(dir.join("expected.container_pairs.json")).ok();
        Self {
            name: format!("{group}/{case}"),
            dir: dir.to_path_buf(),
            source,
            expected_html,
            expected_serialize,
            expected_diagnostics,
            expected_nodes,
            expected_pairs,
            expected_container_pairs,
        }
    }

    /// Write `expected.html` if `UPDATE_GOLDEN=1` is set, return the
    /// new content. Otherwise return the existing golden or panic.
    #[must_use]
    pub fn html_golden(&self, actual: &str) -> String {
        self.golden_for(&GoldenSpec {
            kind: "html",
            filename: "expected.html",
            existing: self.expected_html.as_ref(),
            actual,
        })
    }

    /// Write `expected.serialize.txt` if `UPDATE_GOLDEN=1` is set.
    #[must_use]
    pub fn serialize_golden(&self, actual: &str) -> String {
        self.golden_for(&GoldenSpec {
            kind: "serialize",
            filename: "expected.serialize.txt",
            existing: self.expected_serialize.as_ref(),
            actual,
        })
    }

    /// Write `expected.diagnostics.json` if `UPDATE_GOLDEN=1` is set.
    #[must_use]
    pub fn diagnostics_golden(&self, actual: &str) -> String {
        self.golden_for(&GoldenSpec {
            kind: "diagnostics",
            filename: "expected.diagnostics.json",
            existing: self.expected_diagnostics.as_ref(),
            actual,
        })
    }

    /// Write `expected.nodes.json` if `UPDATE_GOLDEN=1` is set.
    #[must_use]
    pub fn nodes_golden(&self, actual: &str) -> String {
        self.golden_for(&GoldenSpec {
            kind: "nodes",
            filename: "expected.nodes.json",
            existing: self.expected_nodes.as_ref(),
            actual,
        })
    }

    /// Write `expected.pairs.json` if `UPDATE_GOLDEN=1` is set.
    #[must_use]
    pub fn pairs_golden(&self, actual: &str) -> String {
        self.golden_for(&GoldenSpec {
            kind: "pairs",
            filename: "expected.pairs.json",
            existing: self.expected_pairs.as_ref(),
            actual,
        })
    }

    /// Write `expected.container_pairs.json` if `UPDATE_GOLDEN=1` is set.
    #[must_use]
    pub fn container_pairs_golden(&self, actual: &str) -> String {
        self.golden_for(&GoldenSpec {
            kind: "container_pairs",
            filename: "expected.container_pairs.json",
            existing: self.expected_container_pairs.as_ref(),
            actual,
        })
    }

    fn golden_for(&self, spec: &GoldenSpec<'_>) -> String {
        let path = self.dir.join(spec.filename);
        if env::var_os("UPDATE_GOLDEN").is_some() {
            fs::write(&path, spec.actual).unwrap_or_else(|err| {
                panic!(
                    "UPDATE_GOLDEN: failed to write {} golden for {}: {err}",
                    spec.kind, self.name
                );
            });
            return spec.actual.to_owned();
        }
        spec.existing.cloned().unwrap_or_else(|| {
            panic!(
                "fixture {}: expected {} golden missing — run with UPDATE_GOLDEN=1 to seed",
                self.name, spec.kind
            )
        })
    }
}

/// Internal helper bundle for `RenderFixture::golden_for` — keeps the
/// argument count under the workspace `clippy::too_many_arguments`
/// threshold without splitting the Read / Write paths.
struct GoldenSpec<'a> {
    kind: &'a str,
    filename: &'a str,
    existing: Option<&'a String>,
    actual: &'a str,
}

/// Path to the workspace's `crates/aozora-conformance/fixtures/`
/// directory. Resolved at test time via `CARGO_MANIFEST_DIR`.
#[must_use]
pub fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}
