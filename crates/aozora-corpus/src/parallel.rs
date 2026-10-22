use std::path::PathBuf;
use std::sync::OnceLock;

use rayon::prelude::*;
use rayon::{ThreadPool, ThreadPoolBuilder};

use crate::{CorpusError, CorpusItem, FilesystemCorpus};

// Corpus loading measured slower with logical-core workers than with physical-core workers on WSL2.
fn physical_core_pool() -> Option<&'static ThreadPool> {
    static POOL: OnceLock<Option<ThreadPool>> = OnceLock::new();
    POOL.get_or_init(|| {
        let n = num_cpus::get_physical().max(1);
        ThreadPoolBuilder::new()
            .num_threads(n)
            .thread_name(|i| format!("aozora-corpus-load-{i}"))
            .build()
            .ok()
    })
    .as_ref()
}

/// Run `f` on the physical-core load pool.
///
/// Waits for `f` to return and falls back to the calling thread if pool construction fails.
pub fn with_load_pool<R, F>(f: F) -> R
where
    F: FnOnce() -> R + Send,
    R: Send,
{
    match physical_core_pool() {
        Some(pool) => pool.install(f),
        None => f(),
    }
}

/// Read corpus files and apply `per_item` on the shared load pool.
///
/// Output follows filesystem order.
/// Processing falls back to the calling thread if pool construction fails.
///
/// # Errors
///
/// Returns traversal and file I/O failures as inline error entries.
pub fn par_load_decoded<F, T>(corpus: &FilesystemCorpus, per_item: F) -> Vec<Result<T, CorpusError>>
where
    F: Fn(CorpusItem) -> T + Sync + Send,
    T: Send,
{
    let paths: Vec<Result<PathBuf, CorpusError>> = corpus.walk_paths().collect();

    load_decoded_with_pool(corpus, paths, &per_item, physical_core_pool())
}

fn load_decoded_with_pool<F, T>(
    corpus: &FilesystemCorpus,
    paths: Vec<Result<PathBuf, CorpusError>>,
    per_item: &F,
    pool: Option<&ThreadPool>,
) -> Vec<Result<T, CorpusError>>
where
    F: Fn(CorpusItem) -> T + Sync + Send,
    T: Send,
{
    match pool {
        Some(pool) => pool.install(|| {
            paths
                .into_par_iter()
                .map(|path_result| load_one(corpus, per_item, path_result))
                .collect()
        }),
        None => paths
            .into_iter()
            .map(|path_result| load_one(corpus, per_item, path_result))
            .collect(),
    }
}

fn load_one<F, T>(
    corpus: &FilesystemCorpus,
    per_item: &F,
    path_result: Result<PathBuf, CorpusError>,
) -> Result<T, CorpusError>
where
    F: Fn(CorpusItem) -> T,
{
    let path = path_result?;
    let item = corpus.read_path(&path)?;
    Ok(per_item(item))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::thread;

    use tempfile::TempDir;

    use super::*;

    fn fresh_root_with(files: &[(&str, &[u8])]) -> TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        for (rel, bytes) in files {
            let path = dir.path().join(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("mkdir");
            }
            fs::write(&path, bytes).expect("write");
        }
        dir
    }

    #[test]
    fn par_load_decoded_visits_every_file() {
        let dir = fresh_root_with(&[
            ("a.txt", b"alpha"),
            ("b.txt", b"beta"),
            ("c.txt", b"gamma"),
            ("nested/d.txt", b"delta"),
        ]);
        let corpus = FilesystemCorpus::new(dir.path()).expect("corpus");

        let mut results: Vec<usize> = par_load_decoded(&corpus, |item| item.bytes.len())
            .into_iter()
            .filter_map(Result::ok)
            .collect();
        results.sort_unstable();
        assert_eq!(results, vec![4, 5, 5, 5]);
    }

    #[test]
    fn par_load_decoded_returns_empty_for_empty_corpus() {
        let dir = tempfile::tempdir().expect("tempdir");
        let corpus = FilesystemCorpus::new(dir.path()).expect("corpus");
        let results = par_load_decoded(&corpus, |item| item.bytes.len());
        assert!(results.is_empty());
    }

    #[test]
    fn par_load_decoded_skips_non_txt_files() {
        let dir = fresh_root_with(&[
            ("keep.txt", b"yes"),
            ("skip.md", b"no"),
            ("skip.bin", &[0xFF, 0xFE]),
        ]);
        let corpus = FilesystemCorpus::new(dir.path()).expect("corpus");
        let results = par_load_decoded(&corpus, |item| (item.label, item.bytes.len()));
        let kept: Vec<_> = results.into_iter().filter_map(Result::ok).collect();
        assert_eq!(kept, vec![(String::from("keep.txt"), 3)]);
    }

    #[test]
    fn par_load_decoded_visits_every_file_at_scale() {
        let files: Vec<(String, Vec<u8>)> = (0..50)
            .map(|i| (format!("doc-{i:03}.txt"), format!("body-{i}").into_bytes()))
            .collect();
        let file_refs: Vec<(&str, &[u8])> = files
            .iter()
            .map(|(p, b)| (p.as_str(), b.as_slice()))
            .collect();
        let dir = fresh_root_with(&file_refs);
        let corpus = FilesystemCorpus::new(dir.path()).expect("corpus");

        let mut labels: Vec<String> = par_load_decoded(&corpus, |item| item.label)
            .into_iter()
            .filter_map(Result::ok)
            .collect();
        labels.sort();
        let mut expected: Vec<String> = files.iter().map(|(p, _)| p.clone()).collect();
        expected.sort();
        assert_eq!(labels, expected);
    }

    #[test]
    fn pool_construction_failure_falls_back_to_the_calling_thread() {
        let dir = fresh_root_with(&[("a.txt", b"alpha"), ("b.txt", b"beta")]);
        let corpus = FilesystemCorpus::new(dir.path()).expect("corpus");
        let caller = thread::current().id();
        let paths = corpus.walk_paths().collect();

        let observed = load_decoded_with_pool(&corpus, paths, &|_| thread::current().id(), None);

        assert_eq!(observed.len(), 2);
        assert!(
            observed
                .into_iter()
                .all(|result| result.is_ok_and(|thread| thread == caller))
        );
    }
}
