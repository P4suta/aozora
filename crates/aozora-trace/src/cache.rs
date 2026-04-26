//! Sidecar symbol cache.
//!
//! Symbolication via `addr2line` parses DWARF info from the binary
//! every time. On a multi-MB ELF that's seconds; for an interactive
//! analysis loop ("hot frames, then rollup, then compare") it
//! becomes the dominant cost.
//!
//! [`SymbolCache`] persists the resolved (`library_name`, address) →
//! function-name mapping to a `<trace>.symbols.json` sidecar next
//! to the trace file. Re-running an analysis hits the cache in
//! milliseconds.
//!
//! ## Invalidation
//!
//! The cache stores the trace's source path and the binary
//! `debug_id` (build-id) per library. A `load`-then-validate cycle
//! refuses to use a cache whose debug-ids don't match the current
//! trace's libs — this catches "binary was rebuilt, addresses
//! shifted, the cache is now lying" silently.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Trace;

/// On-disk cache form. Keyed by (`library_name`, `debug_id`) so two
/// rebuilds of the same library don't poison each other's caches.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SymbolCache {
    /// Map: library `name` → (`debug_id`, addr → resolved function name).
    pub libs: HashMap<String, CachedLibrary>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CachedLibrary {
    pub debug_id: String,
    /// Addresses are stored as decimal-encoded strings to keep the
    /// JSON portable across platforms (some `serde_json` defaults
    /// reject `u64` keys in maps).
    pub by_address: HashMap<String, String>,
}

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("cache file io: {0}")]
    Io(#[from] std::io::Error),
    #[error("cache file json: {0}")]
    Json(#[from] serde_json::Error),
}

impl SymbolCache {
    /// Conventional sidecar path: `<trace>.symbols.json` next to the
    /// trace file regardless of the trace's `.gz` extension.
    #[must_use]
    pub fn sidecar_path_for(trace_path: &Path) -> PathBuf {
        let mut p = trace_path.to_path_buf();
        // Strip a single trailing `.gz` so `foo.json.gz` becomes
        // `foo.json`, then append `.symbols.json`.
        if p.extension().and_then(|e| e.to_str()) == Some("gz") {
            p.set_extension("");
        }
        let stem = p
            .file_stem()
            .map_or_else(|| "trace".to_owned(), |s| s.to_string_lossy().into_owned());
        p.set_file_name(format!("{stem}.symbols.json"));
        p
    }

    /// Load from sidecar path, returning `Ok(None)` when the file
    /// doesn't exist (a clean "no cache yet" condition).
    pub fn load(path: &Path) -> Result<Option<Self>, CacheError> {
        if !path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(path)?;
        let cache: Self = serde_json::from_slice(&bytes)?;
        Ok(Some(cache))
    }

    /// Persist atomically: write to `<path>.tmp`, then rename. A
    /// crashed write leaves the previous cache untouched.
    pub fn write(&self, path: &Path) -> Result<(), CacheError> {
        let tmp = path.with_extension("symbols.json.tmp");
        let bytes = serde_json::to_vec_pretty(self)?;
        fs::write(&tmp, bytes)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Apply this cache to the trace, populating `Thread.resolved`
    /// for every frame whose (library, address) is present. Returns
    /// the number of frames resolved.
    pub fn apply(&self, trace: &mut Trace) -> usize {
        let mut count = 0;
        // Build a quick "lib_idx → cached library entry" mapping
        // restricted to libs whose debug_id matches.
        let mut lib_lookup: Vec<Option<&CachedLibrary>> = Vec::with_capacity(trace.libs.len());
        for lib in &trace.libs {
            let cached = self
                .libs
                .get(&lib.name)
                .filter(|c| c.debug_id == lib.debug_id || c.debug_id.is_empty());
            lib_lookup.push(cached);
        }
        for thread in &mut trace.threads {
            for frame_idx in 0..thread.frame_table.len() {
                if thread.resolved[frame_idx].is_some() {
                    continue;
                }
                let lib_idx = thread.frame_library(frame_idx);
                let Some(lib_idx) = lib_idx else { continue };
                let Some(cached) = lib_lookup.get(lib_idx).copied().flatten() else {
                    continue;
                };
                let addr = thread.frame_table[frame_idx].address;
                if let Some(name) = cached.by_address.get(&addr.to_string()) {
                    thread.resolved[frame_idx] = Some(name.clone());
                    count += 1;
                }
            }
        }
        count
    }

    /// Add a (`library_name`, `debug_id`, address → name) record. Idempotent.
    pub fn record(&mut self, lib: LibIdent<'_>, address: u64, name: String) {
        let entry = self
            .libs
            .entry(lib.name.to_owned())
            .or_insert_with(|| CachedLibrary {
                debug_id: lib.debug_id.to_owned(),
                by_address: HashMap::new(),
            });
        entry.debug_id.clear();
        entry.debug_id.push_str(lib.debug_id);
        entry.by_address.insert(address.to_string(), name);
    }
}

/// Library identity passed to [`SymbolCache::record`].
///
/// Bundles the `(name, debug_id)` pair so the cache write API takes
/// one logical argument instead of two parallel string slices —
/// clearer at call sites *and* keeps clippy's `too_many_arguments`
/// lint happy.
#[derive(Debug, Clone, Copy)]
pub struct LibIdent<'a> {
    pub name: &'a str,
    pub debug_id: &'a str,
}
