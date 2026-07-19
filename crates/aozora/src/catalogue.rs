use crate::spec::{SLUGS, SlugEntry, canonicalise_slug};
use crate::syntax::{degraded, lint};

/// Canonical directive metadata used by editor integrations.
#[derive(Debug, Clone, Copy, Default)]
pub struct Catalogue;

/// Notation-hygiene normalization tier for a directive body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogueMatch {
    /// A verified spelling correction.
    Canonical,
    /// A lossy but recognized degradation.
    Degraded,
}

impl Catalogue {
    /// Create a view of the built-in directive catalogue.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// All completion entries.
    #[must_use]
    pub const fn directives() -> &'static [SlugEntry] {
        SLUGS
    }

    /// Resolve a canonical directive or a supported spelling variant.
    #[must_use]
    pub fn canonical(input: &str) -> Option<&'static str> {
        canonicalise_slug(input)
    }

    /// Find metadata for a canonical directive.
    #[must_use]
    pub fn find(canonical: &str) -> Option<&'static SlugEntry> {
        SLUGS.iter().find(|entry| entry.canonical == canonical)
    }

    /// Classify a non-canonical directive body by normalization tier.
    #[must_use]
    pub fn normalization(body: &str) -> Option<CatalogueMatch> {
        if lint::canonical_directive(body).is_some() {
            Some(CatalogueMatch::Canonical)
        } else if degraded::degraded_directive(body).is_some() {
            Some(CatalogueMatch::Degraded)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_resolves_only_exact_canonical_entries() {
        let entry = Catalogue::directives()
            .first()
            .expect("catalogue is non-empty");
        assert_eq!(
            Catalogue::find(entry.canonical).map(|found| found.canonical),
            Some(entry.canonical)
        );
        assert!(Catalogue::find("not-a-canonical-directive").is_none());
    }

    #[test]
    fn normalization_distinguishes_supported_and_unknown_spellings() {
        assert_eq!(
            Catalogue::normalization("斜体字"),
            Some(CatalogueMatch::Canonical)
        );
        assert!(Catalogue::normalization("not-a-directive").is_none());
    }
}
