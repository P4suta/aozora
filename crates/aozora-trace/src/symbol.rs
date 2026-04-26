//! Symbolicator — DWARF resolution via [`addr2line::Loader`].
//!
//! The samply gecko trace stores raw addresses (file-offset relative
//! to the loaded library). To get function names we open the binary
//! with `addr2line::Loader`, which mmaps the ELF + parses DWARF in
//! a single zero-copy pass.
//!
//! Demangling is done by `addr2line` itself (Rust + C++ symbols).
//!
//! ## Architecture
//!
//! - [`Symbolicator`] owns one `Loader` per binary path the user
//!   registers via [`Symbolicator::add_binary`].
//! - [`Symbolicator::resolve_into`] walks every frame in every
//!   thread of a [`Trace`], looks up its address in the appropriate
//!   loader, and writes the demangled function name into
//!   `Thread.resolved`.
//! - Resolved names are also recorded into the supplied
//!   [`crate::SymbolCache`] for the next run.
//!
//! ## Inlining
//!
//! `Loader::find_frames` returns an iterator of inline frames at a
//! single address. We currently take the *outermost* function name
//! (matches `addr2line -f` default). An "include inlines" mode
//! could be added by joining names with ` -> `; left out until a
//! profile shows we need it.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use addr2line::Loader;
use object::{Object as _, ObjectKind, ObjectSymbol as _};

use crate::{SymbolCache, Trace};

/// Symbolicator state — one [`Loader`] per registered binary.
#[derive(Default)]
pub struct Symbolicator {
    /// Map: library `name` (matching `Library.name` in the trace)
    /// → opened DWARF context.
    loaders: HashMap<String, Loader>,
    /// Track which paths we registered, for diagnostic prints.
    paths: HashMap<String, PathBuf>,
    /// Build-id (gnu .note.gnu.build-id) per registered library —
    /// used to validate against the trace's recorded `debug_id`.
    build_ids: HashMap<String, String>,
    /// Dynamic symbol table per registered library, sorted by start
    /// address. Used as the *fallback* when DWARF resolution returns
    /// `None` — picks up `memcpy` / `malloc` / etc. in stripped
    /// system libraries (libc.so.6 doesn't ship DWARF debuginfo on
    /// most distros without the libc6-dbg package).
    dyn_symbols: HashMap<String, DynSymbolTable>,
}

/// Sorted (`start_address`, `end_address`, `demangled_name`) triples
/// extracted from an ELF's `.dynsym` table. Lookup is binary-search
/// over `start` then range-check.
#[derive(Debug, Default)]
struct DynSymbolTable {
    entries: Vec<(u64, u64, String)>,
}

/// Maximum distance (bytes) for the fuzzy fallback to attribute an
/// unresolved address to the nearest preceding symbol. 256 KiB
/// covers libc's malloc-area gap (the internal allocator code lives
/// in a contiguous block after `__default_morecore` with no
/// `.dynsym` entries pointing into it). Bigger than the largest
/// reasonable single function body but small enough that we don't
/// label random gaps as `+ huge_offset`.
const DYNSYM_FUZZY_GAP_LIMIT: u64 = 256 * 1024;

impl DynSymbolTable {
    /// Resolve `address` to a function name.
    ///
    /// Two-tier:
    /// 1. **Exact range match** — `address ∈ [start, end)` for a
    ///    symbol with a non-zero size. This is what addr2line gives
    ///    you for fully-DWARF'd binaries; here it covers symbols
    ///    whose ELF size is known.
    /// 2. **Fuzzy nearest-preceding** — for libc-style stripped
    ///    libraries where IFUNC stubs and internal hidden helpers
    ///    leave most code-bytes outside any `.dynsym` range, fall
    ///    back to the nearest preceding symbol + offset, capped at
    ///    [`DYNSYM_FUZZY_GAP_LIMIT`]. Returns names like
    ///    `__libc_malloc+0x582` instead of bare `0xac098`.
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
        // Fuzzy fallback: address is past `end` (or `end == start`,
        // which happens for IFUNC stubs with size 0).
        let gap = address - *start;
        (gap <= DYNSYM_FUZZY_GAP_LIMIT).then(|| format!("{name}+0x{gap:x}"))
    }
}

impl std::fmt::Debug for Symbolicator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Symbolicator")
            // `loaders` is intentionally summarised as a count —
            // `addr2line::Loader` doesn't implement Debug and a
            // verbose dump would just print the same paths again.
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

#[derive(Debug, thiserror::Error)]
pub enum SymbolError {
    #[error("addr2line loader for {path}: {message}")]
    Loader { path: PathBuf, message: String },
    #[error(
        "build-id mismatch for `{lib_name}`: trace expects `{trace}`, binary at {binary} is `{found}` — \
         the binary was rebuilt after the trace was recorded; symbolication would return wrong names. \
         Re-run samply against the current binary, or check out the commit that produced this trace."
    )]
    BuildIdMismatch {
        lib_name: String,
        trace: String,
        found: String,
        binary: PathBuf,
    },
}

impl Symbolicator {
    /// Empty symbolicator. Register binaries with [`Self::add_binary`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a binary that will resolve addresses for all libs
    /// matching `lib_name`. Typically called once per non-system
    /// library; pre-built ELFs of `libc.so.6` etc. are usually not
    /// resolvable and are silently left as raw addresses.
    ///
    /// Also reads the binary's gnu-build-id (`.note.gnu.build-id`)
    /// so [`Self::verify_against`] can detect a stale binary →
    /// trace mismatch (PROFILING.md pitfall #5: rebuilding the
    /// binary after recording silently shifts addresses).
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

    /// Verify every registered binary's build-id matches the trace's
    /// recorded `code_id` (or `debug_id` as fallback) for the same
    /// library. Returns `Ok(())` if every match is good (or the
    /// trace had no identifier to compare against),
    /// [`SymbolError::BuildIdMismatch`] otherwise.
    pub fn verify_against(&self, trace: &Trace) -> Result<(), SymbolError> {
        for lib in &trace.libs {
            let Some(found) = self.build_ids.get(&lib.name) else {
                continue;
            };
            // samply records the gnu-build-id in `codeId`. Some
            // older/other recorders use `debugId` (breakpad-style:
            // uppercase 32 hex + 8 hex age). Try both; first non-empty
            // wins.
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
            // Compare the first 40 hex chars (full sha-1 length the
            // gnu-build-id uses by default). Truncated forms still
            // match if the prefix agrees.
            let n = trace_norm.len().min(found_norm.len()).min(40);
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

    /// Auto-register every library in `trace` whose `path` exists on
    /// disk. Convenience for "I just want to symbolicate this trace,
    /// figure it out yourself." Returns the count of registered
    /// libraries. Unreadable libraries are silently skipped — that's
    /// the whole point of the auto-discovery convenience.
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

    /// Register a binary using *only* the dynamic-symbol table —
    /// no DWARF expected. Convenient for stripped system libraries
    /// (`libc.so.6`, `ld-linux-x86-64.so.2`, …) that ship `.dynsym`
    /// but no debuginfo. Returns the number of dynamic symbols
    /// loaded.
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

    /// Walk every unresolved frame in `trace` and resolve via the
    /// appropriate loader, then via the dynamic-symbol fallback for
    /// libraries that have no DWARF (system libs). Returns
    /// `(resolved, attempted)` counts. Records each new name into
    /// `cache`.
    pub fn resolve_into(&self, trace: &mut Trace, cache: &mut SymbolCache) -> (usize, usize) {
        let mut resolved = 0;
        let mut attempted = 0;
        // Build a per-thread "library_idx → (loader?, dyn_table?,
        // name, debug_id)" lookup once. Either source is optional —
        // a library may have only DWARF, only dynsym, or both.
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
                    lib_bindings.push(Some((
                        loader,
                        dyn_table,
                        lib.name.as_str(),
                        lib.debug_id.as_str(),
                    )));
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
                let Some(Some((loader, dyn_table, lib_name, debug_id))) = lib_bindings.get(lib_idx)
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
                            debug_id,
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
    // `find_frames` yields inline frames — take the outermost.
    let mut frames = loader.find_frames(address).ok()?;
    let outer = frames.next().ok().flatten()?;
    let func = outer.function?;
    func.demangle().ok().map(std::borrow::Cow::into_owned)
}

/// Read the gnu build-id from `.note.gnu.build-id` in the ELF.
/// Returns lowercase hex, no leading 0x. Empty on any failure
/// (build-id is optional in ELFs).
fn read_build_id(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let obj = object::File::parse(&bytes[..]).ok()?;
    if obj.kind() != ObjectKind::Executable && obj.kind() != ObjectKind::Dynamic {
        return None;
    }
    let bytes = obj.build_id().ok().flatten()?;
    let mut hex = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        // write! into a String never fails; the result is asserted
        // away (rather than `let _ =`'d) to satisfy the
        // `let_underscore_must_use` workspace lint.
        write!(&mut hex, "{b:02x}").expect("String write_fmt is infallible");
    }
    Some(hex)
}

/// Read the ELF dynamic-symbol table (`.dynsym`) and return a
/// sorted (`start`, `end`, `demangled_name`) triple table for
/// fallback address → name resolution when DWARF is unavailable.
///
/// `.dynsym` is present on every dynamically-linked ELF, even when
/// the binary is `strip`'d — it's required by the runtime linker
/// for symbol relocation. So `libc.so.6` (typically stripped)
/// yields ~2300 exported symbols including `memcpy` / `memmove` /
/// `malloc` / etc., which is what shows up hot in our traces.
///
/// Returns `None` only if the file isn't a readable ELF.
fn read_dynamic_symbols(path: &Path) -> Option<DynSymbolTable> {
    let bytes = fs::read(path).ok()?;
    let obj = object::File::parse(&bytes[..]).ok()?;
    let mut entries: Vec<(u64, u64, String)> = Vec::new();
    for sym in obj.dynamic_symbols() {
        let Ok(name) = sym.name() else { continue };
        if name.is_empty() {
            continue;
        }
        let address = sym.address();
        if address == 0 {
            continue;
        }
        let size = sym.size().max(1);
        // Best-effort Rust demangle (no-op for C names like `malloc`).
        let demangled =
            addr2line::demangle_auto(std::borrow::Cow::Borrowed(name), None).into_owned();
        entries.push((address, address + size, demangled));
    }
    entries.sort_by_key(|(start, _, _)| *start);
    Some(DynSymbolTable { entries })
}

/// Strip dashes and lowercase. Breakpad and GNU forms differ in
/// case + dash placement; this normalisation makes them comparable.
fn normalise_id(id: &str) -> String {
    id.chars()
        .filter(char::is_ascii_hexdigit)
        .flat_map(char::to_lowercase)
        .collect()
}
