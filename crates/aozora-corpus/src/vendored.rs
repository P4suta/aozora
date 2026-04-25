//! [`VendoredCorpus`] — read in-tree golden fixtures.
//!
//! Exposes `spec/aozora/fixtures/<card-id>/input.sjis.txt` files as a
//! [`CorpusSource`] without picking up the sibling `input.utf8.txt` /
//! `golden.html` that would otherwise show up in a plain
//! [`FilesystemCorpus`] walk of the same directory.
//!
//! Dedicated to the "sweep the goldens themselves" use case — applying
//! sweep invariants to Tier A fixtures to catch regressions without
//! running the full golden diff test, and to provide a non-empty
//! corpus when no external `AOZORA_CORPUS_ROOT` is available.

use std::fs;
use std::path::{Path, PathBuf};
use std::vec::IntoIter;

use crate::{CorpusError, CorpusItem, CorpusSource};

/// Name of the expected SJIS-encoded input file inside each fixture
/// subdirectory. Matches the layout seen under
/// `spec/aozora/fixtures/<card-id>/` in this repository.
const FIXTURE_INPUT_FILENAME: &str = "input.sjis.txt";

/// Corpus source backed by the in-tree `spec/aozora/fixtures/` layout.
///
/// Each immediate subdirectory of `root` is treated as one fixture.
/// A fixture contributes an item iff it contains a file named
/// [`FIXTURE_INPUT_FILENAME`]; missing the file means the fixture is
/// skipped silently (it may be a Tier-A-candidate that hasn't been
/// vendored yet, or a scratch dir).
#[derive(Debug, Clone)]
pub struct VendoredCorpus {
    root: PathBuf,
    provenance: String,
}

impl VendoredCorpus {
    /// Construct from a fixtures directory root (e.g.
    /// `spec/aozora/fixtures`). Rejects non-directory paths eagerly.
    ///
    /// # Errors
    ///
    /// Returns [`CorpusError::RootNotDirectory`] if `root` does not
    /// exist or is not a directory.
    pub fn load(root: impl Into<PathBuf>) -> Result<Self, CorpusError> {
        let root = root.into();
        if !root.is_dir() {
            return Err(CorpusError::RootNotDirectory { path: root });
        }
        let provenance = format!("vendored:{}", root.display());
        Ok(Self { root, provenance })
    }
}

impl CorpusSource for VendoredCorpus {
    fn iter(&self) -> Box<dyn Iterator<Item = Result<CorpusItem, CorpusError>> + '_> {
        Box::new(FixtureIter::new(&self.root))
    }

    fn provenance(&self) -> &str {
        &self.provenance
    }
}

/// Owning iterator over subdirectories of the fixtures root.
///
/// Kept as a concrete struct (rather than a chain of closures) so the
/// `Iterator::next` implementation can cleanly surface directory-read
/// failures as `Err` items and continue past sub-fixtures that lack
/// the expected input file.
struct FixtureIter {
    entries: Option<IntoIter<PathBuf>>,
    error_once: Option<CorpusError>,
}

impl FixtureIter {
    fn new(root: &Path) -> Self {
        match collect_fixture_dirs(root) {
            Ok(dirs) => Self {
                entries: Some(dirs.into_iter()),
                error_once: None,
            },
            Err(err) => Self {
                entries: None,
                error_once: Some(err),
            },
        }
    }
}

impl Iterator for FixtureIter {
    type Item = Result<CorpusItem, CorpusError>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(err) = self.error_once.take() {
            return Some(Err(err));
        }
        let entries = self.entries.as_mut()?;
        for dir in entries.by_ref() {
            let input = dir.join(FIXTURE_INPUT_FILENAME);
            if !input.is_file() {
                continue;
            }
            return Some(read_fixture(&dir, &input));
        }
        None
    }
}

fn collect_fixture_dirs(root: &Path) -> Result<Vec<PathBuf>, CorpusError> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(root)
        .map_err(|source| CorpusError::Io {
            path: root.to_path_buf(),
            source,
        })?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort(); // deterministic iteration order across platforms
    Ok(dirs)
}

fn read_fixture(dir: &Path, input: &Path) -> Result<CorpusItem, CorpusError> {
    let bytes = fs::read(input).map_err(|source| CorpusError::Io {
        path: input.to_path_buf(),
        source,
    })?;
    // `read_dir` yields `DirEntry`s that, when turned into paths, always
    // have a terminal file name. `file_name()` therefore cannot return
    // None for entries we reach here; encode that invariant as `expect`
    // so the "impossible" branch doesn't linger in the coverage data.
    let label = dir
        .file_name()
        .expect("read_dir entries always have a terminal file name")
        .to_string_lossy()
        .into_owned();
    Ok(CorpusItem::new(label, bytes))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn fresh_root() -> TempDir {
        tempfile::tempdir().expect("create tempdir")
    }

    fn seed_fixture(root: &Path, card_id: &str, bytes: &[u8]) {
        let dir = root.join(card_id);
        fs::create_dir_all(&dir).expect("create fixture dir");
        fs::write(dir.join(FIXTURE_INPUT_FILENAME), bytes).expect("write input");
    }

    #[test]
    fn load_rejects_nonexistent_root() {
        let err = VendoredCorpus::load("/no/such/dir").expect_err("must fail");
        assert!(matches!(err, CorpusError::RootNotDirectory { .. }));
    }

    #[test]
    fn load_rejects_file_path_as_root() {
        let dir = fresh_root();
        let file = dir.path().join("regular.txt");
        fs::write(&file, b"").expect("write file");
        let err = VendoredCorpus::load(&file).expect_err("file is not a directory");
        assert!(matches!(err, CorpusError::RootNotDirectory { path } if path == file));
    }

    #[test]
    fn iterates_fixtures_that_contain_input_sjis_txt() {
        let dir = fresh_root();
        seed_fixture(dir.path(), "56656", b"alpha bytes");
        seed_fixture(dir.path(), "00148", b"beta");

        let corpus = VendoredCorpus::load(dir.path()).expect("valid root");
        let items: Vec<CorpusItem> = corpus
            .iter()
            .map(|r| r.expect("valid fixture yields Ok"))
            .collect();
        assert_eq!(items.len(), 2);

        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"56656"));
        assert!(labels.contains(&"00148"));

        let sj = items
            .iter()
            .find(|i| i.label == "56656")
            .expect("56656 present");
        assert_eq!(sj.bytes, b"alpha bytes");
    }

    #[test]
    fn fixtures_without_input_sjis_txt_are_skipped() {
        let dir = fresh_root();
        seed_fixture(dir.path(), "56656", b"real fixture");

        // A sibling directory that does not contain the expected filename:
        // mimics a scratch dir that should not be exposed.
        let scratch = dir.path().join("scratch");
        fs::create_dir_all(&scratch).expect("create scratch dir");
        fs::write(scratch.join("golden.html"), b"<html>").expect("write golden");

        let corpus = VendoredCorpus::load(dir.path()).expect("valid root");
        let labels: Vec<String> = corpus
            .iter()
            .map(|r| r.expect("valid items Ok").label)
            .collect();
        assert_eq!(labels, vec!["56656".to_owned()]);
    }

    #[test]
    fn provenance_encodes_root_path() {
        let dir = fresh_root();
        let corpus = VendoredCorpus::load(dir.path()).expect("valid root");
        assert!(corpus.provenance().starts_with("vendored:"));
        assert!(
            corpus
                .provenance()
                .contains(&dir.path().display().to_string())
        );
    }

    #[test]
    fn iteration_is_deterministic() {
        let dir = fresh_root();
        // Create in a deliberately out-of-order sequence; the iterator
        // must sort so that tests don't flake across filesystems.
        seed_fixture(dir.path(), "bb", b"b");
        seed_fixture(dir.path(), "aa", b"a");
        seed_fixture(dir.path(), "cc", b"c");

        let corpus = VendoredCorpus::load(dir.path()).expect("valid root");
        let labels_first: Vec<String> = corpus.iter().map(|r| r.expect("ok").label).collect();
        let labels_second: Vec<String> = corpus.iter().map(|r| r.expect("ok").label).collect();
        assert_eq!(labels_first, labels_second);
        assert_eq!(labels_first, vec!["aa", "bb", "cc"]);
    }

    #[test]
    fn empty_root_yields_zero_items() {
        let dir = fresh_root();
        let corpus = VendoredCorpus::load(dir.path()).expect("valid root");
        assert_eq!(corpus.iter().count(), 0);
    }
}
