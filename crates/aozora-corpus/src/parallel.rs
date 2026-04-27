//! Parallel I/O + decode helpers (L-2, ADR-0020).
//!
//! The default [`FilesystemCorpus::iter`] returns a streaming iterator
//! that fuses walkdir + read; this module exposes a path-first
//! variant that splits the two so the read step can be fanned out
//! across rayon workers while walkdir runs once on the calling
//! thread.
//!
//! ## Why "collect paths then parallel-read"
//!
//! `walkdir::WalkDir` is not parallel-safe — it owns the kernel
//! `DIR*` cursor and its iterator state is not `Sync`. Forking the
//! traversal across workers would either need a per-worker walker
//! (each scanning a disjoint subtree, which requires a sharding
//! decision the corpus shape doesn't make obvious) or a custom
//! parallel-aware walker. Neither pays here: the walkdir phase is
//! ~0.3 s on the 17 k-file corpus, well under any rayon coordination
//! overhead. Collecting paths sequentially and then fanning the
//! per-file read + caller closure is the embarrassingly-parallel
//! shape that rayon's `par_iter` handles best.
//!
//! ## Why per-item closure (`F: Fn(CorpusItem) -> T`)
//!
//! The caller decides what to do with each decoded item: bench
//! harnesses bucket by post-decode size, sweep tests run invariants,
//! cache builders write derived files. Inverting that — having this
//! module return `Vec<CorpusItem>` directly — would force the caller
//! to walk the vec a second time to do its own work, doubling the
//! read+decode amortisation cost from the rayon pool's per-worker
//! arena warmth that the closure can capture (e.g. an L-3
//! [`thread_local!`] decode buffer) into a per-call cold one. The
//! closure shape lets the caller participate in the parallel pass.

use std::path::PathBuf;

use rayon::prelude::*;

use crate::{CorpusError, CorpusItem, FilesystemCorpus};

/// Walk the corpus, read every file, and apply `per_item` to each
/// `CorpusItem` in parallel via rayon.
///
/// The walkdir traversal runs once on the calling thread (it is not
/// `Sync`); the read + closure runs on rayon's work-stealing pool.
/// Per-item I/O failures land as `Err` entries in the returned `Vec`,
/// preserving the same "skip-or-fail is the caller's call" contract
/// as the sequential `iter()` path.
///
/// Output order is rayon's collect order (matches the input
/// `Vec<PathBuf>` order, which is walkdir's directory-entry order —
/// `readdir(2)`-defined, OS- and filesystem-dependent, NOT
/// lexicographic). Per-thread accumulator merging happens inside
/// rayon's parallel-iterator collect — the caller does not see
/// intermediate per-thread shards. If the caller needs deterministic
/// output ordering, sort the returned `Vec` by `CorpusItem.label`
/// before consuming it.
///
/// # Errors
///
/// Per-item errors are returned inline as `Err` entries; this function
/// itself never returns `Err`. Walkdir-level errors (permission denied
/// on a subdirectory, broken symlink) likewise surface as `Err`
/// entries in their walkdir position.
pub fn par_load_decoded<F, T>(corpus: &FilesystemCorpus, per_item: F) -> Vec<Result<T, CorpusError>>
where
    F: Fn(CorpusItem) -> T + Sync + Send,
    T: Send,
{
    // Step 1: serial walkdir to a Vec<PathBuf>. Cheap (~0.3 s on
    // 17 k files thanks to the `is_text_dir_entry` no-extra-stat
    // path); not parallelisable because walkdir is `!Sync`.
    let paths: Vec<Result<PathBuf, CorpusError>> = corpus.walk_paths().collect();

    // Step 2: parallel read + decode + caller closure. Each rayon
    // worker pulls a `Result<PathBuf, _>`, propagates walkdir errors
    // unchanged, otherwise reads the bytes and runs the closure.
    paths
        .into_par_iter()
        .map(|path_result| {
            let path = path_result?;
            let item = corpus.read_path(&path)?;
            Ok(per_item(item))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

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
        // 5, 4, 5, 5 sorted = 4, 5, 5, 5
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
        // Many files so rayon definitely fans across workers; verify
        // that every expected label survives the parallel collect
        // (set equality — walkdir's directory-entry order is
        // OS-dependent so we don't pin sequence, only membership).
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
}
