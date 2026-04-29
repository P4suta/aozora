//! Function-name categoriser.
//!
//! Bucketise function names into named groups. Drives the `rollup`
//! analysis ("how much CPU does *Phase 1* spend?" rather than
//! "how much CPU does specifically `aho_corasick::packed::teddy::…`
//! spend?").
//!
//! ## Two ways to define categories
//!
//! - **Built-in defaults** ([`RollupConfig::aozora_defaults`]):
//!   matches the project's known phase / library structure. Good
//!   for "I just want a phase breakdown of my own trace right now".
//! - **TOML config** ([`RollupConfig::from_toml`]): users / projects
//!   override or supply their own. The format is intentionally
//!   simple — a list of categories, each with a name + ordered
//!   list of regex patterns. First-match-wins.

use std::path::Path;

use regex::Regex;

/// Built-in category table for [`RollupConfig::aozora_defaults`].
/// First-match wins, in declaration order. The ordering matters —
/// e.g. AVX2 SIMD intrinsics are attributed to `phase1_scan`
/// (because that's where the only AVX2 use is) BEFORE the generic
/// `core_simd_intrinsics` catch-all sees them.
const AOZORA_DEFAULT_CATEGORIES: &[(&str, &[&str])] = &[
    (
        "phase1_scan",
        &[
            r"aho_corasick::packed",
            r"aozora_scan::backends",
            r"aozora_scan::naive",
            // AVX2 SIMD primitives only used inside Teddy / packed
            // (the production scanner). Attribute to the caller.
            r"core::core_arch::x86_64::avx::_mm256_extract",
            r"core::core_arch::x86::avx2::_mm256_movemask",
            r"core::core_arch::x86::avx2::_mm256_cmpeq",
            r"core::core_arch::x86::avx2::_mm256_shuffle",
            r"core::core_arch::x86::avx2::_mm256_or",
            r"core::core_arch::x86::avx2::_mm256_and",
            r"core::core_arch::x86::avx2::_mm256_alignr",
        ],
    ),
    ("phase1_walker", &[r"aozora_lexer::phase1_events"]),
    (
        "phase0_sanitize",
        &[r"aozora_lexer::sanitize", r"aozora_lexer::phase0"],
    ),
    ("phase2_pair", &[r"aozora_lexer::phase2_pair"]),
    (
        "phase3_classify",
        &[r"aozora_lexer::phase3", r"aozora_lexer::recognise"],
    ),
    (
        "phase4_intern",
        &[r"aozora_syntax::borrowed::intern", r"Interner"],
    ),
    (
        "memchr_scan",
        &[
            r"memchr::arch",
            r"memchr::memmem",
            r"memchr::vector",
            r"core::slice::memchr",
        ],
    ),
    (
        "corpus_load_sjis",
        &[
            r"encoding_rs",
            r"aozora_encoding",
            r"shift_jis",
            r"Utf8Bmp",
            r"Utf8Destination",
        ],
    ),
    ("corpus_walk", &[r"aozora_corpus", r"walkdir"]),
    (
        "pipeline_orchestration",
        &[
            r"aozora_lex::pipeline",
            r"aozora_lex::borrowed",
            r"run_to_completion",
        ],
    ),
    (
        "hashing",
        &[
            r"core::hash",
            r"hashbrown",
            r"siphasher",
            r"phf::map",
            r"phf_shared",
        ],
    ),
    // Allocation buckets — split the monolithic "allocation" category
    // into hot-path-attributable sub-buckets. The first-match-wins
    // regex order matters: more-specific patterns above the generic
    // ones.
    (
        // Bumpalo arena allocator: every `Arena::alloc*`, BumpVec
        // push/extend, and the underlying Bump chunk-allocation /
        // chunk-extend helpers. Counts the cost of "where in the
        // arena did this go" — the cost we *can* attack with
        // pooling, capacity hints, or chunk-size tuning.
        "alloc_bumpalo_arena",
        &[r"bumpalo::", r"bumpalo_collections::", r"BumpVec"],
    ),
    (
        // libc malloc/free/realloc family. Hits here are the cost of
        // bumpalo's chunk-extend `mmap` syscalls, plus any heap-Vec
        // allocations in Pipeline / diagnostics / interner growth.
        "alloc_libc_heap",
        &[
            r"^malloc$",
            r"^realloc$",
            r"^calloc$",
            r"^free$",
            r"^cfree$",
            r"^aligned_alloc$",
            r"^posix_memalign$",
            r"^__libc_malloc",
            r"^__libc_free",
            r"^__libc_calloc",
            r"^__libc_realloc",
            r"^_int_malloc",
            r"^_int_free",
            r"^_int_realloc",
            r"^arena_",
            r"^malloc_consolidate",
            r"^__default_morecore",
        ],
    ),
    (
        // libc memcpy/memmove/memset and their AVX2 dispatch. These
        // surface inside Vec::push grow paths, BumpVec re-grow
        // moves, hashmap rehash, and string clone. Separate bucket
        // so we can tell "the alloc allocator is busy" (heap)
        // apart from "the move-after-alloc is busy" (memcpy).
        "alloc_memcpy_memmove",
        &[
            r"^memmove",
            r"^memcpy",
            r"^memset",
            r"^bzero",
            r"^__memmove",
            r"^__memcpy",
            r"^__memset",
            r"^__nss_database_lookup\+",
        ],
    ),
    (
        // Rust std heap-Vec / String / HashMap / VecDeque / etc.
        // These are the heap allocators the *non-arena* code path
        // uses — Pipeline's `Vec<Diagnostic>`, the corpus iter's
        // `Vec<CorpusItem>`, the `HashMap<String, u32>` in the
        // forward-target index. Address of attack: pre-size or move
        // to arena.
        "alloc_rust_std",
        &[
            r"alloc::vec",
            r"alloc::raw_vec",
            r"alloc::string",
            r"alloc::collections",
            r"alloc::alloc",
            r"smallvec::",
            r"__rust_alloc",
            r"__rust_dealloc",
            r"GlobalAlloc",
        ],
    ),
    (
        "io_syscalls",
        &[
            r"^__read$",
            r"^__write$",
            r"^read$",
            r"^write$",
            r"^open(at)?$",
            r"^close$",
            r"^fstat",
            r"^lseek",
            r"^mmap",
            r"^munmap",
            r"^brk$",
            r"^sbrk$",
            r"^syscall",
            r"^__libc_read",
            r"^__libc_write",
        ],
    ),
    ("rendering", &[r"aozora_render"]),
    // Generic helpers that can't be attributed to a specific phase
    // because the same primitive is called from many places. Useful
    // to surface as a single bucket so they don't drown in `unknown`.
    (
        "core_ptr_ops",
        &[
            r"core::ptr::write",
            r"core::ptr::read",
            r"core::ptr::const_ptr",
            r"core::ptr::mut_ptr",
            r"core::ptr::non_null",
        ],
    ),
    (
        "core_slice_ops",
        &[
            r"core::slice::cmp",
            r"core::slice::index",
            r"core::slice::iter",
            r"core::str::pattern",
            r"core::str::iter",
            r"core::str::traits",
            r"core::ops::range",
        ],
    ),
    (
        "core_arith",
        &[
            r"<u8>",
            r"<u16>",
            r"<u32>",
            r"<u64>",
            r"<usize>",
            r"<i8>",
            r"<i16>",
            r"<i32>",
            r"<i64>",
            r"<isize>",
            r"core::intrinsics",
            r"core::num",
        ],
    ),
    (
        "core_misc",
        &[
            r"core::option",
            r"core::result",
            r"core::cmp",
            r"core::convert",
            r"core::iter",
            r"core::mem",
            r"core::unicode",
            r"core::char",
            r"core::fmt",
            r"core::str::validations",
            r"core::ops",
        ],
    ),
];

#[derive(Debug, thiserror::Error)]
pub enum CategoryError {
    #[error("toml parse: {0}")]
    Toml(String),
    #[error("regex `{pattern}` for category `{name}`: {error}")]
    Regex {
        name: String,
        pattern: String,
        #[source]
        error: regex::Error,
    },
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// User-supplied category definitions. Compile to a [`Categorizer`]
/// once and reuse across analyses.
#[derive(Debug, Clone)]
pub struct RollupConfig {
    pub categories: Vec<CategorySpec>,
}

#[derive(Debug, Clone)]
pub struct CategorySpec {
    pub name: String,
    pub patterns: Vec<String>,
}

impl RollupConfig {
    /// Built-in defaults tuned for the aozora workspace's phase
    /// structure. Useful as a starting point even outside aozora —
    /// the `unknown` fallback always catches anything unmatched.
    #[must_use]
    pub fn aozora_defaults() -> Self {
        Self {
            categories: AOZORA_DEFAULT_CATEGORIES
                .iter()
                .map(|(name, patterns)| CategorySpec {
                    name: (*name).to_owned(),
                    patterns: patterns.iter().map(|p| (*p).to_owned()).collect(),
                })
                .collect(),
        }
    }

    /// Parse from TOML. Schema:
    ///
    /// ```toml
    /// [[categories]]
    /// name = "phase1_scan"
    /// patterns = ["aho_corasick::packed::teddy", "aozora_scan"]
    ///
    /// [[categories]]
    /// name = "..."
    /// patterns = [...]
    /// ```
    pub fn from_toml(text: &str) -> Result<Self, CategoryError> {
        // Use serde's untyped value to keep the surface tiny.
        let parsed: toml::Value =
            toml::from_str(text).map_err(|e| CategoryError::Toml(e.to_string()))?;
        let arr = parsed
            .get("categories")
            .and_then(|v| v.as_array())
            .ok_or_else(|| CategoryError::Toml("expected top-level [[categories]]".into()))?;
        let mut categories = Vec::with_capacity(arr.len());
        for entry in arr {
            let name = entry
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| CategoryError::Toml("category missing `name`".into()))?
                .to_owned();
            let patterns = entry
                .get("patterns")
                .and_then(|v| v.as_array())
                .ok_or_else(|| {
                    CategoryError::Toml(format!("category `{name}` missing `patterns`"))
                })?
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect();
            categories.push(CategorySpec { name, patterns });
        }
        Ok(Self { categories })
    }

    /// Convenience: read a TOML file from disk.
    pub fn from_toml_file(path: &Path) -> Result<Self, CategoryError> {
        let text = std::fs::read_to_string(path)?;
        Self::from_toml(&text)
    }

    /// Compile patterns into a runtime [`Categorizer`].
    pub fn compile(&self) -> Result<Categorizer, CategoryError> {
        let mut compiled = Vec::with_capacity(self.categories.len());
        for cat in &self.categories {
            let mut regexes = Vec::with_capacity(cat.patterns.len());
            for pat in &cat.patterns {
                let re = Regex::new(pat).map_err(|error| CategoryError::Regex {
                    name: cat.name.clone(),
                    pattern: pat.clone(),
                    error,
                })?;
                regexes.push(re);
            }
            compiled.push(CompiledCategory {
                name: cat.name.clone(),
                regexes,
            });
        }
        Ok(Categorizer {
            categories: compiled,
        })
    }
}

/// Compiled, runtime-cheap categoriser. Use [`Categorizer::classify`]
/// to bucket a function name; first-match wins.
#[derive(Debug)]
pub struct Categorizer {
    categories: Vec<CompiledCategory>,
}

#[derive(Debug)]
struct CompiledCategory {
    name: String,
    regexes: Vec<Regex>,
}

impl Categorizer {
    /// Returns the matching category name, or `"unknown"` if none.
    #[must_use]
    pub fn classify<'a>(&'a self, function_name: &str) -> &'a str {
        for cat in &self.categories {
            if cat.regexes.iter().any(|r| r.is_match(function_name)) {
                return &cat.name;
            }
        }
        "unknown"
    }

    /// Names of all configured categories, in declaration order.
    /// Useful for ensuring report rows show a stable category order
    /// even if a particular category had zero hits.
    #[must_use]
    pub fn category_names(&self) -> Vec<&str> {
        self.categories.iter().map(|c| c.name.as_str()).collect()
    }
}
