//! Lexer-emitted observations.
//!
//! A [`Diagnostic`] is non-fatal: the lexer always produces a
//! best-effort output and never aborts mid-stream. Callers decide how
//! to surface the diagnostics — the CLI can render them via
//! [`miette::Report`], tests can assert on the variants, library
//! consumers can ignore them.
//!
//! Every variant carries a byte-range [`Span`] in the *sanitized* source
//! — the Phase 0 output (BOM stripped, CRLF→LF, 〔…〕 accents decomposed),
//! which is the text the later phases tokenize. To render a snippet,
//! attach that sanitized text (e.g. via `aozora::pipeline::lexer::sanitize`)
//! so miette's caret lands on the right character; for input with no BOM /
//! CRLF / accent digraphs the sanitized text equals the original bytes.
//!
//! # Severity and source axes
//!
//! Diagnostics split along two orthogonal axes:
//!
//! - **[`Severity`]** — `Error` / `Warning` / `Note`. Determines how
//!   strictly a host (CLI, LSP, editor decorator) should treat the
//!   observation. Defaults to `Error` for genuine syntax issues and
//!   `Warning` for input that the parser can carry around but the
//!   user should be told about.
//! - **[`DiagnosticSource`]** — `Source` (problem traces back to
//!   user input) vs. `Internal` (a pipeline-invariant violation;
//!   appearance indicates a library bug). Hosts that filter by
//!   `Internal` get a clear "library bug" channel without having to
//!   match on individual variants.
//!
//! Each variant exposes both axes through accessors
//! ([`Diagnostic::severity`] / [`Diagnostic::source`]).
//!
//! # Internal variant
//!
//! The four library-bug sanity checks
//! (`ResidualAnnotationMarker`, `UnregisteredSentinel`,
//! `RegistryOutOfOrder`, `RegistryPositionMismatch`) live as a single
//! [`Diagnostic::Internal`] variant whose `code` field
//! ([`InternalCheckCode`]) tags the specific check. Tests and tooling
//! match on that code via [`codes`]. Consumers that want to filter
//! library-bug diagnostics out of the [`crate::Diagnostic`] stream
//! reach for [`Diagnostic::source`].

use miette::Diagnostic as MietteDiagnostic;
use thiserror::Error;

use crate::PairKind;
use crate::Span;

/// Stable identifier strings for known [`Diagnostic`] variants.
///
/// [`Diagnostic::code`] returns one of these for any production
/// diagnostic. They are guaranteed stable across patch and minor
/// releases; major-release variant additions land new constants here
/// without touching existing ones.
pub mod codes {
    /// Source contains a lexer PUA sentinel codepoint.
    pub const SOURCE_CONTAINS_PUA: &str = "aozora::lex::source_contains_pua";

    /// Open delimiter reached end-of-input with no matching close.
    pub const UNCLOSED_BRACKET: &str = "aozora::lex::unclosed_bracket";

    /// Close delimiter saw an empty stack or a mismatched stack top.
    pub const UNMATCHED_CLOSE: &str = "aozora::lex::unmatched_close";

    /// A `〔…〕` accent digraph was decomposed during Phase 0 sanitize.
    pub const ACCENT_DECOMPOSITION_APPLIED: &str = "aozora::lex::accent_decomposition_applied";

    /// A 外字 (gaiji) reference resolved to neither Unicode nor JIS X 0213.
    pub const UNRESOLVED_GAIJI: &str = "aozora::lex::unresolved_gaiji";

    /// A paired container was closed by a closer of a different kind.
    pub const MISMATCHED_CONTAINER_CLOSE: &str = "aozora::lex::mismatched_container_close";

    /// An explicit-base ruby (`｜base《》`) had an empty reading.
    pub const EMPTY_RUBY_READING: &str = "aozora::lex::empty_ruby_reading";

    /// A ruby reading body itself opened another ruby (`《…《…》…》`).
    pub const NESTED_RUBY: &str = "aozora::lex::nested_ruby";

    /// A `［＃ここから…］` opener matched no known container kind.
    pub const UNRECOGNISED_CONTAINER_DIRECTIVE: &str =
        "aozora::lex::unrecognised_container_directive";

    /// A 縦中横 forward reference whose target is absent from the look-back.
    pub const TCY_TARGET_NOT_FOUND: &str = "aozora::lex::tcy_target_not_found";

    /// A forward-reference bouten target occurs more than once before it.
    pub const BOUTEN_TARGET_AMBIGUOUS: &str = "aozora::lex::bouten_target_ambiguous";

    /// A page/section break appeared inside a single-line container.
    pub const BREAK_IN_SINGLE_LINE_CONTAINER: &str = "aozora::lex::break_in_single_line_container";

    /// A bracketed kaeriten (`［＃二］`) has no matching lower-rank partner.
    pub const BRACKETED_KAERITEN_NO_PAIR: &str = "aozora::lex::bracketed_kaeriten_no_pair";

    /// A kaeriten appeared outside a 漢文-like context (lookahead heuristic).
    pub const KAERITEN_OUTSIDE_KANBUN: &str = "aozora::lex::kaeriten_outside_kanbun";

    /// A 傍点 range opener was closed by a 傍線 closer (or vice-versa).
    pub const MISMATCHED_BOUTEN_CONTAINER: &str = "aozora::lex::mismatched_bouten_container";

    /// Pipeline-internal: an `［＃` digraph survived classification
    /// into the normalized text. Indicates a missing recogniser for
    /// the keyword.
    pub const RESIDUAL_ANNOTATION_MARKER: &str = "aozora::lex::residual_annotation_marker";

    /// Pipeline-internal: a PUA sentinel codepoint is present in the
    /// normalized text at a position that is not recorded in the
    /// placeholder registry.
    ///
    /// Source-side PUA collisions emit [`SOURCE_CONTAINS_PUA`]
    /// upstream; this code is distinct.
    pub const UNREGISTERED_SENTINEL: &str = "aozora::lex::unregistered_sentinel";

    /// Pipeline-internal: a placeholder-registry vector is not
    /// strictly ordered by position. Indicates a normalizer driver
    /// bug.
    pub const REGISTRY_OUT_OF_ORDER: &str = "aozora::lex::registry_out_of_order";

    /// Pipeline-internal: a registry entry references a normalized
    /// byte position whose character does not match the expected
    /// sentinel kind.
    pub const REGISTRY_POSITION_MISMATCH: &str = "aozora::lex::registry_position_mismatch";
}

/// Severity of a [`Diagnostic`].
///
/// Hosts route diagnostics by severity: `Error` blocks downstream
/// rendering or fails CI, `Warning` decorates the editor surface,
/// `Note` is informational. The `aozora` library never panics on a
/// `Diagnostic` — the parser produces a best-effort output and
/// surfaces this enum as the host's policy hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Severity {
    /// Genuine error; downstream consumers should treat the parse as
    /// suspect.
    Error,
    /// Recoverable observation; parse continues and output is
    /// preserved, but the user should know.
    Warning,
    /// Informational note; editor surfaces may show it as a tooltip
    /// or annotation but it does not affect CI / build status.
    Note,
}

impl Severity {
    /// Every variant in declaration order. Used by codegen so
    /// downstream artefacts track the enum without drift.
    pub const ALL: [Self; 3] = [Self::Error, Self::Warning, Self::Note];

    /// Stable lowercase wire-format identifier ("error" / "warning"
    /// / "note"). The same string the driver wire format emits in
    /// the `severity` field of `DiagnosticWire`.
    #[must_use]
    pub const fn as_wire_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Note => "note",
        }
    }
}

/// Origin of a [`Diagnostic`] — distinguishes user-input issues from
/// library-internal sanity-check failures.
///
/// Production parses on well-formed input never emit `Internal`
/// diagnostics. An `Internal` diagnostic indicates a bug in
/// `aozora-pipeline` and SHOULD be reported upstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DiagnosticSource {
    /// Issue traces back to the user-provided source text.
    Source,
    /// Pipeline-internal invariant violation. Indicates a library
    /// bug; the parse is still completed best-effort but downstream
    /// tooling should surface this distinctly.
    Internal,
}

impl DiagnosticSource {
    /// Every variant in declaration order.
    pub const ALL: [Self; 2] = [Self::Source, Self::Internal];

    /// Stable lowercase wire-format identifier ("source" /
    /// "internal"). Matches the `source` field of `DiagnosticWire`.
    #[must_use]
    pub const fn as_wire_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Internal => "internal",
        }
    }
}

/// Identifier of a specific pipeline-internal sanity check.
///
/// Carried by the [`Diagnostic::Internal`] variant. Tooling that
/// wants per-check assertions matches on this enum; legacy callers
/// (logs, regex grep) can still reach for the stable
/// `aozora::lex::*` string via [`Self::as_code`].
///
/// `#[non_exhaustive]` so adding a new check variant is a minor
/// release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum InternalCheckCode {
    /// An `［＃` digraph survived classification into the normalized
    /// text. Indicates a missing recogniser for the keyword.
    ResidualAnnotationMarker,
    /// A PUA sentinel codepoint is present in the normalized text at
    /// a position that is not recorded in the placeholder registry.
    UnregisteredSentinel,
    /// A placeholder-registry vector is not strictly ordered by
    /// position. Indicates a normalizer driver bug.
    RegistryOutOfOrder,
    /// A registry entry references a normalized byte position whose
    /// character does not match the expected sentinel kind.
    RegistryPositionMismatch,
}

impl InternalCheckCode {
    /// All known internal check codes in declaration order.
    pub const ALL: [Self; 4] = [
        Self::ResidualAnnotationMarker,
        Self::UnregisteredSentinel,
        Self::RegistryOutOfOrder,
        Self::RegistryPositionMismatch,
    ];

    /// Stable `aozora::lex::*` string identifier for this check.
    /// Equivalent to the corresponding [`codes`] constant.
    #[must_use]
    pub const fn as_code(self) -> &'static str {
        match self {
            Self::ResidualAnnotationMarker => codes::RESIDUAL_ANNOTATION_MARKER,
            Self::UnregisteredSentinel => codes::UNREGISTERED_SENTINEL,
            Self::RegistryOutOfOrder => codes::REGISTRY_OUT_OF_ORDER,
            Self::RegistryPositionMismatch => codes::REGISTRY_POSITION_MISMATCH,
        }
    }
}

/// Observation emitted by any lexer phase.
#[derive(Debug, Clone, Error, MietteDiagnostic)]
#[non_exhaustive]
pub enum Diagnostic {
    /// Source contains a codepoint that collides with one of the
    /// lexer's PUA sentinel reservations
    /// ([`crate::INLINE_SENTINEL`], [`crate::BLOCK_LEAF_SENTINEL`],
    /// [`crate::BLOCK_OPEN_SENTINEL`], [`crate::BLOCK_CLOSE_SENTINEL`]).
    /// Downstream phases will emit those same codepoints into normalized
    /// text, so a collision means the placeholder registry can no longer
    /// distinguish source-text occurrences from lexer-inserted markers.
    #[error("source contains lexer PUA sentinel codepoint {codepoint:?}")]
    #[diagnostic(
        code("aozora::lex::source_contains_pua"),
        url("https://p4suta.github.io/aozora/notation/diagnostics.html#source-contains-pua"),
        severity(Warning),
        help(
            "the lexer reserves U+E001..U+E004 as inline/block markers; \
             a source-side occurrence will confuse the placeholder registry"
        )
    )]
    SourceContainsPua {
        #[label("here")]
        at: miette::SourceSpan,
        codepoint: char,
        /// Byte-range in the sanitized source for programmatic consumers
        /// that don't need miette's [`miette::SourceSpan`].
        span: Span,
    },

    /// An open delimiter reached end-of-input with no matching close on
    /// the pairing stack.
    #[error("unclosed Aozora {kind:?} bracket")]
    #[diagnostic(
        code("aozora::lex::unclosed_bracket"),
        url("https://p4suta.github.io/aozora/notation/diagnostics.html#unclosed-bracket"),
        help(
            "the opener has no matching close delimiter — either the close \
             was omitted or an earlier close matched a nested opener"
        )
    )]
    UnclosedBracket {
        #[label("opened here")]
        at: miette::SourceSpan,
        kind: PairKind,
        /// Byte-range of the unmatched *open* delimiter in the sanitized
        /// source.
        span: Span,
    },

    /// A close delimiter was seen with an empty stack, or with a stack
    /// top of a different [`PairKind`].
    #[error("unmatched Aozora {kind:?} close delimiter")]
    #[diagnostic(
        code("aozora::lex::unmatched_close"),
        url("https://p4suta.github.io/aozora/notation/diagnostics.html#unmatched-close"),
        help(
            "no matching open on the pairing stack — either the open was \
             omitted or an inner unmatched close consumed it"
        )
    )]
    UnmatchedClose {
        #[label("close here")]
        at: miette::SourceSpan,
        kind: PairKind,
        /// Byte-range of the stray *close* delimiter.
        span: Span,
    },

    /// A `〔…〕` accent digraph (e.g. `〔e'〕` → `é`) was decomposed into
    /// its Unicode-combined form during Phase 0 sanitize. Purely
    /// informational: the decomposition is intended behaviour (ADR-0003),
    /// surfaced as a `Note` so an editor can show what changed. The
    /// serializer reconstructs the original `〔…〕` form, so the transform
    /// is loss-free.
    #[error("accent digraph decomposed in Phase 0 sanitize")]
    #[diagnostic(
        code("aozora::lex::accent_decomposition_applied"),
        url(
            "https://p4suta.github.io/aozora/notation/diagnostics.html#accent-decomposition-applied"
        ),
        severity(Advice),
        help(
            "the `〔…〕` accent span was rewritten to its combined Unicode form; \
             this is expected and round-trips back to the source on serialize"
        )
    )]
    AccentDecompositionApplied {
        #[label("decomposed here")]
        at: miette::SourceSpan,
        /// Byte-range of the `〔…〕` span in the sanitized (post-decomposition)
        /// source.
        span: Span,
    },

    /// A 外字 (gaiji) reference — `※［＃…］` — resolved to neither a Unicode
    /// scalar nor a JIS X 0213 cell, so the renderer falls back to the
    /// description text rather than the intended glyph.
    #[error("gaiji reference resolved to neither Unicode nor JIS X 0213")]
    #[diagnostic(
        code("aozora::lex::unresolved_gaiji"),
        url("https://p4suta.github.io/aozora/notation/diagnostics.html#unresolved-gaiji"),
        severity(Warning),
        help(
            "no JIS X 0213 men-ku-ten or U+XXXX reference matched and the \
             description is not a single resolvable character — the glyph \
             renders as its description text only"
        )
    )]
    UnresolvedGaiji {
        #[label("unresolved gaiji")]
        at: miette::SourceSpan,
        /// Byte-range of the `※［＃…］` reference in the sanitized source.
        span: Span,
    },

    /// A paired container opened with one kind (`［＃ここから2字下げ］`)
    /// was closed by a closer of a different kind
    /// (`［＃ここで地付き終わり］`). The label points at the *close* marker.
    ///
    /// `open_kind` / `close_kind` are the stable lowercase container-family
    /// tags (`indent` / `warichu` / `keigakomi` / `align-end`); they are
    /// `&'static str` rather than the `aozora_syntax::ContainerKind` enum
    /// because this crate sits below `aozora-syntax`.
    #[error("container opened as `{open_kind}` closed by a `{close_kind}` closer")]
    #[diagnostic(
        code("aozora::lex::mismatched_container_close"),
        url(
            "https://p4suta.github.io/aozora/notation/diagnostics.html#mismatched-container-close"
        ),
        help(
            "the close directive names a different container family than the \
             open — pair `ここから字下げ` with `ここで字下げ終わり`, `ここから地付き` \
             with `ここで地付き終わり`, etc."
        )
    )]
    MismatchedContainerClose {
        #[label("mismatched close")]
        at: miette::SourceSpan,
        /// Container family of the *open* marker on the pairing stack.
        open_kind: &'static str,
        /// Container family named by the *close* marker.
        close_kind: &'static str,
        /// Byte-range of the close marker in the sanitized source.
        span: Span,
    },

    /// An explicit-base ruby — `｜base《》` — supplied a base but an empty
    /// reading. The base is present (a `｜` precedes the `《`), so this is
    /// a genuine authoring slip, not a bare `《》` literal run. The
    /// construct degrades to plain text. The label spans the whole
    /// `｜base《》`.
    #[error("ruby base given but reading is empty")]
    #[diagnostic(
        code("aozora::lex::empty_ruby_reading"),
        url("https://p4suta.github.io/aozora/notation/diagnostics.html#empty-ruby-reading"),
        help(
            "the `《…》` reading after the `｜` base is empty — supply a reading \
             or remove the `｜…《》` markers to keep the base as plain text"
        )
    )]
    EmptyRubyReading {
        #[label("empty reading")]
        at: miette::SourceSpan,
        /// Byte-range of the `｜base《》` construct in the sanitized source.
        span: Span,
    },

    /// A ruby reading body opened another ruby (`｜漢《字《かん》》`). Ruby
    /// does not nest; the inner `《…》` is the offending opener. The outer
    /// ruby is still parsed best-effort. The label points at the inner
    /// `《`.
    #[error("ruby reading contains a nested ruby")]
    #[diagnostic(
        code("aozora::lex::nested_ruby"),
        url("https://p4suta.github.io/aozora/notation/diagnostics.html#nested-ruby"),
        help(
            "ruby cannot nest — close the outer reading before the inner `《`, \
             or remove the inner `《…》`"
        )
    )]
    NestedRuby {
        #[label("nested ruby opens here")]
        at: miette::SourceSpan,
        /// Byte-range of the inner `《` opener in the sanitized source.
        span: Span,
    },

    /// A `［＃ここから…］` directive looked like a paired-container opener
    /// but named no known container kind (`字下げ` / `地付き` /
    /// `地から…字上げ`). It is kept as an `Annotation{Unknown}` (so output
    /// is preserved) but not treated as a container. The label spans the
    /// directive.
    #[error("unrecognised container directive")]
    #[diagnostic(
        code("aozora::lex::unrecognised_container_directive"),
        url(
            "https://p4suta.github.io/aozora/notation/diagnostics.html#unrecognised-container-directive"
        ),
        severity(Warning),
        help(
            "`［＃ここから…］` must name a known container — `字下げ`, `地付き`, \
             `地から N 字上げ`; this directive was kept as a plain annotation"
        )
    )]
    UnrecognisedContainerDirective {
        #[label("unrecognised directive")]
        at: miette::SourceSpan,
        /// Byte-range of the `［＃ここから…］` directive in the sanitized
        /// source.
        span: Span,
    },

    /// A 縦中横 forward reference (`［＃「X」は縦中横］`) named a target `X`
    /// that does not appear anywhere in the preceding text, so it has no
    /// run to style. The directive degrades to an `Annotation{Unknown}`.
    /// The label spans the directive.
    #[error("縦中横 target not found in the preceding text")]
    #[diagnostic(
        code("aozora::lex::tcy_target_not_found"),
        url("https://p4suta.github.io/aozora/notation/diagnostics.html#tcy-target-not-found"),
        severity(Warning),
        help(
            "the quoted 縦中横 target must occur earlier in the line — check the \
             spelling, or place the `［＃「X」は縦中横］` after the run it styles"
        )
    )]
    TcyTargetNotFound {
        #[label("target has no referent")]
        at: miette::SourceSpan,
        /// Byte-range of the `［＃「X」は縦中横］` directive in the sanitized
        /// source.
        span: Span,
    },

    /// A forward-reference bouten (`［＃「X」に傍点］`) named a target `X`
    /// that occurs more than once in the preceding text, so which run it
    /// emphasises is ambiguous. The parser applies it to the match per its
    /// look-back rule, but the author should disambiguate. The label spans
    /// the directive.
    #[error("ambiguous bouten target: more than one candidate run precedes it")]
    #[diagnostic(
        code("aozora::lex::bouten_target_ambiguous"),
        url("https://p4suta.github.io/aozora/notation/diagnostics.html#bouten-target-ambiguous"),
        severity(Warning),
        help(
            "the quoted target appears more than once before the `［＃…］` — the \
             styled run may not be the intended one; reword so the target is unique"
        )
    )]
    BoutenTargetAmbiguous {
        #[label("ambiguous target")]
        at: miette::SourceSpan,
        /// Byte-range of the `［＃「X」に傍点］` directive in the sanitized
        /// source.
        span: Span,
    },

    /// A page or section break (`［＃改ページ］` / `［＃改段］` / …) appeared
    /// inside a single-line container — a single-line layout directive
    /// (`［＃地付き］` / `［＃N字下げ］`) sharing a source line with a later
    /// break, or a break between `［＃割り注］` and `［＃割り注終わり］`. A
    /// single-line container governs only the rest of its line, so a break
    /// on that line drops the container's effect. The label points at the
    /// break. `container` is the stable family tag of the dropped container
    /// (`indent` / `align-end` / `warichu`).
    #[error("page/section break inside a single-line `{container}` container")]
    #[diagnostic(
        code("aozora::lex::break_in_single_line_container"),
        url(
            "https://p4suta.github.io/aozora/notation/diagnostics.html#break-in-single-line-container"
        ),
        severity(Warning),
        help(
            "a single-line container governs only the rest of its line — move \
             the break off the line, or use the paired `［＃ここから…］` … \
             `［＃ここで…終わり］` block form that persists across breaks"
        )
    )]
    BreakInSingleLineContainer {
        #[label("break drops the container")]
        at: miette::SourceSpan,
        /// Stable family tag of the dropped single-line container
        /// (`indent` / `align-end` / `warichu`).
        container: &'static str,
        /// Byte-range of the break directive in the sanitized source.
        span: Span,
    },

    /// A bracketed kaeriten of rank ≥ 2 (`［＃二］` / `［＃下］` / `［＃乙］` …)
    /// appeared in a document whose matching family base (`［＃一］` /
    /// `［＃上］` / `［＃甲］`) is absent entirely — there is nothing for the
    /// return mark to pair back to. The check is document-wide and
    /// base-only: kanbun return-mark groups routinely span `、` / `。` and
    /// line boundaries and 上下点 skips `中`, so any narrower scope misfires
    /// on valid kanbun. The label points at the unpaired mark.
    #[error("bracketed kaeriten has no matching base mark in the document")]
    #[diagnostic(
        code("aozora::lex::bracketed_kaeriten_no_pair"),
        url(
            "https://p4suta.github.io/aozora/notation/diagnostics.html#bracketed-kaeriten-no-pair"
        ),
        help(
            "a return mark needs its family base somewhere in the document — \
             a `［＃二］`/`［＃三］` needs a `［＃一］`, a `［＃下］`/`［＃中］` needs \
             a `［＃上］`, a `［＃乙］`… needs a `［＃甲］`"
        )
    )]
    BracketedKaeritenNoPair {
        #[label("unpaired kaeriten")]
        at: miette::SourceSpan,
        /// Byte-range of the `［＃…］` kaeriten directive in the sanitized
        /// source.
        span: Span,
    },

    /// A kaeriten (`［＃二］` / `［＃レ］` / …) appeared outside a 漢文-like
    /// context — it is the only kaeriten in the document and its
    /// surroundings read as ordinary kana prose, so the mark is most likely
    /// a stray annotation rather than a genuine return mark. Conservative
    /// lookahead heuristic: a document with a cluster of kaeriten is never
    /// flagged. The label points at the lone mark.
    #[error("kaeriten outside a 漢文-like context")]
    #[diagnostic(
        code("aozora::lex::kaeriten_outside_kanbun"),
        url("https://p4suta.github.io/aozora/notation/diagnostics.html#kaeriten-outside-kanbun"),
        severity(Warning),
        help(
            "this is the only kaeriten in the document and its surroundings \
             look like ordinary prose — check it is a genuine 返り点 and not a \
             stray `［＃…］` annotation"
        )
    )]
    KaeritenOutsideKanbun {
        #[label("isolated kaeriten")]
        at: miette::SourceSpan,
        /// Byte-range of the `［＃…］` kaeriten directive in the sanitized
        /// source.
        span: Span,
    },

    /// A 傍点 / 傍線 range form (`［＃傍点］ … ［＃傍点終わり］`) was opened with
    /// one family (点 / 線) and closed by the other — e.g. a `［＃傍点］`
    /// opener closed by `［＃傍線終わり］`. The two families render
    /// differently (dots vs a line), so the run's emphasis is ambiguous.
    /// The parser recovers by keying the run to the opener's variant. The
    /// label points at the close marker. `open_family` / `close_family`
    /// are the stable family tags (`傍点` / `傍線`).
    #[error("傍点 range opened as `{open_family}` closed by a `{close_family}` closer")]
    #[diagnostic(
        code("aozora::lex::mismatched_bouten_container"),
        url(
            "https://p4suta.github.io/aozora/notation/diagnostics.html#mismatched-bouten-container"
        ),
        help(
            "close a 傍点 range with `［＃傍点終わり］` (any 点 variant) and a 傍線 \
             range with `［＃傍線終わり］` (any 線 variant) — match the opener's \
             family"
        )
    )]
    MismatchedBoutenContainer {
        #[label("mismatched close")]
        at: miette::SourceSpan,
        /// Family of the *open* marker (`傍点` / `傍線`).
        open_family: &'static str,
        /// Family named by the *close* marker.
        close_family: &'static str,
        /// Byte-range of the close marker in the sanitized source.
        span: Span,
    },

    /// Pipeline-internal sanity-check failure — production parses on
    /// well-formed input never emit this. The [`check`](Self::Internal)
    /// payload identifies the specific check via the typed
    /// [`InternalCheckCode`] enum; tooling that prefers the stable
    /// string identifier reaches via
    /// [`Self::code`](Self::code). Library consumers that just want
    /// to filter "library bugs" out of the stream check
    /// [`source`](Self::source) instead.
    #[error("internal aozora pipeline check failed: {}", check.as_code())]
    #[diagnostic(
        code("aozora::internal"),
        url("https://p4suta.github.io/aozora/notation/diagnostics.html#internal"),
        help(
            "this is a pipeline-internal sanity check; appearance \
             indicates a bug in aozora — please report at \
             https://github.com/P4suta/aozora/issues with the source \
             that triggered it"
        )
    )]
    Internal {
        #[label("at this position")]
        at: miette::SourceSpan,
        /// Typed identifier for the specific check that fired. Pin
        /// per-check assertions on this rather than the stringly-typed
        /// [`code`](Self::code) accessor so the compiler enforces
        /// match exhaustiveness at the call site.
        check: InternalCheckCode,
        /// Byte-range covering the violation site.
        span: Span,
    },
}

/// Introspected metadata for a diagnostic code — the data behind
/// `aozora explain <code>`.
///
/// Returned by [`Diagnostic::explain`]. `help` and `url` are read from
/// the live [`miette::Diagnostic`] impl of a representative instance, so
/// they cannot drift from what `aozora check` renders for the same
/// diagnostic.
#[derive(Debug, Clone)]
pub struct DiagnosticInfo {
    /// Stable `aozora::lex::*` code (see [`codes`]).
    pub code: &'static str,
    /// Severity routing axis.
    pub severity: Severity,
    /// Origin axis: user input vs. library-internal.
    pub source: DiagnosticSource,
    /// One-line remediation help — the `#[diagnostic(help(…))]` text.
    pub help: String,
    /// Documentation URL for the code, when the variant carries one.
    pub url: Option<String>,
}

#[allow(
    clippy::same_name_method,
    reason = "intentional: our inherent severity() / code() return strongly-typed (Severity enum, &'static str) values that mirror miette::Diagnostic's loosely-typed defaults — callers prefer the inherent method"
)]
impl Diagnostic {
    /// Constructor for [`Diagnostic::SourceContainsPua`].
    #[must_use]
    pub fn source_contains_pua(at: Span, codepoint: char) -> Self {
        let (offset, length) = span_to_miette_parts(at);
        Self::SourceContainsPua {
            at: miette::SourceSpan::new(offset.into(), length),
            codepoint,
            span: at,
        }
    }

    /// Constructor for [`Diagnostic::UnclosedBracket`].
    #[must_use]
    pub fn unclosed_bracket(at: Span, kind: PairKind) -> Self {
        let (offset, length) = span_to_miette_parts(at);
        Self::UnclosedBracket {
            at: miette::SourceSpan::new(offset.into(), length),
            kind,
            span: at,
        }
    }

    /// Constructor for [`Diagnostic::UnmatchedClose`].
    #[must_use]
    pub fn unmatched_close(at: Span, kind: PairKind) -> Self {
        let (offset, length) = span_to_miette_parts(at);
        Self::UnmatchedClose {
            at: miette::SourceSpan::new(offset.into(), length),
            kind,
            span: at,
        }
    }

    /// Constructor for [`Diagnostic::AccentDecompositionApplied`].
    #[must_use]
    pub fn accent_decomposition_applied(at: Span) -> Self {
        let (offset, length) = span_to_miette_parts(at);
        Self::AccentDecompositionApplied {
            at: miette::SourceSpan::new(offset.into(), length),
            span: at,
        }
    }

    /// Constructor for [`Diagnostic::UnresolvedGaiji`].
    #[must_use]
    pub fn unresolved_gaiji(at: Span) -> Self {
        let (offset, length) = span_to_miette_parts(at);
        Self::UnresolvedGaiji {
            at: miette::SourceSpan::new(offset.into(), length),
            span: at,
        }
    }

    /// Constructor for [`Diagnostic::MismatchedContainerClose`]. The
    /// `open_kind` / `close_kind` are the stable container-family tags
    /// (`aozora_syntax::ContainerKind::kind_str`).
    #[must_use]
    pub fn mismatched_container_close(
        at: Span,
        open_kind: &'static str,
        close_kind: &'static str,
    ) -> Self {
        let (offset, length) = span_to_miette_parts(at);
        Self::MismatchedContainerClose {
            at: miette::SourceSpan::new(offset.into(), length),
            open_kind,
            close_kind,
            span: at,
        }
    }

    /// Constructor for [`Diagnostic::EmptyRubyReading`].
    #[must_use]
    pub fn empty_ruby_reading(at: Span) -> Self {
        let (offset, length) = span_to_miette_parts(at);
        Self::EmptyRubyReading {
            at: miette::SourceSpan::new(offset.into(), length),
            span: at,
        }
    }

    /// Constructor for [`Diagnostic::NestedRuby`].
    #[must_use]
    pub fn nested_ruby(at: Span) -> Self {
        let (offset, length) = span_to_miette_parts(at);
        Self::NestedRuby {
            at: miette::SourceSpan::new(offset.into(), length),
            span: at,
        }
    }

    /// Constructor for [`Diagnostic::UnrecognisedContainerDirective`].
    #[must_use]
    pub fn unrecognised_container_directive(at: Span) -> Self {
        let (offset, length) = span_to_miette_parts(at);
        Self::UnrecognisedContainerDirective {
            at: miette::SourceSpan::new(offset.into(), length),
            span: at,
        }
    }

    /// Constructor for [`Diagnostic::TcyTargetNotFound`].
    #[must_use]
    pub fn tcy_target_not_found(at: Span) -> Self {
        let (offset, length) = span_to_miette_parts(at);
        Self::TcyTargetNotFound {
            at: miette::SourceSpan::new(offset.into(), length),
            span: at,
        }
    }

    /// Constructor for [`Diagnostic::BoutenTargetAmbiguous`].
    #[must_use]
    pub fn bouten_target_ambiguous(at: Span) -> Self {
        let (offset, length) = span_to_miette_parts(at);
        Self::BoutenTargetAmbiguous {
            at: miette::SourceSpan::new(offset.into(), length),
            span: at,
        }
    }

    /// Constructor for [`Diagnostic::BreakInSingleLineContainer`]. The
    /// `container` is the stable family tag of the dropped single-line
    /// container (`indent` / `align-end` / `warichu`).
    #[must_use]
    pub fn break_in_single_line_container(at: Span, container: &'static str) -> Self {
        let (offset, length) = span_to_miette_parts(at);
        Self::BreakInSingleLineContainer {
            at: miette::SourceSpan::new(offset.into(), length),
            container,
            span: at,
        }
    }

    /// Constructor for [`Diagnostic::BracketedKaeritenNoPair`].
    #[must_use]
    pub fn bracketed_kaeriten_no_pair(at: Span) -> Self {
        let (offset, length) = span_to_miette_parts(at);
        Self::BracketedKaeritenNoPair {
            at: miette::SourceSpan::new(offset.into(), length),
            span: at,
        }
    }

    /// Constructor for [`Diagnostic::KaeritenOutsideKanbun`].
    #[must_use]
    pub fn kaeriten_outside_kanbun(at: Span) -> Self {
        let (offset, length) = span_to_miette_parts(at);
        Self::KaeritenOutsideKanbun {
            at: miette::SourceSpan::new(offset.into(), length),
            span: at,
        }
    }

    /// Constructor for [`Diagnostic::MismatchedBoutenContainer`]. The
    /// `open_family` / `close_family` are the stable 点/線 family tags
    /// (`aozora_syntax::BoutenKind::family_str`).
    #[must_use]
    pub fn mismatched_bouten_container(
        at: Span,
        open_family: &'static str,
        close_family: &'static str,
    ) -> Self {
        let (offset, length) = span_to_miette_parts(at);
        Self::MismatchedBoutenContainer {
            at: miette::SourceSpan::new(offset.into(), length),
            open_family,
            close_family,
            span: at,
        }
    }

    /// Constructor for [`Diagnostic::Internal`]. Takes a typed
    /// [`InternalCheckCode`] — the compiler enforces that every
    /// production emit-site classifies the check correctly.
    #[must_use]
    pub fn internal(at: Span, check: InternalCheckCode) -> Self {
        let (offset, length) = span_to_miette_parts(at);
        Self::Internal {
            at: miette::SourceSpan::new(offset.into(), length),
            check,
            span: at,
        }
    }

    /// Severity routing axis. See [`Severity`].
    ///
    /// `#[non_exhaustive]` puts the responsibility on every match
    /// here for adding-new-variant time, not on a catch-all arm —
    /// the compiler will refuse to build until the new variant is
    /// classified.
    #[must_use]
    pub fn severity(&self) -> Severity {
        match self {
            Self::SourceContainsPua { .. }
            | Self::UnresolvedGaiji { .. }
            | Self::UnrecognisedContainerDirective { .. }
            | Self::TcyTargetNotFound { .. }
            | Self::BoutenTargetAmbiguous { .. }
            | Self::BreakInSingleLineContainer { .. }
            | Self::KaeritenOutsideKanbun { .. } => Severity::Warning,
            Self::AccentDecompositionApplied { .. } => Severity::Note,
            Self::UnclosedBracket { .. }
            | Self::UnmatchedClose { .. }
            | Self::MismatchedContainerClose { .. }
            | Self::EmptyRubyReading { .. }
            | Self::NestedRuby { .. }
            | Self::BracketedKaeritenNoPair { .. }
            | Self::MismatchedBoutenContainer { .. }
            | Self::Internal { .. } => Severity::Error,
        }
    }

    /// Origin axis: user input vs. pipeline-internal. See
    /// [`DiagnosticSource`].
    #[must_use]
    pub fn source(&self) -> DiagnosticSource {
        match self {
            Self::SourceContainsPua { .. }
            | Self::UnclosedBracket { .. }
            | Self::UnmatchedClose { .. }
            | Self::AccentDecompositionApplied { .. }
            | Self::UnresolvedGaiji { .. }
            | Self::MismatchedContainerClose { .. }
            | Self::EmptyRubyReading { .. }
            | Self::NestedRuby { .. }
            | Self::UnrecognisedContainerDirective { .. }
            | Self::TcyTargetNotFound { .. }
            | Self::BoutenTargetAmbiguous { .. }
            | Self::BreakInSingleLineContainer { .. }
            | Self::BracketedKaeritenNoPair { .. }
            | Self::KaeritenOutsideKanbun { .. }
            | Self::MismatchedBoutenContainer { .. } => DiagnosticSource::Source,
            Self::Internal { .. } => DiagnosticSource::Internal,
        }
    }

    /// Byte-range covering the diagnostic.
    #[must_use]
    pub fn span(&self) -> Span {
        match self {
            Self::SourceContainsPua { span, .. }
            | Self::UnclosedBracket { span, .. }
            | Self::UnmatchedClose { span, .. }
            | Self::AccentDecompositionApplied { span, .. }
            | Self::UnresolvedGaiji { span, .. }
            | Self::MismatchedContainerClose { span, .. }
            | Self::EmptyRubyReading { span, .. }
            | Self::NestedRuby { span, .. }
            | Self::UnrecognisedContainerDirective { span, .. }
            | Self::TcyTargetNotFound { span, .. }
            | Self::BoutenTargetAmbiguous { span, .. }
            | Self::BreakInSingleLineContainer { span, .. }
            | Self::BracketedKaeritenNoPair { span, .. }
            | Self::KaeritenOutsideKanbun { span, .. }
            | Self::MismatchedBoutenContainer { span, .. }
            | Self::Internal { span, .. } => *span,
        }
    }

    /// Stable string identifier for this diagnostic. Returns one of
    /// the constants from [`codes`] for production variants, or the
    /// `Internal` payload's [`InternalCheckCode::as_code`] for
    /// pipeline-internal checks.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::SourceContainsPua { .. } => codes::SOURCE_CONTAINS_PUA,
            Self::UnclosedBracket { .. } => codes::UNCLOSED_BRACKET,
            Self::UnmatchedClose { .. } => codes::UNMATCHED_CLOSE,
            Self::AccentDecompositionApplied { .. } => codes::ACCENT_DECOMPOSITION_APPLIED,
            Self::UnresolvedGaiji { .. } => codes::UNRESOLVED_GAIJI,
            Self::MismatchedContainerClose { .. } => codes::MISMATCHED_CONTAINER_CLOSE,
            Self::EmptyRubyReading { .. } => codes::EMPTY_RUBY_READING,
            Self::NestedRuby { .. } => codes::NESTED_RUBY,
            Self::UnrecognisedContainerDirective { .. } => codes::UNRECOGNISED_CONTAINER_DIRECTIVE,
            Self::TcyTargetNotFound { .. } => codes::TCY_TARGET_NOT_FOUND,
            Self::BoutenTargetAmbiguous { .. } => codes::BOUTEN_TARGET_AMBIGUOUS,
            Self::BreakInSingleLineContainer { .. } => codes::BREAK_IN_SINGLE_LINE_CONTAINER,
            Self::BracketedKaeritenNoPair { .. } => codes::BRACKETED_KAERITEN_NO_PAIR,
            Self::KaeritenOutsideKanbun { .. } => codes::KAERITEN_OUTSIDE_KANBUN,
            Self::MismatchedBoutenContainer { .. } => codes::MISMATCHED_BOUTEN_CONTAINER,
            Self::Internal { check, .. } => check.as_code(),
        }
    }

    /// Every stable diagnostic code [`Self::code`] can return, in
    /// catalogue order: the fifteen source-level codes followed by the
    /// four pipeline-internal check codes. Backs `aozora explain`'s
    /// catalogue and the round-trip coverage test.
    pub const ALL_CODES: [&'static str; 19] = [
        codes::SOURCE_CONTAINS_PUA,
        codes::UNCLOSED_BRACKET,
        codes::UNMATCHED_CLOSE,
        codes::ACCENT_DECOMPOSITION_APPLIED,
        codes::UNRESOLVED_GAIJI,
        codes::MISMATCHED_CONTAINER_CLOSE,
        codes::EMPTY_RUBY_READING,
        codes::NESTED_RUBY,
        codes::UNRECOGNISED_CONTAINER_DIRECTIVE,
        codes::TCY_TARGET_NOT_FOUND,
        codes::BOUTEN_TARGET_AMBIGUOUS,
        codes::BREAK_IN_SINGLE_LINE_CONTAINER,
        codes::BRACKETED_KAERITEN_NO_PAIR,
        codes::KAERITEN_OUTSIDE_KANBUN,
        codes::MISMATCHED_BOUTEN_CONTAINER,
        codes::RESIDUAL_ANNOTATION_MARKER,
        codes::UNREGISTERED_SENTINEL,
        codes::REGISTRY_OUT_OF_ORDER,
        codes::REGISTRY_POSITION_MISMATCH,
    ];

    /// Introspect the diagnostic identified by `code` — one of
    /// [`Self::ALL_CODES`] (equivalently a [`codes`] constant or an
    /// [`InternalCheckCode::as_code`]). `None` for an unknown code.
    ///
    /// `severity` / `source` come from the inherent accessors; `help` /
    /// `url` are read from the live [`miette::Diagnostic`] impl of a
    /// representative instance, so the explanation always agrees with
    /// what `aozora check` prints for the same diagnostic.
    #[must_use]
    pub fn explain(code: &str) -> Option<DiagnosticInfo> {
        let sample = Self::sample_for_code(code)?;
        Some(DiagnosticInfo {
            code: sample.code(),
            severity: sample.severity(),
            source: sample.source(),
            help: MietteDiagnostic::help(&sample)
                .map(|h| h.to_string())
                .unwrap_or_default(),
            url: MietteDiagnostic::url(&sample).map(|u| u.to_string()),
        })
    }

    /// A representative instance of the variant a `code` names, for
    /// introspection (reading miette help/url without a real parse).
    /// Spans are placeholder-empty; the four internal codes all map to
    /// the single [`Self::Internal`] variant, which shares one help/url.
    fn sample_for_code(code: &str) -> Option<Self> {
        let at = Span::new(0, 0);
        Some(match code {
            codes::SOURCE_CONTAINS_PUA => Self::source_contains_pua(at, '\u{E001}'),
            codes::UNCLOSED_BRACKET => Self::unclosed_bracket(at, PairKind::Bracket),
            codes::UNMATCHED_CLOSE => Self::unmatched_close(at, PairKind::Bracket),
            codes::ACCENT_DECOMPOSITION_APPLIED => Self::accent_decomposition_applied(at),
            codes::UNRESOLVED_GAIJI => Self::unresolved_gaiji(at),
            codes::MISMATCHED_CONTAINER_CLOSE => {
                Self::mismatched_container_close(at, "indent", "align-end")
            }
            codes::EMPTY_RUBY_READING => Self::empty_ruby_reading(at),
            codes::NESTED_RUBY => Self::nested_ruby(at),
            codes::UNRECOGNISED_CONTAINER_DIRECTIVE => Self::unrecognised_container_directive(at),
            codes::TCY_TARGET_NOT_FOUND => Self::tcy_target_not_found(at),
            codes::BOUTEN_TARGET_AMBIGUOUS => Self::bouten_target_ambiguous(at),
            codes::BREAK_IN_SINGLE_LINE_CONTAINER => {
                Self::break_in_single_line_container(at, "align-end")
            }
            codes::BRACKETED_KAERITEN_NO_PAIR => Self::bracketed_kaeriten_no_pair(at),
            codes::KAERITEN_OUTSIDE_KANBUN => Self::kaeriten_outside_kanbun(at),
            codes::MISMATCHED_BOUTEN_CONTAINER => {
                Self::mismatched_bouten_container(at, "傍点", "傍線")
            }
            codes::RESIDUAL_ANNOTATION_MARKER => {
                Self::internal(at, InternalCheckCode::ResidualAnnotationMarker)
            }
            codes::UNREGISTERED_SENTINEL => {
                Self::internal(at, InternalCheckCode::UnregisteredSentinel)
            }
            codes::REGISTRY_OUT_OF_ORDER => {
                Self::internal(at, InternalCheckCode::RegistryOutOfOrder)
            }
            codes::REGISTRY_POSITION_MISMATCH => {
                Self::internal(at, InternalCheckCode::RegistryPositionMismatch)
            }
            _ => return None,
        })
    }
}

/// Split a [`Span`] into the `(offset, length)` pair miette wants.
const fn span_to_miette_parts(span: Span) -> (usize, usize) {
    let offset = span.start as usize;
    let length = (span.end - span.start) as usize;
    (offset, length)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_contains_pua_round_trips_span() {
        let diag = Diagnostic::source_contains_pua(Span::new(5, 8), '\u{E001}');
        let Diagnostic::SourceContainsPua {
            codepoint, span, ..
        } = diag
        else {
            panic!("expected SourceContainsPua, got {diag:?}");
        };
        assert_eq!(codepoint, '\u{E001}');
        assert_eq!(span, Span::new(5, 8));
    }

    #[test]
    fn source_contains_pua_is_warning_severity() {
        let diag = Diagnostic::source_contains_pua(Span::new(0, 3), '\u{E002}');
        assert_eq!(diag.severity(), Severity::Warning);
        assert_eq!(diag.source(), DiagnosticSource::Source);
        assert_eq!(diag.code(), codes::SOURCE_CONTAINS_PUA);
    }

    #[test]
    fn source_contains_pua_display_mentions_codepoint() {
        let diag = Diagnostic::source_contains_pua(Span::new(0, 3), '\u{E002}');
        let rendered = format!("{diag}");
        assert!(
            rendered.contains("E002")
                || rendered.contains("\\u{e002}")
                || rendered.contains('\u{E002}')
        );
    }

    #[test]
    fn unclosed_bracket_round_trips_span_and_kind() {
        let diag = Diagnostic::unclosed_bracket(Span::new(3, 6), PairKind::Bracket);
        match diag {
            Diagnostic::UnclosedBracket { kind, span, .. } => {
                assert_eq!(kind, PairKind::Bracket);
                assert_eq!(span, Span::new(3, 6));
            }
            other => panic!("expected UnclosedBracket, got {other:?}"),
        }
    }

    #[test]
    fn unclosed_bracket_is_error_severity_from_source() {
        let diag = Diagnostic::unclosed_bracket(Span::new(0, 3), PairKind::Bracket);
        assert_eq!(diag.severity(), Severity::Error);
        assert_eq!(diag.source(), DiagnosticSource::Source);
        assert_eq!(diag.code(), codes::UNCLOSED_BRACKET);
    }

    #[test]
    fn unmatched_close_round_trips_span_and_kind() {
        let diag = Diagnostic::unmatched_close(Span::new(7, 10), PairKind::Ruby);
        match diag {
            Diagnostic::UnmatchedClose { kind, span, .. } => {
                assert_eq!(kind, PairKind::Ruby);
                assert_eq!(span, Span::new(7, 10));
            }
            other => panic!("expected UnmatchedClose, got {other:?}"),
        }
    }

    #[test]
    fn unmatched_close_is_error_severity_from_source() {
        let diag = Diagnostic::unmatched_close(Span::new(0, 3), PairKind::Quote);
        assert_eq!(diag.severity(), Severity::Error);
        assert_eq!(diag.source(), DiagnosticSource::Source);
        assert_eq!(diag.code(), codes::UNMATCHED_CLOSE);
    }

    #[test]
    fn unclosed_bracket_display_mentions_kind() {
        let diag = Diagnostic::unclosed_bracket(Span::new(0, 3), PairKind::Tortoise);
        assert!(format!("{diag}").contains("Tortoise"));
    }

    #[test]
    fn unmatched_close_display_mentions_kind() {
        let diag = Diagnostic::unmatched_close(Span::new(0, 3), PairKind::Quote);
        assert!(format!("{diag}").contains("Quote"));
    }

    #[test]
    fn internal_round_trips_check_and_span() {
        let diag = Diagnostic::internal(Span::new(2, 5), InternalCheckCode::RegistryOutOfOrder);
        let Diagnostic::Internal { check, span, .. } = diag else {
            panic!("expected Internal, got {diag:?}");
        };
        assert_eq!(check, InternalCheckCode::RegistryOutOfOrder);
        assert_eq!(span, Span::new(2, 5));
    }

    #[test]
    fn internal_classified_as_internal_source() {
        let diag = Diagnostic::internal(Span::new(0, 1), InternalCheckCode::UnregisteredSentinel);
        assert_eq!(diag.severity(), Severity::Error);
        assert_eq!(diag.source(), DiagnosticSource::Internal);
        assert_eq!(diag.code(), codes::UNREGISTERED_SENTINEL);
    }

    #[test]
    fn internal_display_mentions_code() {
        let diag =
            Diagnostic::internal(Span::new(0, 1), InternalCheckCode::ResidualAnnotationMarker);
        let rendered = format!("{diag}");
        assert!(
            rendered.contains(codes::RESIDUAL_ANNOTATION_MARKER),
            "Internal Display should print the code; got {rendered:?}"
        );
    }

    #[test]
    fn internal_check_code_as_code_round_trips_constants() {
        for kind in InternalCheckCode::ALL {
            let diag = Diagnostic::internal(Span::new(0, 0), kind);
            assert_eq!(
                diag.code(),
                kind.as_code(),
                "code() must agree with as_code() for {kind:?}"
            );
        }
    }

    /// Codes are stable identifiers — pin every constant so accidental
    /// rename of one breaks this test rather than silently breaking
    /// downstream tooling that grep-matches on the string.
    #[test]
    fn code_constants_are_stable() {
        assert_eq!(
            codes::SOURCE_CONTAINS_PUA,
            "aozora::lex::source_contains_pua"
        );
        assert_eq!(codes::UNCLOSED_BRACKET, "aozora::lex::unclosed_bracket");
        assert_eq!(codes::UNMATCHED_CLOSE, "aozora::lex::unmatched_close");
        assert_eq!(
            codes::ACCENT_DECOMPOSITION_APPLIED,
            "aozora::lex::accent_decomposition_applied"
        );
        assert_eq!(codes::UNRESOLVED_GAIJI, "aozora::lex::unresolved_gaiji");
        assert_eq!(
            codes::MISMATCHED_CONTAINER_CLOSE,
            "aozora::lex::mismatched_container_close"
        );
        assert_eq!(codes::EMPTY_RUBY_READING, "aozora::lex::empty_ruby_reading");
        assert_eq!(codes::NESTED_RUBY, "aozora::lex::nested_ruby");
        assert_eq!(
            codes::UNRECOGNISED_CONTAINER_DIRECTIVE,
            "aozora::lex::unrecognised_container_directive"
        );
        assert_eq!(
            codes::TCY_TARGET_NOT_FOUND,
            "aozora::lex::tcy_target_not_found"
        );
        assert_eq!(
            codes::BOUTEN_TARGET_AMBIGUOUS,
            "aozora::lex::bouten_target_ambiguous"
        );
        assert_eq!(
            codes::BREAK_IN_SINGLE_LINE_CONTAINER,
            "aozora::lex::break_in_single_line_container"
        );
        assert_eq!(
            codes::BRACKETED_KAERITEN_NO_PAIR,
            "aozora::lex::bracketed_kaeriten_no_pair"
        );
        assert_eq!(
            codes::KAERITEN_OUTSIDE_KANBUN,
            "aozora::lex::kaeriten_outside_kanbun"
        );
        assert_eq!(
            codes::MISMATCHED_BOUTEN_CONTAINER,
            "aozora::lex::mismatched_bouten_container"
        );
        assert_eq!(
            codes::RESIDUAL_ANNOTATION_MARKER,
            "aozora::lex::residual_annotation_marker"
        );
        assert_eq!(
            codes::UNREGISTERED_SENTINEL,
            "aozora::lex::unregistered_sentinel"
        );
        assert_eq!(
            codes::REGISTRY_OUT_OF_ORDER,
            "aozora::lex::registry_out_of_order"
        );
        assert_eq!(
            codes::REGISTRY_POSITION_MISMATCH,
            "aozora::lex::registry_position_mismatch"
        );
    }

    /// Severity / source axes are independent — pin the cross-product
    /// for the four production variants so a future variant addition
    /// has to think about both axes deliberately.
    #[test]
    fn severity_source_cross_product_is_pinned() {
        let pua = Diagnostic::source_contains_pua(Span::new(0, 3), '\u{E001}');
        assert_eq!(pua.severity(), Severity::Warning);
        assert_eq!(pua.source(), DiagnosticSource::Source);

        let unclosed = Diagnostic::unclosed_bracket(Span::new(0, 3), PairKind::Bracket);
        assert_eq!(unclosed.severity(), Severity::Error);
        assert_eq!(unclosed.source(), DiagnosticSource::Source);

        let unmatched = Diagnostic::unmatched_close(Span::new(0, 3), PairKind::Bracket);
        assert_eq!(unmatched.severity(), Severity::Error);
        assert_eq!(unmatched.source(), DiagnosticSource::Source);

        let accent = Diagnostic::accent_decomposition_applied(Span::new(0, 9));
        assert_eq!(accent.severity(), Severity::Note);
        assert_eq!(accent.source(), DiagnosticSource::Source);
        assert_eq!(accent.code(), codes::ACCENT_DECOMPOSITION_APPLIED);

        let gaiji = Diagnostic::unresolved_gaiji(Span::new(0, 12));
        assert_eq!(gaiji.severity(), Severity::Warning);
        assert_eq!(gaiji.source(), DiagnosticSource::Source);
        assert_eq!(gaiji.code(), codes::UNRESOLVED_GAIJI);

        let mismatch =
            Diagnostic::mismatched_container_close(Span::new(0, 6), "indent", "align-end");
        assert_eq!(mismatch.severity(), Severity::Error);
        assert_eq!(mismatch.source(), DiagnosticSource::Source);
        assert_eq!(mismatch.code(), codes::MISMATCHED_CONTAINER_CLOSE);

        let empty_ruby = Diagnostic::empty_ruby_reading(Span::new(0, 15));
        assert_eq!(empty_ruby.severity(), Severity::Error);
        assert_eq!(empty_ruby.source(), DiagnosticSource::Source);
        assert_eq!(empty_ruby.code(), codes::EMPTY_RUBY_READING);

        let nested_ruby = Diagnostic::nested_ruby(Span::new(6, 9));
        assert_eq!(nested_ruby.severity(), Severity::Error);
        assert_eq!(nested_ruby.source(), DiagnosticSource::Source);
        assert_eq!(nested_ruby.code(), codes::NESTED_RUBY);

        let unrec = Diagnostic::unrecognised_container_directive(Span::new(0, 18));
        assert_eq!(unrec.severity(), Severity::Warning);
        assert_eq!(unrec.source(), DiagnosticSource::Source);
        assert_eq!(unrec.code(), codes::UNRECOGNISED_CONTAINER_DIRECTIVE);

        let tcy = Diagnostic::tcy_target_not_found(Span::new(0, 18));
        assert_eq!(tcy.severity(), Severity::Warning);
        assert_eq!(tcy.source(), DiagnosticSource::Source);
        assert_eq!(tcy.code(), codes::TCY_TARGET_NOT_FOUND);

        let bouten = Diagnostic::bouten_target_ambiguous(Span::new(0, 18));
        assert_eq!(bouten.severity(), Severity::Warning);
        assert_eq!(bouten.source(), DiagnosticSource::Source);
        assert_eq!(bouten.code(), codes::BOUTEN_TARGET_AMBIGUOUS);

        let break_slc = Diagnostic::break_in_single_line_container(Span::new(0, 18), "align-end");
        assert_eq!(break_slc.severity(), Severity::Warning);
        assert_eq!(break_slc.source(), DiagnosticSource::Source);
        assert_eq!(break_slc.code(), codes::BREAK_IN_SINGLE_LINE_CONTAINER);

        let kaeriten_pair = Diagnostic::bracketed_kaeriten_no_pair(Span::new(0, 9));
        assert_eq!(kaeriten_pair.severity(), Severity::Error);
        assert_eq!(kaeriten_pair.source(), DiagnosticSource::Source);
        assert_eq!(kaeriten_pair.code(), codes::BRACKETED_KAERITEN_NO_PAIR);

        let kaeriten_kanbun = Diagnostic::kaeriten_outside_kanbun(Span::new(0, 9));
        assert_eq!(kaeriten_kanbun.severity(), Severity::Warning);
        assert_eq!(kaeriten_kanbun.source(), DiagnosticSource::Source);
        assert_eq!(kaeriten_kanbun.code(), codes::KAERITEN_OUTSIDE_KANBUN);

        let bouten_mismatch =
            Diagnostic::mismatched_bouten_container(Span::new(0, 12), "傍点", "傍線");
        assert_eq!(bouten_mismatch.severity(), Severity::Error);
        assert_eq!(bouten_mismatch.source(), DiagnosticSource::Source);
        assert_eq!(bouten_mismatch.code(), codes::MISMATCHED_BOUTEN_CONTAINER);

        let internal = Diagnostic::internal(Span::new(0, 3), InternalCheckCode::RegistryOutOfOrder);
        assert_eq!(internal.severity(), Severity::Error);
        assert_eq!(internal.source(), DiagnosticSource::Internal);
    }

    /// Every catalogued code resolves to a representative instance with
    /// non-empty help and an https URL — guards `explain` against a code
    /// that has no sample (and pins the catalogue length).
    #[test]
    fn explain_covers_every_catalogued_code() {
        assert_eq!(
            Diagnostic::ALL_CODES.len(),
            19,
            "ALL_CODES must list every code code() can return"
        );
        for &code in &Diagnostic::ALL_CODES {
            let info = Diagnostic::explain(code)
                .unwrap_or_else(|| panic!("catalogued code {code} is not explainable"));
            assert_eq!(
                info.code, code,
                "explain echoed a different code for {code}"
            );
            assert!(!info.help.trim().is_empty(), "{code}: empty help text");
            assert!(
                info.url
                    .as_deref()
                    .is_some_and(|u| u.starts_with("https://")),
                "{code}: missing or non-https url"
            );
        }
    }

    #[test]
    fn explain_rejects_unknown_and_unprefixed_codes() {
        // explain wants the full code; the CLI expands short forms.
        assert!(Diagnostic::explain(codes::UNCLOSED_BRACKET).is_some());
        assert!(Diagnostic::explain("unclosed_bracket").is_none());
        assert!(Diagnostic::explain("ruby").is_none());
        assert!(Diagnostic::explain("aozora::lex::does_not_exist").is_none());
    }

    #[test]
    fn explain_internal_codes_share_help_but_keep_distinct_codes() {
        let resid = Diagnostic::explain(codes::RESIDUAL_ANNOTATION_MARKER).unwrap();
        let unreg = Diagnostic::explain(codes::UNREGISTERED_SENTINEL).unwrap();
        assert_eq!(resid.source, DiagnosticSource::Internal);
        assert_eq!(resid.code, codes::RESIDUAL_ANNOTATION_MARKER);
        assert_eq!(unreg.code, codes::UNREGISTERED_SENTINEL);
        // All four internal checks are one Diagnostic::Internal variant,
        // so they share the umbrella help/url.
        assert_eq!(resid.help, unreg.help);
        assert_eq!(resid.url, unreg.url);
    }
}
