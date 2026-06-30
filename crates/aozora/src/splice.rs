//! Source-region ownership and minimal-diff source splicing.
//!
//! The **minimal-diff edit splice** (issue #202) — the last pillar of the
//! coremodel-purification epic (#189). An editor surface that "adds ruby to
//! this word" or "changes this heading level" wants the resulting source to
//! differ from the original by the smallest possible diff; it must *not*
//! reflow the whole document to canonical form ([`Tree::to_source`]), which
//! would rewrite the author's verbatim formatting everywhere.
//!
//! This layer answers, for every byte of the sanitized source, two questions:
//!
//! 1. **Who owns each source byte?** [`Tree::owned_regions`] projects the
//!    source-node table into a *total, non-overlapping, ordered* tiling: one
//!    [`OwnedRegion`] per classified node plus the interstitial plain runs
//!    between them. Concatenating every region's bytes reproduces
//!    [`Tree::to_source_verbatim`] exactly.
//!
//! 2. **How is this region edited coherently?** Each region carries a
//!    terminal [`SpliceSafety`]:
//!    - [`Direct`](SpliceSafety::Direct) — the region fully owns its rendered
//!      content (a self-contained node, or plain interstitial text), so
//!      replacing its bytes is a complete edit.
//!    - [`Coupled`](SpliceSafety::Coupled) — a coherent edit spans a *derived
//!      partner*: the upstream literal of a non-adjacent forward reference
//!      ([`ForwardOrigin::Referenced`]), a heading hint, a margin note, or the
//!      paired marker of a container. The partner is recovered on demand
//!      ([`Tree::coupling`]) and the edit is checked by re-parse.
//!    - [`Opaque`](SpliceSafety::Opaque) — a future node variant this build of
//!      the parser does not classify (forward-compat only; never produced by
//!      any construct this version understands).
//!
//! [`Tree::splice`] performs the edit and returns the minimal-diff source —
//! every byte outside the affected region(s) stays identical, unlike the
//! whole-document reflow of [`Tree::to_source`]. A coupled edit *derives* the
//! partner change, re-parses the candidate, and **verifies** the construct
//! re-formed; it returns [`SpliceError`] rather than emit a byte-valid but
//! semantically desynced edit. The parser is the single source of truth for
//! "what couples to what" — this layer proposes, the parser confirms.
//!
//! # Why this shape
//!
//! Nothing is stored on the AST to support the splice. The coupling of a
//! forward reference is exactly the irreducible [`ForwardOrigin`] provenance
//! the epic already materialized; a container's pairing is the structural
//! nesting already present in [`Tree::source_nodes`]. The splice model is the
//! dual of the parser's classification, derived entirely on demand from data
//! that already exists. See ADR-0018 (foundation) and ADR-0019 (coupled /
//! container splice).
//!
//! Incremental *re-parse* (reusing the unaffected tree across an edit) is a
//! separate performance concern, not part of this model: the parser is
//! single-digit milliseconds on real corpus documents, so the verify re-parse
//! is cheap and there is no current pressure to reuse subtrees.

use core::error::Error;
use core::fmt;

use aozora_render::serialize::container_close_source;
use aozora_spec::{SourceOffset, Span};
use aozora_syntax::{ForwardOrigin, RegionClose, RegionFormat};

use crate::{Document, NodeOwned, NodeRefOwned, Tree};

/// What a single source region represents.
///
/// Informational — tooling renders it to explain a region — while the
/// actionable bit is the region's [`SpliceSafety`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RegionRole {
    /// Plain text between classified constructs. Not a node; carried so the
    /// tiling is complete. Directly editable as bytes.
    Interstitial,
    /// Ruby (furigana). Self-contained: the base run is included in the region
    /// (the explicit `｜` or the implicit trailing-kanji pull-back).
    Ruby,
    /// Forward emphasis whose target literal the classifier pulled into the
    /// node from the immediately-preceding source
    /// ([`ForwardOrigin::Reclaimed`]). The literal lives inside the region, so
    /// it is self-contained.
    ForwardReclaimed,
    /// Forward emphasis whose target literal stays in a separate upstream run
    /// ([`ForwardOrigin::Referenced`]). Ownership is split across two regions
    /// — the bracket here and the upstream literal — so a coherent target
    /// edit is a [`Coupled`](SpliceSafety::Coupled) splice.
    ForwardReferenced,
    /// Out-of-character-range glyph (外字).
    Gaiji,
    /// Single-line layout directive (字下げ / 地付き / 中央 / 罫囲み).
    Line,
    /// Warichu (割り注, split annotation) — the inline form owns its body.
    Warichu,
    /// Page break (`［＃改ページ］`).
    PageBreak,
    /// Section break (`［＃改丁／改段／改見開き］`).
    SectionBreak,
    /// Body-end marker (`［＃本文終わり］`) — a self-contained structural leaf.
    BodyEnd,
    /// Forced line break (`［＃改行］`) — a self-contained inline marker.
    ForcedBreak,
    /// Heading promoted from a bare line above its directive — the referent
    /// line is reclaimed into the region, so it is self-contained.
    Heading,
    /// Forward heading hint whose referent is *not* the bare line above it, so
    /// the referent lives elsewhere. A coherent target edit is coupled.
    HeadingHint,
    /// Illustration (`［＃挿絵］`).
    Illustration,
    /// Chinese-reading-order mark (返り点).
    Kaeriten,
    /// Generic annotation (`［＃ママ］`, an unresolved `［＃…］`, …). The
    /// directive bracket is self-contained.
    Directive,
    /// `≪…≫` double-angle quotation.
    AngleQuote,
    /// Left-side note (注記 / 傍記) attached to a preceding base run. A
    /// coherent target edit is coupled with that base run.
    MarginNote,
    /// A leaf container node (rare; containers usually surface as
    /// [`RegionRole::ContainerOpen`] / [`RegionRole::ContainerClose`]).
    Container,
    /// A paired-container open marker (`［＃ここから…］`). Coupled with its
    /// matching close.
    ContainerOpen,
    /// A paired-container close marker (`［＃ここで…終わり］`). Coupled with its
    /// matching open.
    ContainerClose,
    /// A future [`NodeOwned`] variant not yet classified by this projection.
    Other,
}

/// The kind of two-region coupling a region participates in — the payload of a
/// coupled splice classification.
///
/// Distinct from [`RegionRole`], which names a single tile: this names the
/// *relationship* between the region and its derived partner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CoupledKind {
    /// A non-adjacent forward reference ([`ForwardOrigin::Referenced`]): the
    /// directive bracket plus its upstream target literal.
    ForwardReference,
    /// A forward heading hint plus its upstream referent run.
    HeadingHint,
    /// A margin note (注記 / 傍記) plus the upstream base run it annotates.
    MarginNote,
    /// A paired container: the `［＃ここから…］` open plus the `［＃ここで…終わり］`
    /// close.
    Container,
}

/// How a region is edited as a coherent minimal-diff splice. Terminal: every
/// classified region is exactly one of these — there is no "deferred to a
/// later phase" state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SpliceSafety {
    /// The region fully owns its rendered content (a self-contained node, or
    /// plain interstitial text). Replacing its bytes is a complete edit;
    /// neighbouring regions stay byte-identical.
    Direct,
    /// A coherent edit spans a *derived partner* region (an upstream literal,
    /// or a paired container marker). [`Tree::splice`] derives the partner
    /// change and verifies it by re-parse; [`Tree::coupling`] exposes the
    /// partner span.
    Coupled(CoupledKind),
    /// A future node variant this build of the parser does not classify.
    /// [`Tree::splice`] declines it rather than guess. In practice
    /// unreachable: every construct this version understands is `Direct` or
    /// `Coupled`.
    Opaque,
}

/// A contiguous run of source bytes and what it owns.
///
/// Yielded by [`Tree::owned_regions`] / [`Tree::owned_region_at`]. The
/// [`span`](Self::span) indexes the **sanitized** source — the same coordinate
/// space as [`Tree::to_source_verbatim`] and every `source_span` on
/// [`Tree::source_nodes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnedRegion {
    /// Half-open byte range in sanitized-source coordinates.
    pub span: Span,
    /// What the region represents.
    pub role: RegionRole,
    /// How the region is edited coherently.
    pub safety: SpliceSafety,
}

/// The two source regions a coupled edit touches.
///
/// Recovered on demand by [`Tree::coupling`] from the source-node table — no
/// link is stored on the AST. Both spans are in sanitized-source coordinates;
/// `primary` and `partner` are *not* ordered relative to each other (a forward
/// reference's literal precedes its bracket; a container's open precedes its
/// close).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Coupling {
    /// The relationship between the two regions.
    pub kind: CoupledKind,
    /// The queried region (the directive bracket, or the queried container
    /// marker).
    pub primary: Span,
    /// The derived partner: the upstream target literal (forward / heading
    /// hint / margin note), or the matching container marker.
    pub partner: Span,
}

/// Error returned by [`Tree::splice`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SpliceError {
    /// A [`Coupled`](SpliceSafety::Coupled) edit could not be carried out
    /// coherently: the candidate source did not re-parse to the intended
    /// construct, so applying it would silently desync the reference. The
    /// honest terminal outcome for the corpus-attested hard cases — an
    /// ambiguous forward referent, a ruby-base target literal, or a
    /// 、-joined multi-target. The source is left unchanged.
    Unverifiable {
        /// The edited region's role, for diagnostics.
        role: RegionRole,
        /// The coupling that could not be completed.
        kind: CoupledKind,
    },
    /// The region's node kind is [`Opaque`](SpliceSafety::Opaque) — a future
    /// variant this build does not understand — so it is declined rather than
    /// edited by a guess.
    Opaque {
        /// The region's role, for diagnostics.
        role: RegionRole,
    },
}

impl fmt::Display for SpliceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unverifiable { role, kind } => write!(
                f,
                "coupled {kind:?} edit of region {role:?} could not be verified \
                 (the candidate did not re-parse to the intended construct)"
            ),
            Self::Opaque { role } => {
                write!(
                    f,
                    "region {role:?} has an unclassified node kind and cannot be spliced"
                )
            }
        }
    }
}

impl Error for SpliceError {}

/// Classify a node region's role and splice safety. Pure: a function of the
/// [`NodeRefOwned`] variant and (for a forward leaf) its [`ForwardOrigin`] alone.
///
/// `pub(crate)` so the incremental splice
/// ([`crate::incremental`]) shares this single source of truth for the
/// text-coupling check (a forward reference / heading hint / margin note
/// resolves by whole-document text search, so a region re-lex cannot localise
/// it).
pub(crate) fn classify_node_ref(node: NodeRefOwned) -> (RegionRole, SpliceSafety) {
    use SpliceSafety::{Coupled, Direct, Opaque};

    match node {
        NodeRefOwned::BlockOpen(_) => (RegionRole::ContainerOpen, Coupled(CoupledKind::Container)),
        NodeRefOwned::BlockClose(_) => {
            (RegionRole::ContainerClose, Coupled(CoupledKind::Container))
        }
        NodeRefOwned::Inline(n) | NodeRefOwned::BlockLeaf(n) => match n {
            NodeOwned::Format(f) => match f.origin {
                ForwardOrigin::Reclaimed => (RegionRole::ForwardReclaimed, Direct),
                ForwardOrigin::Referenced => (
                    RegionRole::ForwardReferenced,
                    Coupled(CoupledKind::ForwardReference),
                ),
            },
            NodeOwned::HeadingHint(_) => {
                (RegionRole::HeadingHint, Coupled(CoupledKind::HeadingHint))
            }
            NodeOwned::MarginNote(_) => (RegionRole::MarginNote, Coupled(CoupledKind::MarginNote)),
            NodeOwned::Container(_) => (RegionRole::Container, Coupled(CoupledKind::Container)),
            NodeOwned::Ruby(_) => (RegionRole::Ruby, Direct),
            NodeOwned::Heading(_) => (RegionRole::Heading, Direct),
            NodeOwned::Gaiji(_) => (RegionRole::Gaiji, Direct),
            NodeOwned::Warichu(_) => (RegionRole::Warichu, Direct),
            NodeOwned::AngleQuote(_) => (RegionRole::AngleQuote, Direct),
            NodeOwned::Kaeriten(_) => (RegionRole::Kaeriten, Direct),
            NodeOwned::Illustration(_) => (RegionRole::Illustration, Direct),
            NodeOwned::Line(_) => (RegionRole::Line, Direct),
            NodeOwned::PageBreak => (RegionRole::PageBreak, Direct),
            NodeOwned::SectionBreak(_) => (RegionRole::SectionBreak, Direct),
            // Self-contained structural-marker leaves (#78) — they fully own
            // their rendered bytes, so editing the bracket is a Direct splice.
            NodeOwned::BodyEnd => (RegionRole::BodyEnd, Direct),
            NodeOwned::ForcedBreak => (RegionRole::ForcedBreak, Direct),
            NodeOwned::Directive(_) => (RegionRole::Directive, Direct),
            // `NodeOwned` is `#[non_exhaustive]`; an unknown future variant is
            // declined rather than assumed editable.
            _ => (RegionRole::Other, Opaque),
        },
        // `NodeRefOwned` is `#[non_exhaustive]`; decline an unknown future variant.
        _ => (RegionRole::Other, Opaque),
    }
}

/// Interstitial plain text: not a node, but the most directly editable region
/// of all — replacing the bytes is a complete edit.
const INTERSTITIAL: (RegionRole, SpliceSafety) = (RegionRole::Interstitial, SpliceSafety::Direct);

/// Whether `node` belongs to the same construct family as a coupled `kind` —
/// the verify predicate for a re-parsed single-region edit.
///
/// A forward reference may re-form as either origin (`Reclaimed` or
/// `Referenced` are both valid forwards); a heading hint may re-form as a hint
/// *or* a promoted heading; a container marker as an open or close.
fn reparsed_in_family(node: NodeRefOwned, kind: CoupledKind) -> bool {
    let leaf = match node {
        NodeRefOwned::Inline(n) | NodeRefOwned::BlockLeaf(n) => Some(n),
        _ => None,
    };
    match kind {
        CoupledKind::ForwardReference => matches!(leaf, Some(NodeOwned::Format(_))),
        CoupledKind::HeadingHint => {
            matches!(
                leaf,
                Some(NodeOwned::HeadingHint(_) | NodeOwned::Heading(_))
            )
        }
        CoupledKind::MarginNote => matches!(leaf, Some(NodeOwned::MarginNote(_))),
        CoupledKind::Container => {
            matches!(
                node,
                NodeRefOwned::BlockOpen(_) | NodeRefOwned::BlockClose(_)
            ) || matches!(leaf, Some(NodeOwned::Container(_)))
        }
    }
}

impl Tree<'_> {
    /// Project the source-node table into a complete tiling of the sanitized
    /// source: one [`OwnedRegion`] per classified node plus the interstitial
    /// plain runs between (and around) them.
    ///
    /// The regions are contiguous, non-overlapping, and ordered by start
    /// offset; the first starts at `0`, the last ends at the sanitized length,
    /// and concatenating each region's bytes reproduces
    /// [`Tree::to_source_verbatim`] exactly. A truly empty source yields no
    /// regions.
    #[must_use]
    pub fn owned_regions(&self) -> Vec<OwnedRegion> {
        let nodes = self.source_nodes();
        // The sanitized length fits u32 — every offset in the tree is a u32
        // `Span` — so the saturating fallback is never taken.
        let src_len = u32::try_from(self.sanitized().len()).unwrap_or(u32::MAX);
        let mut out: Vec<OwnedRegion> = Vec::with_capacity(nodes.len() * 2 + 1);
        let mut cursor: u32 = 0;
        for sn in nodes {
            let start = sn.source_span.start;
            if start > cursor {
                out.push(OwnedRegion {
                    span: Span::new(cursor, start),
                    role: INTERSTITIAL.0,
                    safety: INTERSTITIAL.1,
                });
            }
            let (role, safety) = classify_node_ref(sn.node);
            out.push(OwnedRegion {
                span: sn.source_span,
                role,
                safety,
            });
            cursor = sn.source_span.end;
        }
        if cursor < src_len {
            out.push(OwnedRegion {
                span: Span::new(cursor, src_len),
                role: INTERSTITIAL.0,
                safety: INTERSTITIAL.1,
            });
        }
        out
    }

    /// The [`OwnedRegion`] covering `off`, a sanitized-source byte offset.
    ///
    /// Returns the classified node region when `off` lands on a construct
    /// ([`O(log n)`](Tree::node_at_source)), or the surrounding interstitial
    /// run otherwise. Returns `None` only when `off` is past the end of the
    /// sanitized source.
    #[must_use]
    pub fn owned_region_at(&self, off: SourceOffset) -> Option<OwnedRegion> {
        let src_len = u32::try_from(self.sanitized().len()).unwrap_or(u32::MAX);
        if off.get() >= src_len {
            return None;
        }
        if let Some(sn) = self.node_at_source(off) {
            let (role, safety) = classify_node_ref(sn.node);
            return Some(OwnedRegion {
                span: sn.source_span,
                role,
                safety,
            });
        }
        // `off` falls in an interstitial gap, bounded by the end of the last
        // node starting at/before `off` and the start of the next node.
        let nodes = self.source_nodes();
        let raw = off.get();
        let next_idx = nodes.partition_point(|n| n.source_span.start <= raw);
        let gap_start = if next_idx == 0 {
            0
        } else {
            nodes[next_idx - 1].source_span.end
        };
        let gap_end = nodes.get(next_idx).map_or(src_len, |n| n.source_span.start);
        Some(OwnedRegion {
            span: Span::new(gap_start, gap_end),
            role: INTERSTITIAL.0,
            safety: INTERSTITIAL.1,
        })
    }

    /// Recover the two regions a [`Coupled`](SpliceSafety::Coupled) edit
    /// touches, or `None` for a `Direct` / `Opaque` region (no partner) or
    /// when the partner cannot be located.
    ///
    /// For a container marker the partner is its matching open/close, paired
    /// directly in source coordinates by a depth-stack walk over
    /// [`Tree::source_nodes`] — no normalized-coordinate detour. For a forward
    /// reference / heading hint / margin note the partner is the upstream
    /// target literal.
    ///
    /// This is read-only introspection (e.g. for an editor to highlight both
    /// sites). [`Tree::splice`] performs the actual coherent edit.
    #[must_use]
    pub fn coupling(&self, region: OwnedRegion) -> Option<Coupling> {
        match region.safety {
            SpliceSafety::Coupled(CoupledKind::Container) => self.container_coupling(region.span),
            SpliceSafety::Coupled(kind) => {
                // Forward reference / heading hint / margin note: the partner is
                // the unique upstream plain occurrence of the node's target.
                // `None` for the irreducible cases (ambiguous referent, a
                // ruby-base literal, a multi-segment target).
                let target = self.coupled_target_text(region.span)?;
                let partner = self.unique_upstream_plain(region.span.start, &target)?;
                Some(Coupling {
                    kind,
                    primary: region.span,
                    partner,
                })
            }
            _ => None,
        }
    }

    /// Produce minimal-diff source by editing `region` to `replacement`.
    ///
    /// `replacement` is the new source for the region's own bytes (the new
    /// directive bracket, the new container open marker, the new self-contained
    /// node text, or `""` to delete). The result preserves every byte outside
    /// the affected region(s) exactly, unlike the whole-document
    /// canonicalisation of [`Tree::to_source`].
    ///
    /// - A [`Direct`](SpliceSafety::Direct) region is a single-region byte
    ///   replacement.
    /// - A [`Coupled`](SpliceSafety::Coupled) region derives its partner change
    ///   (the matching container close for a new open; the upstream literal for
    ///   a forward-reference target change) and **verifies** the candidate by
    ///   re-parse before returning it.
    ///
    /// The caller typically re-parses the result (`Document::new(spliced)`) to
    /// obtain an updated tree, or uses [`Document::edit_region`] which does so.
    ///
    /// # Errors
    ///
    /// Returns [`SpliceError::Unverifiable`] when a coupled edit cannot be
    /// made coherent (the candidate did not re-parse to the intended
    /// construct), and [`SpliceError::Opaque`] for an unclassified future node
    /// kind.
    ///
    /// # Panics
    ///
    /// Panics if `region` did not come from this tree (its span is out of
    /// bounds for the sanitized source, or not on a UTF-8 codepoint boundary).
    /// Regions from this tree's [`Tree::owned_regions`] /
    /// [`Tree::owned_region_at`] always satisfy the precondition.
    pub fn splice(&self, region: OwnedRegion, replacement: &str) -> Result<String, SpliceError> {
        match region.safety {
            SpliceSafety::Direct => Ok(splice_one(self.sanitized(), region.span, replacement)),
            SpliceSafety::Coupled(CoupledKind::Container) => {
                self.splice_container(region, replacement)
            }
            SpliceSafety::Coupled(kind) => self.splice_split(region, kind, replacement),
            SpliceSafety::Opaque => Err(SpliceError::Opaque { role: region.role }),
        }
    }

    /// Pair a container marker at `span` with its partner, directly in source
    /// coordinates, via a depth-stack walk over the source-node table (the
    /// same LIFO the normalizer itself uses). Returns the coupling, or `None`
    /// for an unmatched stray marker.
    fn container_coupling(&self, span: Span) -> Option<Coupling> {
        let (open, close) = self.container_pair_for(span)?;
        let primary = if span == open { open } else { close };
        let partner = if span == open { close } else { open };
        Some(Coupling {
            kind: CoupledKind::Container,
            primary,
            partner,
        })
    }

    /// The `(open_span, close_span)` of the balanced container pair whose open
    /// or close is `span`. `None` if `span` is not a paired container marker.
    fn container_pair_for(&self, span: Span) -> Option<(Span, Span)> {
        let mut stack: Vec<Span> = Vec::new();
        for sn in self.source_nodes() {
            match sn.node {
                NodeRefOwned::BlockOpen(_) => stack.push(sn.source_span),
                NodeRefOwned::BlockClose(_) => {
                    if let Some(open_span) = stack.pop() {
                        let close_span = sn.source_span;
                        if span == open_span || span == close_span {
                            return Some((open_span, close_span));
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Container edit: rewrite the open marker (and, when its family changes,
    /// the matching close) or delete the pair.
    ///
    /// Verification is **scoped** — the replacement *marker* is parsed in
    /// isolation (a `［＃…］` bracket is self-delimiting, so it parses
    /// identically in or out of context), never the whole document. The
    /// matching close is then a pure function of that open
    /// ([`RegionClose::of`]), and the 1:1 marker replacement preserves the
    /// document's nesting, so the edit is correct by construction — `O(marker)`,
    /// not `O(document)`.
    fn splice_container(
        &self,
        region: OwnedRegion,
        replacement: &str,
    ) -> Result<String, SpliceError> {
        let unverifiable = SpliceError::Unverifiable {
            role: region.role,
            kind: CoupledKind::Container,
        };
        let src = self.sanitized();
        let pair = self.container_pair_for(region.span);

        // Delete: drop the marker(s), keeping the body. A structural unwrap is
        // always byte-valid; there is no reference to desync.
        if replacement.is_empty() {
            return Ok(match pair {
                Some((open_span, close_span)) => splice_two(src, (open_span, ""), (close_span, "")),
                None => splice_one(src, region.span, ""),
            });
        }

        // Only an *open* that is part of a pair drives the coupled
        // close-derivation. A close marker, an unmatched stray marker, or a
        // leaf container is a single-region edit — accept it only if the
        // replacement is itself a container marker (so we never "fix" a
        // pre-existing mismatch or silently change the construct).
        let Some((open_span, close_span)) = pair.filter(|(open, _)| *open == region.span) else {
            return if marker_in_family(replacement, CoupledKind::Container) {
                Ok(splice_one(src, region.span, replacement))
            } else {
                Err(unverifiable)
            };
        };

        // Recover the new open's format from the replacement marker alone
        // (O(marker)); the existing open's format is a lookup on the already-
        // parsed tree (no re-parse).
        let new_format = lone_open_format(replacement).ok_or(unverifiable)?;
        let old_format = block_open_format_at(self, open_span.start);

        // Same close family (identity, amount change): keep the existing close
        // verbatim — replacing the open alone is the true minimal diff, and it
        // preserves a pre-existing mismatch rather than "fixing" it.
        if old_format.is_some_and(|old| RegionClose::of(old) == RegionClose::of(new_format)) {
            return Ok(splice_one(src, open_span, replacement));
        }

        // The close family changed: derive the canonical matching close and
        // rewrite both markers. Correct by construction — `new_format` parsed
        // cleanly, the close is its canonical partner, and the markers replace
        // 1:1 so the document's nesting is unchanged.
        let new_close = container_close_source(new_format);
        Ok(splice_two(
            src,
            (open_span, replacement),
            (close_span, &new_close),
        ))
    }

    /// Split-ownership edit (forward reference / heading hint / margin note).
    ///
    /// Removing the directive (`""`) is coherent — the upstream literal simply
    /// becomes plain. An identity or attribute-only change is a single-region
    /// edit, **verified in a scoped context**: the new bracket parsed against
    /// the node's own target (`<target><replacement>`, the minimal window that
    /// re-forms the reference), never the whole document.
    ///
    /// A target-text change is a **coupled two-region edit**: the new target is
    /// recovered from the replacement and the *unique* upstream plain
    /// occurrence of the old target; both are rewritten and the reference is
    /// re-verified in a window bounded by their distance. The irreducible cases
    /// — an ambiguous referent (the target occurs more than once), a ruby-base
    /// literal (the occurrence is not a lone plain run), or a multi-segment
    /// target — are declined as [`SpliceError::Unverifiable`] rather than
    /// silently desynced.
    fn splice_split(
        &self,
        region: OwnedRegion,
        kind: CoupledKind,
        replacement: &str,
    ) -> Result<String, SpliceError> {
        let src = self.sanitized();
        if replacement.is_empty() {
            return Ok(splice_one(src, region.span, replacement));
        }
        let unverifiable = SpliceError::Unverifiable {
            role: region.role,
            kind,
        };
        // The node's own target text. `None` for a multi-segment target (the
        // 、-joined case), which is declined.
        let old_target = self.coupled_target_text(region.span).ok_or(unverifiable)?;

        // Single-region attempt: with the node's own target byte-adjacent to the
        // new bracket (`<target><replacement>`, the minimal context that
        // re-forms the reference), an identity or attribute-only change keeps
        // the construct in its family.
        let bracket_at = u32::try_from(old_target.len()).map_err(|_| unverifiable)?;
        let ctx = format!("{old_target}{replacement}");
        if reparsed_family_at(&ctx, bracket_at, kind) {
            return Ok(splice_one(src, region.span, replacement));
        }

        // Coupled two-region attempt: the bracket's target changed, so the
        // upstream literal must change with it. Recover the new target from the
        // replacement and the *unique* upstream plain occurrence of the old
        // target, rewrite both, and verify the reference re-forms — declining
        // the irreducible cases (ambiguous referent, ruby-base literal) where
        // the occurrence is not a lone plain run.
        let new_target = first_quoted(replacement).ok_or(unverifiable)?;
        let occ = self
            .unique_upstream_plain(region.span.start, &old_target)
            .ok_or(unverifiable)?;
        let candidate = splice_two(src, (occ, new_target), (region.span, replacement));

        // Scoped verify: a window from the rewritten literal to the new
        // directive's end (bounded by the reference distance, not the document
        // size) must re-parse to a `kind` node referencing `new_target`. The
        // occurrence precedes the directive, so every offset is non-negative.
        let win_start = occ.start as usize;
        let interstice = region.span.start as usize - occ.end as usize;
        let new_directive_start = win_start + new_target.len() + interstice;
        let win_end = new_directive_start + replacement.len();
        let reformed = candidate
            .get(win_start..win_end)
            .is_some_and(|w| window_reforms_coupled(w, kind, new_target));
        if reformed {
            Ok(candidate)
        } else {
            Err(unverifiable)
        }
    }

    /// The target text of a split-ownership node (a forward reference's target,
    /// a heading hint's target, or a margin note's base) as a plain string.
    /// `None` when the node is not a split-ownership leaf, or its target is not
    /// a single plain run (a 、-joined multi-target).
    fn coupled_target_text(&self, span: Span) -> Option<String> {
        let store = &self.lex_output().store;
        let (NodeRefOwned::Inline(leaf) | NodeRefOwned::BlockLeaf(leaf)) =
            self.node_at_source(SourceOffset::new(span.start))?.node
        else {
            return None;
        };
        match leaf {
            NodeOwned::Format(f) => store.content_range_as_plain(f.target).map(str::to_owned),
            NodeOwned::HeadingHint(h) => Some(store.resolve_str(h.target).to_owned()),
            NodeOwned::MarginNote(m) => store.content_range_as_plain(m.base).map(str::to_owned),
            _ => None,
        }
    }

    /// The span of the **unique** occurrence of `target` in the sanitized
    /// source before `before`, but only when it lies wholly within a single
    /// plain interstitial run. `None` if the target is absent, appears more
    /// than once (an ambiguous referent), or its occurrence falls inside a
    /// classified construct (e.g. a ruby base) — the irreducible cases a
    /// coupled edit must decline rather than guess.
    fn unique_upstream_plain(&self, before: u32, target: &str) -> Option<Span> {
        let prefix = self.sanitized().get(..before as usize)?;
        let mut hit: Option<usize> = None;
        let mut from = 0usize;
        while let Some(rel) = prefix.get(from..)?.find(target) {
            let at = from + rel;
            if hit.is_some() {
                return None; // more than one occurrence — ambiguous
            }
            hit = Some(at);
            from = at + target.len();
        }
        let start = u32::try_from(hit?).ok()?;
        let span = Span::new(start, start + u32::try_from(target.len()).ok()?);
        let region = self.owned_region_at(SourceOffset::new(span.start))?;
        (region.role == RegionRole::Interstitial && span.end <= region.span.end).then_some(span)
    }
}

/// The `RegionFormat` of a `BlockOpen` node starting at sanitized offset
/// `start`, if any.
fn block_open_format_at(tree: &Tree<'_>, start: u32) -> Option<RegionFormat> {
    tree.node_at_source(SourceOffset::new(start))
        .and_then(|sn| match sn.node {
            NodeRefOwned::BlockOpen(f) if sn.source_span.start == start => Some(f),
            _ => None,
        })
}

/// The `RegionFormat` of `marker` parsed *in isolation* as a single container
/// open. A `［＃…］` bracket is self-delimiting, so its standalone parse equals
/// its in-context parse; this recovers the new open's format in `O(marker)`
/// without re-parsing the document. `None` if `marker` is not a clean lone open
/// marker.
fn lone_open_format(marker: &str) -> Option<RegionFormat> {
    let doc = Document::new(marker);
    match doc.parse().source_nodes().first() {
        Some(sn) if sn.source_span.start == 0 => match sn.node {
            NodeRefOwned::BlockOpen(f) => Some(f),
            _ => None,
        },
        _ => None,
    }
}

/// Whether `marker`, parsed in isolation, leads with a node in `kind`'s
/// construct family (a self-delimiting `［＃…］` marker). Used to accept a
/// single-region container-marker edit without re-parsing the document.
fn marker_in_family(marker: &str, kind: CoupledKind) -> bool {
    let doc = Document::new(marker);
    doc.parse()
        .source_nodes()
        .first()
        .is_some_and(|sn| sn.source_span.start == 0 && reparsed_in_family(sn.node, kind))
}

/// Parse `ctx` and report whether the node covering sanitized offset `off` is
/// in `kind`'s construct family. The single-region split-edit verify, in a
/// minimal `<target><replacement>` context.
fn reparsed_family_at(ctx: &str, off: u32, kind: CoupledKind) -> bool {
    let doc = Document::new(ctx);
    doc.parse()
        .node_at_source(SourceOffset::new(off))
        .is_some_and(|sn| reparsed_in_family(sn.node, kind))
}

/// The text inside the first `「…」` of a directive (the quoted target / base of
/// a forward reference, heading hint, or margin note). `None` if absent.
fn first_quoted(directive: &str) -> Option<&str> {
    let after_open = directive.split_once('「')?.1;
    Some(after_open.split_once('」')?.0)
}

/// Parse `window` and report whether it re-forms a `kind` construct whose
/// target / base text equals `new_target`. The coupled split-edit verify, in a
/// window bounded by the reference distance rather than the whole document.
fn window_reforms_coupled(window: &str, kind: CoupledKind, new_target: &str) -> bool {
    let doc = Document::new(window);
    let tree = doc.parse();
    let store = &tree.lex_output().store;
    tree.source_nodes().iter().any(|sn| {
        let (NodeRefOwned::Inline(leaf) | NodeRefOwned::BlockLeaf(leaf)) = sn.node else {
            return false;
        };
        let text = match (kind, leaf) {
            (CoupledKind::ForwardReference, NodeOwned::Format(f)) => {
                store.content_range_as_plain(f.target)
            }
            (CoupledKind::HeadingHint, NodeOwned::HeadingHint(h)) => {
                Some(store.resolve_str(h.target))
            }
            // A promoted heading is an equally valid re-formation of the hint.
            (CoupledKind::HeadingHint, NodeOwned::Heading(h)) => {
                store.content_range_as_plain(h.text)
            }
            (CoupledKind::MarginNote, NodeOwned::MarginNote(m)) => {
                store.content_range_as_plain(m.base)
            }
            _ => None,
        };
        text == Some(new_target)
    })
}

/// Replace `span`'s bytes in `src` with `replacement`. Panics (via slicing) if
/// `span` is out of bounds or off a codepoint boundary.
fn splice_one(src: &str, span: Span, replacement: &str) -> String {
    let start = span.start as usize;
    let end = span.end as usize;
    let prefix = &src[..start];
    let suffix = &src[end..];
    let mut out = String::with_capacity(
        prefix
            .len()
            .saturating_add(replacement.len())
            .saturating_add(suffix.len()),
    );
    out.push_str(prefix);
    out.push_str(replacement);
    out.push_str(suffix);
    out
}

/// Replace two non-overlapping regions (`first` before `second`) in one pass.
/// Each is a `(span, replacement)` pair.
fn splice_two(src: &str, first: (Span, &str), second: (Span, &str)) -> String {
    let (a, repl_a) = first;
    let (b, repl_b) = second;
    debug_assert!(
        a.end <= b.start,
        "splice_two: regions must be ordered and disjoint"
    );
    let (a_start, a_end) = (a.start as usize, a.end as usize);
    let (b_start, b_end) = (b.start as usize, b.end as usize);
    let mut out = String::with_capacity(src.len() + repl_a.len() + repl_b.len());
    out.push_str(&src[..a_start]);
    out.push_str(repl_a);
    out.push_str(&src[a_end..b_start]);
    out.push_str(repl_b);
    out.push_str(&src[b_end..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Document;

    /// Concatenating every owned region's bytes reproduces the verbatim
    /// (sanitized) source, and the regions form a gap-free, ordered,
    /// non-overlapping cover.
    fn assert_tiling(src: &str) {
        let doc = Document::new(src);
        let tree = doc.parse();
        let verbatim = tree.to_source_verbatim();
        let regions = tree.owned_regions();

        if verbatim.is_empty() {
            assert!(regions.is_empty(), "empty source must yield no regions");
            return;
        }

        assert_eq!(regions[0].span.start, 0, "tiling must start at 0");
        assert_eq!(
            regions.last().unwrap().span.end as usize,
            verbatim.len(),
            "tiling must end at the source length",
        );
        for pair in regions.windows(2) {
            assert_eq!(
                pair[0].span.end, pair[1].span.start,
                "regions must be contiguous with no gap/overlap",
            );
            assert!(
                pair[0].span.start < pair[0].span.end,
                "regions must be non-empty",
            );
        }
        let rebuilt: String = regions
            .iter()
            .map(|r| &verbatim[r.span.start as usize..r.span.end as usize])
            .collect();
        assert_eq!(
            rebuilt, verbatim,
            "region concatenation must equal verbatim"
        );

        // Identity splice of every region reproduces the verbatim source.
        for r in &regions {
            let same = &verbatim[r.span.start as usize..r.span.end as usize];
            assert_eq!(
                tree.splice(*r, same).unwrap(),
                verbatim,
                "identity splice of {:?} must be the verbatim source",
                r.role,
            );
        }
    }

    /// Find the first region with the given role.
    fn role_of(src: &str, role: RegionRole) -> OwnedRegion {
        let doc = Document::new(src);
        let tree = doc.parse();
        tree.owned_regions()
            .into_iter()
            .find(|r| r.role == role)
            .unwrap_or_else(|| panic!("no {role:?} region in {src:?}"))
    }

    #[test]
    fn empty_source_has_no_regions() {
        assert_tiling("");
    }

    #[test]
    fn plain_text_is_one_direct_interstitial() {
        assert_tiling("ただの本文です。");
        let doc = Document::new("ただの本文です。");
        let tree = doc.parse();
        let regions = tree.owned_regions();
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].role, RegionRole::Interstitial);
        assert_eq!(regions[0].safety, SpliceSafety::Direct);
    }

    #[test]
    fn ruby_is_direct_and_self_contained() {
        assert_tiling("｜青梅《おうめ》の実");
        let r = role_of("｜青梅《おうめ》の実", RegionRole::Ruby);
        assert_eq!(r.safety, SpliceSafety::Direct);
    }

    #[test]
    fn reclaimed_forward_is_direct() {
        assert_tiling("青空［＃「青空」に傍点］の下");
        let r = role_of("青空［＃「青空」に傍点］の下", RegionRole::ForwardReclaimed);
        assert_eq!(r.safety, SpliceSafety::Direct);
    }

    #[test]
    fn referenced_forward_is_coupled() {
        let src = "青空がひろがる、その［＃「青空」に傍点］";
        assert_tiling(src);
        let r = role_of(src, RegionRole::ForwardReferenced);
        assert_eq!(
            r.safety,
            SpliceSafety::Coupled(CoupledKind::ForwardReference)
        );
    }

    #[test]
    fn container_markers_are_coupled() {
        let src = "前\n［＃ここから2字下げ］\n本文\n［＃ここで字下げ終わり］\n後";
        assert_tiling(src);
        let open = role_of(src, RegionRole::ContainerOpen);
        assert_eq!(open.safety, SpliceSafety::Coupled(CoupledKind::Container));
        let close = role_of(src, RegionRole::ContainerClose);
        assert_eq!(close.safety, SpliceSafety::Coupled(CoupledKind::Container));
    }

    #[test]
    fn gaiji_is_direct() {
        assert_tiling("※［＃「さんずい＋垂」、第3水準1-86-69］");
        let r = role_of("※［＃「さんずい＋垂」、第3水準1-86-69］", RegionRole::Gaiji);
        assert_eq!(r.safety, SpliceSafety::Direct);
    }

    #[test]
    fn direct_splice_replaces_only_the_region() {
        // A real non-identity minimal-diff edit on a Direct (Reclaimed) node.
        let src = "青空［＃「青空」に傍点］の下を歩く";
        let doc = Document::new(src);
        let tree = doc.parse();
        let region = role_of(src, RegionRole::ForwardReclaimed);
        let spliced = tree
            .splice(region, "海［＃「海」に傍点］")
            .expect("Reclaimed forward is Direct");
        assert_eq!(spliced, "海［＃「海」に傍点］の下を歩く");
    }

    #[test]
    fn coupling_pairs_container_markers() {
        let src = "前\n［＃ここから2字下げ］\n本文\n［＃ここで字下げ終わり］\n後";
        let doc = Document::new(src);
        let tree = doc.parse();
        let open = role_of(src, RegionRole::ContainerOpen);
        let c = tree.coupling(open).expect("container open is coupled");
        assert_eq!(c.kind, CoupledKind::Container);
        assert_eq!(c.primary, open.span);
        // The partner is the close marker; it sits after the open.
        assert!(c.partner.start > c.primary.start);
        let close = role_of(src, RegionRole::ContainerClose);
        assert_eq!(c.partner, close.span);
    }

    #[test]
    fn container_kind_change_rewrites_both_markers() {
        let src = "前\n［＃ここから2字下げ］\n本文\n［＃ここで字下げ終わり］\n後";
        let doc = Document::new(src);
        let tree = doc.parse();
        let open = role_of(src, RegionRole::ContainerOpen);
        let spliced = tree
            .splice(open, "［＃ここから罫囲み］")
            .expect("kind change is verifiable");
        assert!(spliced.contains("［＃ここから罫囲み］"));
        assert!(spliced.contains("［＃罫囲み終わり］"));
        assert!(!spliced.contains("字下げ"));
        // Body is preserved verbatim.
        assert!(spliced.contains("本文"));
        // And it re-parses to a balanced 罫囲み container.
        let rt = Document::new(spliced.as_str());
        let rtree = rt.parse();
        assert!(
            rtree
                .owned_regions()
                .iter()
                .any(|r| r.role == RegionRole::ContainerOpen)
        );
    }

    #[test]
    fn container_amount_change_touches_only_the_open() {
        let src = "前\n［＃ここから2字下げ］\n本文\n［＃ここで字下げ終わり］\n後";
        let doc = Document::new(src);
        let tree = doc.parse();
        let open = role_of(src, RegionRole::ContainerOpen);
        let spliced = tree
            .splice(open, "［＃ここから4字下げ］")
            .expect("amount change is verifiable");
        assert!(spliced.contains("［＃ここから4字下げ］"));
        // The close keyword is unchanged (字下げ終わり carries no amount).
        assert!(spliced.contains("［＃ここで字下げ終わり］"));
    }

    #[test]
    fn mismatched_container_pair_identity_is_verbatim() {
        // A pre-existing family mismatch (open 字下げ, close 地付き) must survive
        // an identity splice of either marker unchanged — the splice must not
        // "fix" the mismatch by rewriting the close to the open's family.
        assert_tiling("前\n［＃ここから2字下げ］\n本文\n［＃ここで地付き終わり］\n後");
    }

    #[test]
    fn container_delete_drops_both_markers() {
        let src = "前\n［＃ここから2字下げ］\n本文\n［＃ここで字下げ終わり］\n後";
        let doc = Document::new(src);
        let tree = doc.parse();
        let open = role_of(src, RegionRole::ContainerOpen);
        let spliced = tree.splice(open, "").expect("delete is coherent");
        assert!(!spliced.contains("字下げ"));
        assert!(spliced.contains("本文"));
        assert!(spliced.contains("前"));
        assert!(spliced.contains("後"));
    }

    #[test]
    fn referenced_forward_attribute_change_is_coherent() {
        // Changing 傍点 → 傍線 keeps the same target, so the forward re-forms.
        let src = "青空がひろがる、その［＃「青空」に傍点］";
        let doc = Document::new(src);
        let tree = doc.parse();
        let r = role_of(src, RegionRole::ForwardReferenced);
        let spliced = tree
            .splice(r, "［＃「青空」に傍線］")
            .expect("attribute-only change keeps the forward");
        assert!(spliced.starts_with("青空がひろがる、その"));
        assert!(spliced.ends_with("［＃「青空」に傍線］"));
    }

    #[test]
    fn referenced_forward_target_change_is_coupled() {
        // Changing the target rewrites BOTH the bracket and the unique upstream
        // literal so the reference stays in sync.
        let src = "青空がひろがる、その［＃「青空」に傍点］";
        let doc = Document::new(src);
        let tree = doc.parse();
        let r = role_of(src, RegionRole::ForwardReferenced);
        let spliced = tree
            .splice(r, "［＃「海」に傍点］")
            .expect("target change is a coupled edit");
        assert_eq!(spliced, "海がひろがる、その［＃「海」に傍点］");
        // It re-parses to a forward reference again (now to 海).
        let rt = Document::new(spliced.as_str());
        assert!(
            rt.parse()
                .owned_regions()
                .iter()
                .any(|r| r.role == RegionRole::ForwardReferenced)
        );
    }

    #[test]
    fn ambiguous_referent_target_change_declines() {
        // 青空 appears twice upstream — no unique referent, so a target change
        // is honestly declined rather than guessing which copy to rewrite.
        let src = "青空と青空、その［＃「青空」に傍点］";
        let doc = Document::new(src);
        let tree = doc.parse();
        let r = role_of(src, RegionRole::ForwardReferenced);
        let err = tree
            .splice(r, "［＃「海」に傍点］")
            .expect_err("ambiguous referent declines");
        assert!(matches!(err, SpliceError::Unverifiable { .. }));
    }

    #[test]
    fn multi_target_forward_identity_is_a_noop() {
        // A 、-joined multi-target forward (`「A」「B」`) is `Referenced`; its
        // canonical target lowers to a single plain run ("A、B"), so its
        // identity splice re-forms through the scoped single-region verify and
        // is a no-op. `assert_tiling` runs the real splice machinery on every
        // region, pinning the identity invariant for this trickiest shape.
        assert_tiling("AとB［＃「A」「B」に傍点］");
    }

    #[test]
    fn multi_target_forward_target_change_declines() {
        // Changing the target of a multi-target forward is genuinely
        // irreducible: the canonical "A、B" is not a contiguous source substring
        // (the source reads "AとB"), so the upstream literal cannot be located
        // and the edit is honestly declined rather than guessed.
        let src = "AとB［＃「A」「B」に傍点］";
        let doc = Document::new(src);
        let tree = doc.parse();
        let r = role_of(src, RegionRole::ForwardReferenced);
        assert_eq!(
            r.safety,
            SpliceSafety::Coupled(CoupledKind::ForwardReference)
        );
        let err = tree
            .splice(r, "［＃「海」に傍点］")
            .expect_err("a multi-segment target change is irreducible");
        assert!(matches!(err, SpliceError::Unverifiable { .. }));
    }

    #[test]
    fn owned_region_at_finds_node_and_gap() {
        let src = "あ｜青梅《おうめ》い";
        let doc = Document::new(src);
        let tree = doc.parse();
        let head = tree.owned_region_at(SourceOffset::new(0)).unwrap();
        assert_eq!(head.role, RegionRole::Interstitial);
        assert_eq!(head.span.start, 0);
        let ruby_off = SourceOffset::new(tree.owned_regions()[1].span.start);
        let mid = tree.owned_region_at(ruby_off).unwrap();
        assert_eq!(mid.role, RegionRole::Ruby);
        assert!(
            tree.owned_region_at(SourceOffset::new(
                u32::try_from(tree.sanitized().len()).unwrap()
            ))
            .is_none(),
        );
    }
}
