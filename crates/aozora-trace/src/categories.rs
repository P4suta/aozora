use std::fs;
use std::io;
use std::path::Path;

use regex::Regex;

const AOZORA_DEFAULT_CATEGORIES: &[(&str, &[&str])] = &[
    (
        "scan",
        &[
            r"aho_corasick::packed",
            r"aozora::scan",
            r"core::core_arch::x86_64::avx::_mm256_extract",
            r"core::core_arch::x86::avx2::_mm256_movemask",
            r"core::core_arch::x86::avx2::_mm256_cmpeq",
            r"core::core_arch::x86::avx2::_mm256_shuffle",
            r"core::core_arch::x86::avx2::_mm256_or",
            r"core::core_arch::x86::avx2::_mm256_and",
            r"core::core_arch::x86::avx2::_mm256_alignr",
        ],
    ),
    ("tokenize", &[r"aozora::pipeline::lexer::tokenize"]),
    ("sanitize", &[r"aozora::pipeline::lexer::sanitize"]),
    ("pair", &[r"aozora::pipeline::lexer::pair"]),
    (
        "classify",
        &[
            r"aozora::pipeline::lexer::classify",
            r"aozora::pipeline::lexer::recognise",
        ],
    ),
    ("intern", &[r"aozora::syntax::ast::intern", r"Interner"]),
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
            r"aozora::encoding",
            r"shift_jis",
            r"Utf8Bmp",
            r"Utf8Destination",
        ],
    ),
    ("corpus_walk", &[r"aozora_corpus", r"walkdir"]),
    (
        "pipeline_orchestration",
        &[r"aozora::pipeline::state_machine", r"run_to_completion"],
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
    (
        "alloc_bumpalo_arena",
        &[r"bumpalo::", r"bumpalo_collections::", r"BumpVec"],
    ),
    (
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
    ("rendering", &[r"aozora::render"]),
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
    Io(#[from] io::Error),
}

/// User-supplied category definitions.
/// Compile to a [`Categorizer`] once and reuse across analyses.
#[derive(Debug, Clone)]
pub struct RollupConfig {
    /// Ordered category definitions.
    /// The first matching category wins.
    pub categories: Vec<CategorySpec>,
}

/// A named category with regular expressions for matching function names.
#[derive(Debug, Clone)]
pub struct CategorySpec {
    /// Category label shown in the rollup report.
    pub name: String,
    /// Regular expressions that assign matching function names to this category.
    pub patterns: Vec<String>,
}

impl RollupConfig {
    /// Built-in categories for the aozora workspace's pipeline stages.
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

    /// Parses ordered category definitions from TOML.
    ///
    /// ```toml
    /// [[categories]]
    /// name = "scan"
    /// patterns = ["aho_corasick::packed::teddy", "aozora::scan"]
    ///
    /// [[categories]]
    /// name = "..."
    /// patterns = [...]
    /// ```
    ///
    /// # Errors
    /// Returns an error for invalid TOML, missing category fields, or non-string patterns.
    pub fn from_toml(text: &str) -> Result<Self, CategoryError> {
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
                .map(|value| {
                    value.as_str().map(str::to_owned).ok_or_else(|| {
                        CategoryError::Toml(format!(
                            "category `{name}` has a non-string `patterns` entry"
                        ))
                    })
                })
                .collect::<Result<_, _>>()?;
            categories.push(CategorySpec { name, patterns });
        }
        Ok(Self { categories })
    }

    /// Read category definitions from a TOML file.
    ///
    /// # Errors
    /// Returns an error if reading or parsing the file fails.
    pub fn from_toml_file(path: &Path) -> Result<Self, CategoryError> {
        let text = fs::read_to_string(path)?;
        Self::from_toml(&text)
    }

    /// Compile patterns into a runtime [`Categorizer`].
    ///
    /// # Errors
    /// Returns an error for invalid regular expressions.
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

/// Compiled categorizer that selects the first matching category.
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
    #[must_use]
    pub fn category_names(&self) -> Vec<&str> {
        self.categories.iter().map(|c| c.name.as_str()).collect()
    }
}
