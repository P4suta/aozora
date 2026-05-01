//! Golden-fixture infrastructure for the aozora parser conformance
//! suite.
//!
//! # Fixture layout
//!
//! Each fixture lives under `crates/aozora-conformance/fixtures/<group>/<case>/`
//! and contains:
//!
//! - `source.txt` — input bytes (UTF-8) for the parser
//! - `expected.html` — golden HTML output from `tree.to_html()`
//! - `expected.serialize.txt` — golden text output from `tree.serialize()`
//!
//! Tests load a fixture, parse it, and assert the rendered output
//! matches the golden byte-for-byte. To regenerate goldens after an
//! intentional output change, run with `UPDATE_GOLDEN=1`:
//!
//! ```text
//! UPDATE_GOLDEN=1 cargo test -p aozora-conformance --test render_gate
//! ```
//!
//! This scaffolding backs both the byte-identical render gate over a
//! small representative set and the hand-curated reference corpus
//! (25-30 fixtures spanning every `NodeKind`) used by the WPT-style
//! conformance runner.

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
        Self {
            name: format!("{group}/{case}"),
            dir: dir.to_path_buf(),
            source,
            expected_html,
            expected_serialize,
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
