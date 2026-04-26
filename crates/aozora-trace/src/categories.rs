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
/// First-match wins, in declaration order.
const AOZORA_DEFAULT_CATEGORIES: &[(&str, &[&str])] = &[
    (
        "phase1_scan",
        &[
            r"aho_corasick::packed::teddy",
            r"aho_corasick::packed::vector",
            r"aozora_scan::backends::teddy",
            r"aozora_scan::backends::structural_bitmap",
            r"aozora_scan::backends::dfa",
            r"aozora_scan::naive",
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
    ("memchr_scan", &[r"memchr::arch", r"memchr::memmem"]),
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
    (
        "allocation",
        &[
            r"alloc::",
            r"bumpalo::",
            r"__rust_alloc",
            r"__rust_dealloc",
            r"malloc",
            r"realloc",
            r"free",
            r"memmove",
            r"memcpy",
        ],
    ),
    ("rendering", &[r"aozora_render"]),
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
