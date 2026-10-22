#![expect(
    clippy::expect_used,
    reason = "the fallback symbol regex is a valid static pattern"
)]

//! Resolves trace addresses through [`Symbolicator`] using debug information and dynamic symbols.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::{self, Write as _};
use std::fs;
use std::path::{Path, PathBuf};

use addr2line::Loader;
use object::{Object as _, ObjectKind, ObjectSymbol as _};

use crate::{SymbolCache, Trace};

/// Symbol resolution state for registered binaries.
#[derive(Default)]
pub struct Symbolicator {
    loaders: HashMap<String, Loader>,
    paths: HashMap<String, PathBuf>,
    build_ids: HashMap<String, String>,
    dyn_symbols: HashMap<String, DynSymbolTable>,
}

#[derive(Debug, Default)]
struct DynSymbolTable {
    entries: Vec<(u64, u64, String)>,
}

const DYNSYM_FUZZY_GAP_LIMIT: u64 = 256 * 1024;
const BUILD_ID_COMPARE_HEX_DIGITS: usize = 40;

impl DynSymbolTable {
    fn lookup(&self, address: u64) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }
        let idx = self
            .entries
            .partition_point(|(start, _, _)| *start <= address);
        if idx == 0 {
            return None;
        }
        let (start, end, name) = &self.entries[idx - 1];
        if address >= *start && address < *end {
            return Some(name.clone());
        }
        let gap = address - *start;
        (gap <= DYNSYM_FUZZY_GAP_LIMIT).then(|| format!("{name}+0x{gap:x}"))
    }
}

impl fmt::Debug for Symbolicator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Symbolicator")
            .field("loader_count", &self.loaders.len())
            .field("registered_libs", &self.paths)
            .field("build_ids", &self.build_ids)
            .field(
                "dyn_symbol_libs",
                &self
                    .dyn_symbols
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.entries.len()))
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// Symbolication failures from [`Symbolicator`].
#[derive(Debug, thiserror::Error)]
pub enum SymbolError {
    /// Loading the binary or its symbols failed.
    #[error("addr2line loader for {path}: {message}")]
    Loader {
        /// Path to the binary that failed to open.
        path: PathBuf,
        /// `addr2line`'s error rendered as a string.
        message: String,
    },
    /// The registered binary and recorded library have different build identifiers.
    #[error(
        "build-id mismatch for `{lib_name}`: trace expects `{trace}`, binary at {binary} is `{found}` — \
         the binary was rebuilt after the trace was recorded; symbolication would return wrong names. \
         Re-run samply against the current binary, or check out the commit that produced this trace."
    )]
    BuildIdMismatch {
        /// Library whose build-id failed to match.
        lib_name: String,
        /// Build identifier or debug identifier from the trace.
        trace: String,
        /// Build-id read from the registered binary on disk.
        found: String,
        /// Path to that binary.
        binary: PathBuf,
    },
}

impl Symbolicator {
    /// Empty symbolicator.
    /// Register binaries with [`Self::add_binary`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a binary to resolve addresses for libraries matching `lib_name`.
    ///
    /// # Errors
    /// Returns an error if `addr2line` fails to load the binary.
    pub fn add_binary(&mut self, lib_name: &str, binary_path: &Path) -> Result<(), SymbolError> {
        let loader = Loader::new(binary_path).map_err(|e| SymbolError::Loader {
            path: binary_path.to_path_buf(),
            message: e.to_string(),
        })?;
        let build_id = read_build_id(binary_path).unwrap_or_default();
        let dyn_table = read_dynamic_symbols(binary_path).unwrap_or_default();
        self.dyn_symbols.insert(lib_name.to_owned(), dyn_table);
        self.loaders.insert(lib_name.to_owned(), loader);
        self.paths
            .insert(lib_name.to_owned(), binary_path.to_path_buf());
        self.build_ids.insert(lib_name.to_owned(), build_id);
        Ok(())
    }

    /// Compares registered build identifiers with the trace's `code_id`, falling back to `debug_id`.
    /// Skips libraries without comparable identifiers.
    ///
    /// # Errors
    /// Returns [`SymbolError::BuildIdMismatch`] if a registered identity differs from the trace.
    pub fn verify_against(&self, trace: &Trace) -> Result<(), SymbolError> {
        for lib in &trace.libs {
            let Some(found) = self.build_ids.get(&lib.name) else {
                continue;
            };
            let trace_id = if !lib.code_id.is_empty() {
                lib.code_id.as_str()
            } else if !lib.debug_id.is_empty() {
                lib.debug_id.as_str()
            } else {
                continue;
            };
            let trace_norm = normalise_id(trace_id);
            let found_norm = normalise_id(found);
            if trace_norm.is_empty() || found_norm.is_empty() {
                continue;
            }
            let n = trace_norm
                .len()
                .min(found_norm.len())
                .min(BUILD_ID_COMPARE_HEX_DIGITS);
            if !trace_norm[..n].eq_ignore_ascii_case(&found_norm[..n]) {
                let path = self.paths.get(&lib.name).cloned().unwrap_or_default();
                return Err(SymbolError::BuildIdMismatch {
                    lib_name: lib.name.clone(),
                    trace: trace_id.to_owned(),
                    found: found.clone(),
                    binary: path,
                });
            }
        }
        Ok(())
    }

    /// Registers readable libraries from the trace and returns the number registered.
    /// Skips missing or unreadable binaries.
    pub fn add_libs_from(&mut self, trace: &Trace) -> usize {
        let mut count = 0;
        for lib in &trace.libs {
            let path = Path::new(&lib.path);
            if path.exists() && self.add_binary(&lib.name, path).is_ok() {
                count += 1;
            }
        }
        count
    }

    /// Registers a binary for resolution through its dynamic-symbol table.
    /// Returns the number of dynamic symbols loaded.
    ///
    /// # Errors
    /// Returns an error if reading or parsing the binary fails.
    pub fn add_binary_dynamic_only(
        &mut self,
        lib_name: &str,
        binary_path: &Path,
    ) -> Result<usize, SymbolError> {
        let dyn_table = read_dynamic_symbols(binary_path).ok_or_else(|| SymbolError::Loader {
            path: binary_path.to_path_buf(),
            message: "no readable .dynsym table".into(),
        })?;
        let count = dyn_table.entries.len();
        self.dyn_symbols.insert(lib_name.to_owned(), dyn_table);
        self.paths
            .insert(lib_name.to_owned(), binary_path.to_path_buf());
        Ok(count)
    }

    /// Resolves frames using debug information or dynamic symbols and records new names in `cache`.
    /// Returns `(resolved, attempted)` counts.
    pub fn resolve_into(&self, trace: &mut Trace, cache: &mut SymbolCache) -> (usize, usize) {
        let mut resolved = 0;
        let mut attempted = 0;
        for thread_idx in 0..trace.threads.len() {
            type LibBindings<'a> = Option<(
                Option<&'a Loader>,
                Option<&'a DynSymbolTable>,
                &'a str,
                &'a str,
            )>;
            let mut lib_bindings: Vec<LibBindings<'_>> = Vec::with_capacity(trace.libs.len());
            for lib in &trace.libs {
                let loader = self.loaders.get(&lib.name);
                let dyn_table = self.dyn_symbols.get(&lib.name);
                if loader.is_some() || dyn_table.is_some() {
                    let identity = if lib.code_id.is_empty() {
                        lib.debug_id.as_str()
                    } else {
                        lib.code_id.as_str()
                    };
                    lib_bindings.push(Some((loader, dyn_table, lib.name.as_str(), identity)));
                } else {
                    lib_bindings.push(None);
                }
            }
            let thread = &mut trace.threads[thread_idx];
            for frame_idx in 0..thread.frame_table.len() {
                if thread.resolved[frame_idx].is_some() {
                    continue;
                }
                let Some(lib_idx) = thread.frame_library(frame_idx) else {
                    continue;
                };
                let Some(Some((loader, dyn_table, lib_name, identity))) = lib_bindings.get(lib_idx)
                else {
                    continue;
                };
                attempted += 1;
                let addr = thread.frame_table[frame_idx].address;
                let name = loader
                    .and_then(|l| resolve_one(l, addr))
                    .or_else(|| dyn_table.and_then(|t| t.lookup(addr)));
                if let Some(name) = name {
                    cache.record(
                        crate::LibIdent {
                            name: lib_name,
                            debug_id: identity,
                        },
                        addr,
                        name.clone(),
                    );
                    thread.resolved[frame_idx] = Some(name);
                    resolved += 1;
                }
            }
        }
        (resolved, attempted)
    }
}

fn resolve_one(loader: &Loader, address: u64) -> Option<String> {
    let mut frames = loader.find_frames(address).ok()?;
    let outer = frames.next().ok().flatten()?;
    let func = outer.function?;
    func.demangle().ok().map(Cow::into_owned)
}

fn read_build_id(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let obj = object::File::parse(&bytes[..]).ok()?;
    if obj.kind() != ObjectKind::Executable && obj.kind() != ObjectKind::Dynamic {
        return None;
    }
    let bytes = obj.build_id().ok().flatten()?;
    let mut hex = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        write!(&mut hex, "{b:02x}").expect("String write_fmt is infallible");
    }
    Some(hex)
}

fn read_dynamic_symbols(path: &Path) -> Option<DynSymbolTable> {
    let bytes = fs::read(path).ok()?;
    let obj = object::File::parse(&bytes[..]).ok()?;
    let mut entries: Vec<(u64, u64, String)> = Vec::new();
    for sym in obj.dynamic_symbols() {
        let Ok(name) = sym.name() else { continue };
        if name.is_empty() {
            continue;
        }
        let Some((address, end)) = dynamic_symbol_range(sym.address(), sym.size()) else {
            continue;
        };
        let demangled = addr2line::demangle_auto(Cow::Borrowed(name), None).into_owned();
        entries.push((address, end, demangled));
    }
    entries.sort_by_key(|(start, _, _)| *start);
    Some(DynSymbolTable { entries })
}

fn dynamic_symbol_range(address: u64, size: u64) -> Option<(u64, u64)> {
    if address == 0 {
        return None;
    }
    address.checked_add(size.max(1)).map(|end| (address, end))
}

fn normalise_id(id: &str) -> String {
    id.chars()
        .filter(char::is_ascii_hexdigit)
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::{
        DYNSYM_FUZZY_GAP_LIMIT, DynSymbolTable, Symbolicator, dynamic_symbol_range, normalise_id,
    };
    use crate::Trace;

    fn table(entries: &[(u64, u64, &str)]) -> DynSymbolTable {
        DynSymbolTable {
            entries: entries
                .iter()
                .map(|(s, e, n)| (*s, *e, (*n).to_owned()))
                .collect(),
        }
    }

    #[test]
    fn dynsym_exact_range_match() {
        let t = table(&[(0x100, 0x200, "foo"), (0x200, 0x300, "bar")]);
        assert_eq!(t.lookup(0x150).as_deref(), Some("foo"), "in foo's range");
        assert_eq!(t.lookup(0x200).as_deref(), Some("bar"), "start of bar");
        assert_eq!(t.lookup(0x2ff).as_deref(), Some("bar"), "last byte of bar");
    }

    #[test]
    fn dynamic_symbol_range_rejects_zero_and_overflowing_addresses() {
        assert_eq!(dynamic_symbol_range(0, 10), None);
        assert_eq!(dynamic_symbol_range(0x100, 0), Some((0x100, 0x101)));
        assert_eq!(dynamic_symbol_range(u64::MAX, 1), None);
        assert_eq!(dynamic_symbol_range(u64::MAX - 1, 2), None);
    }

    #[test]
    fn dynsym_fuzzy_nearest_preceding_within_gap() {
        let t = table(&[(0x1000, 0x1000, "stub")]);
        assert_eq!(
            t.lookup(0x1010).as_deref(),
            Some("stub+0x10"),
            "fuzzy offset from nearest preceding symbol"
        );
    }

    #[test]
    fn dynsym_fuzzy_beyond_gap_limit_is_none() {
        let t = table(&[(0x1000, 0x1000, "stub")]);
        let far = 0x1000 + DYNSYM_FUZZY_GAP_LIMIT + 1;
        assert!(
            t.lookup(far).is_none(),
            "address past the fuzzy gap limit is unresolved"
        );
        let edge = 0x1000 + DYNSYM_FUZZY_GAP_LIMIT;
        assert!(t.lookup(edge).is_some(), "at the gap limit ⇒ resolved");
    }

    #[test]
    fn dynsym_before_first_symbol_is_none() {
        let t = table(&[(0x1000, 0x1100, "foo")]);
        assert!(
            t.lookup(0x500).is_none(),
            "address below the first symbol has no preceding entry"
        );
    }

    #[test]
    fn dynsym_empty_table_is_none() {
        let t = DynSymbolTable::default();
        assert!(t.lookup(0x10).is_none(), "empty table resolves nothing");
    }

    #[test]
    fn normalise_id_strips_dashes_and_lowercases() {
        assert_eq!(
            normalise_id("AB-CD-12"),
            "abcd12",
            "dashes dropped, hex lowercased"
        );
        assert_eq!(
            normalise_id("xyz123"),
            "123",
            "non-hex letters filtered out"
        );
    }

    fn trace_with_lib(name: &str, code_id: &str, debug_id: &str) -> Trace {
        let json = json!({
            "libs": [{
                "name": name,
                "path": "/x",
                "codeId": code_id,
                "breakpadId": debug_id,
            }],
            "threads": [],
        });
        Trace::from_json(&json, PathBuf::from("t")).expect("load")
    }

    #[test]
    fn verify_against_skips_unregistered_and_idless_libs() {
        let sym = Symbolicator::new();
        let t = trace_with_lib("bin", "deadbeef", "");
        assert!(
            sym.verify_against(&t).is_ok(),
            "unregistered lib is skipped"
        );
    }

    #[test]
    fn verify_against_matches_and_mismatches_build_id() {
        let mut sym = Symbolicator::new();
        sym.build_ids
            .insert("bin".to_owned(), "deadbeefcafe".to_owned());

        let good = trace_with_lib("bin", "deadbeefcafe", "");
        assert!(
            sym.verify_against(&good).is_ok(),
            "matching codeId verifies"
        );

        let idless = trace_with_lib("bin", "", "");
        assert!(sym.verify_against(&idless).is_ok(), "no trace id ⇒ skipped");

        let via_debug = trace_with_lib("bin", "", "DEAD-BEEF-CAFE");
        assert!(
            sym.verify_against(&via_debug).is_ok(),
            "breakpadId fallback verifies after normalisation"
        );

        let bad = trace_with_lib("bin", "00000000ffff", "");
        let err = sym.verify_against(&bad).expect_err("mismatch must error");
        assert!(
            err.to_string().contains("build-id mismatch"),
            "error is a build-id mismatch: {err}"
        );
    }
}
