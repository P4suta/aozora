//! Persists resolved symbols through [`SymbolCache`] and validates library identities when applying them.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Trace;

#[cfg(unix)]
const CACHE_FILE_MODE: u32 = 0o666;

/// Resolved symbols keyed by library name and binary identity.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SymbolCache {
    /// Cached symbols keyed by library name.
    pub libs: HashMap<String, CachedLibrary>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CachedLibrary {
    pub debug_id: String,
    /// Resolved names keyed by decimal addresses.
    pub by_address: HashMap<String, String>,
}

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("cache file io: {0}")]
    Io(#[from] io::Error),
    #[error("cache file json: {0}")]
    Json(#[from] serde_json::Error),
}

impl SymbolCache {
    /// Conventional sidecar path: `<trace>.symbols.json` next to the trace file regardless of the trace's `.gz` extension.
    #[must_use]
    pub fn sidecar_path_for(trace_path: &Path) -> PathBuf {
        let mut p = trace_path.to_path_buf();
        if p.extension().and_then(|e| e.to_str()) == Some("gz") {
            p.set_extension("");
        }
        let stem = p
            .file_stem()
            .map_or_else(|| "trace".to_owned(), |s| s.to_string_lossy().into_owned());
        p.set_file_name(format!("{stem}.symbols.json"));
        p
    }

    /// Loads a sidecar, returning `Ok(None)` for a missing file.
    ///
    /// # Errors
    /// Returns an error if reading or parsing the file fails.
    pub fn load(path: &Path) -> Result<Option<Self>, CacheError> {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let cache: Self = serde_json::from_slice(&bytes)?;
        Ok(Some(cache))
    }

    /// Persists the cache with an atomic file replacement.
    ///
    /// # Errors
    /// Returns an error if serialization, writing, replacement, or directory synchronization fails.
    pub fn write(&self, path: &Path) -> Result<(), CacheError> {
        let bytes = serde_json::to_vec_pretty(self)?;
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let permissions = match fs::metadata(path) {
            Ok(metadata) => Some(metadata.permissions()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let mut builder = tempfile::Builder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            builder.permissions(fs::Permissions::from_mode(CACHE_FILE_MODE))
        };
        let mut temporary = builder.tempfile_in(parent)?;
        if let Some(permissions) = permissions {
            temporary.as_file().set_permissions(permissions)?;
        }
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        temporary.persist(path).map_err(|error| error.error)?;
        sync_directory(parent)?;
        Ok(())
    }

    /// Fills unresolved frames with cached symbols whose library identities match the trace.
    /// Returns the number of frames resolved.
    pub fn apply(&self, trace: &mut Trace) -> usize {
        let mut count = 0;
        let mut lib_lookup: Vec<Option<&CachedLibrary>> = Vec::with_capacity(trace.libs.len());
        for lib in &trace.libs {
            let identity = if lib.code_id.is_empty() {
                lib.debug_id.as_str()
            } else {
                lib.code_id.as_str()
            };
            let cached = self
                .libs
                .get(&lib.name)
                .filter(|cached| same_identity(&cached.debug_id, identity));
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

    /// Records a resolved address, clearing old symbols when the library identity changes.
    pub fn record(&mut self, lib: LibIdent<'_>, address: u64, name: String) {
        let entry = self
            .libs
            .entry(lib.name.to_owned())
            .or_insert_with(|| CachedLibrary {
                debug_id: lib.debug_id.to_owned(),
                by_address: HashMap::new(),
            });
        if !same_identity(&entry.debug_id, lib.debug_id) {
            entry.by_address.clear();
        }
        entry.debug_id.clear();
        entry.debug_id.push_str(lib.debug_id);
        entry.by_address.insert(address.to_string(), name);
    }
}

fn same_identity(left: &str, right: &str) -> bool {
    let left = normalise_identity(left);
    let right = normalise_identity(right);
    !left.is_empty() && left == right
}

fn normalise_identity(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_hexdigit)
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    fs::File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// Library identity passed to [`SymbolCache::record`].
#[derive(Debug, Clone, Copy)]
pub struct LibIdent<'a> {
    /// Library name matching [`crate::Library::name`].
    pub name: &'a str,
    /// The library's code-id, or debug-id fallback, stored so a later [`SymbolCache::apply`] can reject a cache entry whose binary has since been rebuilt.
    pub debug_id: &'a str,
}
