use std::borrow::Cow;

use miette::Diagnostic as MietteDiagnostic;
use thiserror::Error;

use crate::spec::PairKind;
use crate::spec::Span;

pub(crate) mod codes {
    pub(crate) const SOURCE_CONTAINS_PUA: &str = "aozora::lex::source_contains_pua";

    pub(crate) const UNCLOSED_BRACKET: &str = "aozora::lex::unclosed_bracket";

    pub(crate) const UNMATCHED_CLOSE: &str = "aozora::lex::unmatched_close";

    pub(crate) const ACCENT_DECOMPOSITION_APPLIED: &str =
        "aozora::lex::accent_decomposition_applied";

    pub(crate) const UNRESOLVED_GAIJI: &str = "aozora::lex::unresolved_gaiji";

    pub(crate) const MISMATCHED_CONTAINER_CLOSE: &str = "aozora::lex::mismatched_container_close";

    pub(crate) const EMPTY_RUBY_READING: &str = "aozora::lex::empty_ruby_reading";

    pub(crate) const NESTED_RUBY: &str = "aozora::lex::nested_ruby";

    pub(crate) const UNRECOGNISED_CONTAINER_DIRECTIVE: &str =
        "aozora::lex::unrecognised_container_directive";

    pub(crate) const TCY_TARGET_NOT_FOUND: &str = "aozora::lex::tcy_target_not_found";

    pub(crate) const BOUTEN_TARGET_AMBIGUOUS: &str = "aozora::lex::bouten_target_ambiguous";

    pub(crate) const FORWARD_REFERENT_NOT_STYLABLE: &str =
        "aozora::lex::forward_referent_not_stylable";

    pub(crate) const BREAK_IN_SINGLE_LINE_CONTAINER: &str =
        "aozora::lex::break_in_single_line_container";

    pub(crate) const BRACKETED_KAERITEN_NO_PAIR: &str = "aozora::lex::bracketed_kaeriten_no_pair";

    pub(crate) const KAERITEN_OUTSIDE_KANBUN: &str = "aozora::lex::kaeriten_outside_kanbun";

    pub(crate) const MISMATCHED_BOUTEN_CONTAINER: &str = "aozora::lex::mismatched_bouten_container";

    pub(crate) const NON_CANONICAL_DIRECTIVE: &str = "aozora::lint::non_canonical_directive";

    pub(crate) const LINT_NAMESPACE: &str = "aozora::lint::";

    pub(crate) const RESIDUAL_ANNOTATION_MARKER: &str = "aozora::lex::residual_annotation_marker";

    pub(crate) const UNREGISTERED_SENTINEL: &str = "aozora::lex::unregistered_sentinel";

    pub(crate) const REGISTRY_OUT_OF_ORDER: &str = "aozora::lex::registry_out_of_order";

    pub(crate) const REGISTRY_POSITION_MISMATCH: &str = "aozora::lex::registry_position_mismatch";
}

/// Severity of a [`Diagnostic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Severity {
    /// Error that may invalidate the parse result.
    Error,
    /// Recoverable parsing issue.
    Warning,
    /// Informational observation.
    Note,
}

impl Severity {
    /// Every variant in declaration order.
    pub const ALL: [Self; 3] = [Self::Error, Self::Warning, Self::Note];

    /// Stable lowercase wire-format identifier.
    #[must_use]
    pub const fn as_json_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Note => "note",
        }
    }
}

/// Whether a diagnostic concerns user input or a library defect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DiagnosticSource {
    /// Issue traces back to the user-provided source text.
    Source,
    /// Library defect encountered while parsing.
    Internal,
}

impl DiagnosticSource {
    /// Every variant in declaration order.
    pub const ALL: [Self; 2] = [Self::Source, Self::Internal];

    /// Stable lowercase wire-format identifier.
    #[must_use]
    pub const fn as_json_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Internal => "internal",
        }
    }
}

/// Identifier for the check reported by [`Diagnostic::Internal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum InternalCheckCode {
    /// An annotation opener `［＃` remains in normalized text after classification.
    ResidualAnnotationMarker,
    /// A reserved sentinel has no corresponding registry entry.
    UnregisteredSentinel,
    /// Registry positions repeat or appear out of order.
    RegistryOutOfOrder,
    /// A registry entry points to the wrong sentinel kind.
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

    /// Stable `aozora::lex::*` identifier for this check.
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

/// Observation emitted by any lexer stage.
#[derive(Debug, Clone, Error, MietteDiagnostic)]
#[non_exhaustive]
pub enum Diagnostic {
    /// Source contains a codepoint reserved for parser placeholders.
    #[error(
        "source contains lexer PUA sentinel codepoint U+{:04X}",
        u32::from(*.codepoint)
    )]
    #[diagnostic(
        code("aozora::lex::source_contains_pua"),
        url("https://p4suta.github.io/aozora-notation-spec/diagnostics.html#source-contains-pua"),
        severity(Warning),
        help(
            "the lexer reserves U+E001..U+E004 as inline/block markers; \
             a source-side occurrence will confuse the placeholder registry"
        )
    )]
    SourceContainsPua {
        /// Label location for diagnostic rendering.
        #[label("here")]
        at: miette::SourceSpan,
        /// The reserved codepoint found in the source.
        codepoint: char,
        /// Byte span for consumers that don't use [`miette::SourceSpan`].
        span: Span,
    },

    /// Opening delimiter without a matching closer.
    #[error(
        "unclosed Aozora {}…{} delimiter",
        .kind.open_str(),
        .kind.close_str()
    )]
    #[diagnostic(
        code("aozora::lex::unclosed_bracket"),
        url("https://p4suta.github.io/aozora-notation-spec/diagnostics.html#unclosed-bracket"),
        help(
            "the opener has no matching close delimiter — either the close \
             was omitted or an earlier close matched a nested opener"
        )
    )]
    UnclosedBracket {
        /// Label location for diagnostic rendering.
        #[label("opened here")]
        at: miette::SourceSpan,
        /// Delimiter family of the unmatched opener.
        kind: PairKind,
        /// Byte-range of the unmatched *open* delimiter in the diagnostic's source buffer.
        span: Span,
    },

    /// Closing delimiter without a compatible opener on the pairing stack.
    #[error("unmatched Aozora {} close delimiter", .kind.close_str())]
    #[diagnostic(
        code("aozora::lex::unmatched_close"),
        url("https://p4suta.github.io/aozora-notation-spec/diagnostics.html#unmatched-close"),
        help(
            "no matching open on the pairing stack — either the open was \
             omitted or an inner unmatched close consumed it"
        )
    )]
    UnmatchedClose {
        /// Label location for diagnostic rendering.
        #[label("close here")]
        at: miette::SourceSpan,
        /// Delimiter family of the stray closer.
        kind: PairKind,
        /// Byte-range of the stray *close* delimiter.
        span: Span,
    },

    /// Sanitization composed an accent digraph under ADR-0003.
    #[error("accent digraph decomposed in sanitize stage")]
    #[diagnostic(
        code("aozora::lex::accent_decomposition_applied"),
        url(
            "https://p4suta.github.io/aozora-notation-spec/diagnostics.html#accent-decomposition-applied"
        ),
        severity(Advice),
        help(
            "the `〔…〕` accent span was rewritten to its combined Unicode form; \
             this is expected and round-trips back to the source on serialize"
        )
    )]
    AccentDecompositionApplied {
        /// Label location for diagnostic rendering.
        #[label("decomposed here")]
        at: miette::SourceSpan,
        /// Byte span of the accent notation in the diagnostic's source buffer.
        span: Span,
    },

    /// A gaiji reference has no Unicode resolution, so rendering uses its description.
    #[error("gaiji reference resolved to neither Unicode nor JIS X 0213")]
    #[diagnostic(
        code("aozora::lex::unresolved_gaiji"),
        url("https://p4suta.github.io/aozora-notation-spec/diagnostics.html#unresolved-gaiji"),
        severity(Warning),
        help(
            "no JIS X 0213 men-ku-ten or U+XXXX reference matched and the \
             description is not a single resolvable character — the glyph \
             renders as its description text only"
        )
    )]
    UnresolvedGaiji {
        /// Label location for diagnostic rendering.
        #[label("unresolved gaiji")]
        at: miette::SourceSpan,
        /// Byte-range of the `※［＃…］` reference in the diagnostic's source buffer.
        span: Span,
    },

    /// A container close names a different family from its opener.
    #[error("container opened as `{open_kind}` closed by a `{close_kind}` closer")]
    #[diagnostic(
        code("aozora::lex::mismatched_container_close"),
        url(
            "https://p4suta.github.io/aozora-notation-spec/diagnostics.html#mismatched-container-close"
        ),
        help(
            "the close directive names a different container family than the \
             open — pair `ここから字下げ` with `ここで字下げ終わり`, `ここから地付き` \
             with `ここで地付き終わり`, etc."
        )
    )]
    MismatchedContainerClose {
        /// Label location for diagnostic rendering.
        #[label("mismatched close")]
        at: miette::SourceSpan,
        /// Container family of the *open* marker on the pairing stack.
        open_kind: &'static str,
        /// Container family named by the *close* marker.
        close_kind: &'static str,
        /// Byte-range of the close marker in the diagnostic's source buffer.
        span: Span,
    },

    /// A verified non-canonical directive spelling with a suggested correction.
    // Notation-hygiene policy belongs to this library, so its explanation points to ADR-0022 rather than the notation specification.
    #[error("non-canonical directive; the canonical form is `{canonical}`")]
    #[diagnostic(
        code("aozora::lint::non_canonical_directive"),
        url(
            "https://github.com/P4suta/aozora/blob/main/docs/adr/\
             0022-notation-hygiene-layer-roles.md"
        ),
        severity(Warning),
        help(
            "this ［＃…］ body matches a recognized directive spelled \
             non-canonically, so it was kept as an Unknown directive; rewrite \
             it to the canonical form (`aozora fmt --fix`)."
        )
    )]
    NonCanonicalDirective {
        /// Label location for diagnostic rendering.
        #[label("non-canonical directive")]
        at: miette::SourceSpan,
        /// The suggested canonical spelling.
        canonical: Cow<'static, str>,
        /// Byte-range of the directive in the diagnostic's source buffer.
        span: Span,
    },

    /// An explicit ruby base has an empty reading and degrades to plain text.
    #[error("ruby base given but reading is empty")]
    #[diagnostic(
        code("aozora::lex::empty_ruby_reading"),
        url("https://p4suta.github.io/aozora-notation-spec/diagnostics.html#empty-ruby-reading"),
        help(
            "the `《…》` reading after the `｜` base is empty — supply a reading \
             or remove the `｜…《》` markers to keep the base as plain text"
        )
    )]
    EmptyRubyReading {
        /// Label location for diagnostic rendering.
        #[label("empty reading")]
        at: miette::SourceSpan,
        /// Byte-range of the `｜base《》` construct in the diagnostic's source buffer.
        span: Span,
    },

    /// A ruby reading contains a nested ruby opener.
    #[error("ruby reading contains a nested ruby")]
    #[diagnostic(
        code("aozora::lex::nested_ruby"),
        url("https://p4suta.github.io/aozora-notation-spec/diagnostics.html#nested-ruby"),
        help(
            "ruby cannot nest — close the outer reading before the inner `《`, \
             or remove the inner `《…》`"
        )
    )]
    NestedRuby {
        /// Label location for diagnostic rendering.
        #[label("nested ruby opens here")]
        at: miette::SourceSpan,
        /// Byte-range of the inner `《` opener in the diagnostic's source buffer.
        span: Span,
    },

    /// An unrecognized container opener retained as an unknown directive.
    #[error("unrecognised container directive")]
    #[diagnostic(
        code("aozora::lex::unrecognised_container_directive"),
        url(
            "https://p4suta.github.io/aozora-notation-spec/diagnostics.html#unrecognised-container-directive"
        ),
        severity(Warning),
        help(
            "`［＃ここから…］` must name a known container — `字下げ`, `地付き`, \
             `地から N 字上げ`; this directive was kept as a plain annotation"
        )
    )]
    UnrecognisedContainerDirective {
        /// Label location for diagnostic rendering.
        #[label("unrecognised directive")]
        at: miette::SourceSpan,
        /// Byte-range of the `［＃ここから…］` directive in the diagnostic's source buffer.
        span: Span,
    },

    /// A `縦中横` directive names a target absent from preceding text.
    #[error("縦中横 target not found in the preceding text")]
    #[diagnostic(
        code("aozora::lex::tcy_target_not_found"),
        url("https://p4suta.github.io/aozora-notation-spec/diagnostics.html#tcy-target-not-found"),
        severity(Warning),
        help(
            "the quoted 縦中横 target must occur earlier in the line — check the \
             spelling, or place the `［＃「X」は縦中横］` after the run it styles"
        )
    )]
    TcyTargetNotFound {
        /// Label location for diagnostic rendering.
        #[label("target has no referent")]
        at: miette::SourceSpan,
        /// Byte-range of the `［＃「X」は縦中横］` directive in the diagnostic's source buffer.
        span: Span,
    },

    /// A bouten directive names a target that appears more than once in preceding text.
    #[error("ambiguous bouten target: more than one candidate run precedes it")]
    #[diagnostic(
        code("aozora::lex::bouten_target_ambiguous"),
        url(
            "https://p4suta.github.io/aozora-notation-spec/diagnostics.html#bouten-target-ambiguous"
        ),
        severity(Warning),
        help(
            "the quoted target appears more than once before the `［＃…］` — the \
             styled run may not be the intended one; reword so the target is unique"
        )
    )]
    BoutenTargetAmbiguous {
        /// Label location for diagnostic rendering.
        #[label("ambiguous target")]
        at: miette::SourceSpan,
        /// Byte-range of the `［＃「X」に傍点］` directive in the diagnostic's source buffer.
        span: Span,
    },

    /// A forward directive has no supported way to style its preceding referent.
    #[error("forward-reference target found but not stylable in place")]
    #[diagnostic(
        code("aozora::lex::forward_referent_not_stylable"),
        url(
            "https://p4suta.github.io/aozora-notation-spec/diagnostics.html#forward-referent-not-stylable"
        ),
        severity(Warning),
        help(
            "the quoted target is a ruby base, on an earlier line, inside another \
             construct, or one of several targets — move the `［＃…］` next to a \
             plain occurrence of the target so the styling can be applied"
        )
    )]
    ForwardReferentNotStylable {
        /// Label location for diagnostic rendering.
        #[label("target not stylable in place")]
        at: miette::SourceSpan,
        /// Byte-range of the `［＃「X」は…］` directive in the diagnostic's source buffer.
        span: Span,
    },

    /// A page or section break drops a single-line container's effect.
    #[error("page/section break inside a single-line `{container}` container")]
    #[diagnostic(
        code("aozora::lex::break_in_single_line_container"),
        url(
            "https://p4suta.github.io/aozora-notation-spec/diagnostics.html#break-in-single-line-container"
        ),
        severity(Warning),
        help(
            "a single-line container governs only the rest of its line — move \
             the break off the line, or use the paired `［＃ここから…］` … \
             `［＃ここで…終わり］` block form that persists across breaks"
        )
    )]
    BreakInSingleLineContainer {
        /// Label location for diagnostic rendering.
        #[label("break drops the container")]
        at: miette::SourceSpan,
        /// Family tag of the dropped container, such as `indent`, `align-end`, or `warichu`.
        container: &'static str,
        /// Byte-range of the break directive in the diagnostic's source buffer.
        span: Span,
    },

    /// A bracketed kaeriten has no family base anywhere in the document.
    #[error("bracketed kaeriten has no matching base mark in the document")]
    #[diagnostic(
        code("aozora::lex::bracketed_kaeriten_no_pair"),
        url(
            "https://p4suta.github.io/aozora-notation-spec/diagnostics.html#bracketed-kaeriten-no-pair"
        ),
        help(
            "a return mark needs its family base somewhere in the document — \
             a `［＃二］`/`［＃三］` needs a `［＃一］`, a `［＃下］`/`［＃中］` needs \
             a `［＃上］`, a `［＃乙］`… needs a `［＃甲］`"
        )
    )]
    BracketedKaeritenNoPair {
        /// Label location for diagnostic rendering.
        #[label("unpaired kaeriten")]
        at: miette::SourceSpan,
        /// Byte-range of the `［＃…］` kaeriten directive in the diagnostic's source buffer.
        span: Span,
    },

    /// An isolated kaeriten appears in ordinary kana prose.
    #[error("kaeriten outside a 漢文-like context")]
    #[diagnostic(
        code("aozora::lex::kaeriten_outside_kanbun"),
        url(
            "https://p4suta.github.io/aozora-notation-spec/diagnostics.html#kaeriten-outside-kanbun"
        ),
        severity(Warning),
        help(
            "this is the only kaeriten in the document and its surroundings \
             look like ordinary prose — check it is a genuine 返り点 and not a \
             stray `［＃…］` annotation"
        )
    )]
    KaeritenOutsideKanbun {
        /// Label location for diagnostic rendering.
        #[label("isolated kaeriten")]
        at: miette::SourceSpan,
        /// Byte-range of the `［＃…］` kaeriten directive in the diagnostic's source buffer.
        span: Span,
    },

    /// A bouten range closes with a different family, recovering with the opener's style.
    #[error("傍点 range opened as `{open_family}` closed by a `{close_family}` closer")]
    #[diagnostic(
        code("aozora::lex::mismatched_bouten_container"),
        url(
            "https://p4suta.github.io/aozora-notation-spec/diagnostics.html#mismatched-bouten-container"
        ),
        help(
            "close a 傍点 range with `［＃傍点終わり］` (any 点 variant) and a 傍線 \
             range with `［＃傍線終わり］` (any 線 variant) — match the opener's \
             family"
        )
    )]
    MismatchedBoutenContainer {
        /// Label location for diagnostic rendering.
        #[label("mismatched close")]
        at: miette::SourceSpan,
        /// Family of the opening marker, either `傍点` or `傍線`.
        open_family: &'static str,
        /// Family named by the *close* marker.
        close_family: &'static str,
        /// Byte-range of the close marker in the diagnostic's source buffer.
        span: Span,
    },

    /// A library check failed with the supplied [`InternalCheckCode`].
    #[error("internal aozora pipeline check failed: {}", check.as_code())]
    #[diagnostic(
        code("aozora::internal"),
        url("https://github.com/P4suta/aozora/issues"),
        help(
            "this is a pipeline-internal sanity check; appearance \
             indicates a bug in aozora — please report at \
             https://github.com/P4suta/aozora/issues with the source \
             that triggered it"
        )
    )]
    Internal {
        /// Label location for diagnostic rendering.
        #[label("at this position")]
        at: miette::SourceSpan,
        /// The check that failed.
        check: InternalCheckCode,
        /// Byte-range covering the violation site.
        span: Span,
    },
}

/// Metadata and examples returned by [`Diagnostic::explain`].
///
/// Resolve localized text with `code` and `body_args`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DiagnosticInfo {
    /// Stable diagnostic code.
    pub code: &'static str,
    /// Severity routing axis.
    pub severity: Severity,
    /// Whether the diagnostic concerns user input or a library defect.
    pub source: DiagnosticSource,
    /// Suggested action from the diagnostic's help text.
    pub help: String,
    /// Documentation URL for the code, when the variant carries one.
    pub url: Option<String>,
    /// A minimal reproduction of the condition in Aozora notation.
    pub repro: &'static str,
    /// The corrected form corresponding to [`Self::repro`].
    pub fixed: &'static str,
    /// Arguments interpolated into this diagnostic's localized body message.
    pub body_args: Vec<(&'static str, String)>,
}

struct DiagnosticDoc {
    code: &'static str,
    repro: &'static str,
    fixed: &'static str,
}

const DOCS: [DiagnosticDoc; 21] = [
    DiagnosticDoc {
        code: codes::SOURCE_CONTAINS_PUA,
        repro: "（不可視の U+E001 などが混入した行）",
        fixed: "（その 1 文字を削除した行）",
    },
    DiagnosticDoc {
        code: codes::UNCLOSED_BRACKET,
        repro: "本文［＃改ページ",
        fixed: "本文［＃改ページ］",
    },
    DiagnosticDoc {
        code: codes::UNMATCHED_CLOSE,
        repro: "本文 ］",
        fixed: "本文",
    },
    DiagnosticDoc {
        code: codes::ACCENT_DECOMPOSITION_APPLIED,
        repro: "Cre〔e'〕vez",
        fixed: "（修正不要。保存時に元の〔…〕へ復元されます）",
    },
    DiagnosticDoc {
        code: codes::UNRESOLVED_GAIJI,
        repro: "※［＃「ある字」の説明］",
        fixed: "※［＃「ある字」、第3水準1-15-23］（面区点か U+ を補う）",
    },
    DiagnosticDoc {
        code: codes::MISMATCHED_CONTAINER_CLOSE,
        repro: "［＃ここから2字下げ］\n本文\n［＃ここで地付き終わり］",
        fixed: "［＃ここから2字下げ］\n本文\n［＃ここで字下げ終わり］",
    },
    DiagnosticDoc {
        code: codes::EMPTY_RUBY_READING,
        repro: "｜青空《》",
        fixed: "｜青空《あおぞら》",
    },
    DiagnosticDoc {
        code: codes::NESTED_RUBY,
        repro: "｜漢《字《かん》》",
        fixed: "｜漢字《かんじ》",
    },
    DiagnosticDoc {
        code: codes::UNRECOGNISED_CONTAINER_DIRECTIVE,
        repro: "［＃ここから謎レイアウト］",
        fixed: "［＃ここから2字下げ］（既知のコンテナ名にする）",
    },
    DiagnosticDoc {
        code: codes::TCY_TARGET_NOT_FOUND,
        repro: "本文［＃「25」は縦中横］",
        fixed: "25［＃「25」は縦中横］（対象を注記より前に置く）",
    },
    DiagnosticDoc {
        code: codes::BOUTEN_TARGET_AMBIGUOUS,
        repro: "花と花［＃「花」に傍点］",
        fixed: "赤い花と白い花［＃「白い花」に傍点］（対象を一意にする）",
    },
    DiagnosticDoc {
        code: codes::FORWARD_REFERENT_NOT_STYLABLE,
        repro: "｜我《われ》は […]［＃「我」に傍点］",
        fixed: "我［＃「我」に傍点］（プレーンな出現の隣に置く）",
    },
    DiagnosticDoc {
        code: codes::BREAK_IN_SINGLE_LINE_CONTAINER,
        repro: "［＃地付き］本文［＃改ページ］",
        fixed: "本文［＃改ページ］\n［＃地付き］本文（改ページを行外に出す）",
    },
    DiagnosticDoc {
        code: codes::BRACKETED_KAERITEN_NO_PAIR,
        repro: "学而時習之［＃二］",
        fixed: "学而［＃一］時習之［＃二］（家系の基点［＃一］を置く）",
    },
    DiagnosticDoc {
        code: codes::KAERITEN_OUTSIDE_KANBUN,
        repro: "ふつうの文章です［＃レ］",
        fixed: "（返り点でないなら注記を削除、本物の漢文文脈で使う）",
    },
    DiagnosticDoc {
        code: codes::MISMATCHED_BOUTEN_CONTAINER,
        repro: "［＃傍点］本文［＃傍線終わり］",
        fixed: "［＃傍点］本文［＃傍点終わり］",
    },
    DiagnosticDoc {
        code: codes::NON_CANONICAL_DIRECTIVE,
        repro: "本文［＃字下げ終わり］",
        fixed: "本文［＃ここで字下げ終わり］",
    },
    DiagnosticDoc {
        code: codes::RESIDUAL_ANNOTATION_MARKER,
        repro: "本文［＃なぞの注記］",
        fixed: "本文［＃改ページ］",
    },
    DiagnosticDoc {
        code: codes::UNREGISTERED_SENTINEL,
        repro: "（通常のソースからは発生しません）",
        fixed: "（パイプラインのバグ。手元の修正では直せません）",
    },
    DiagnosticDoc {
        code: codes::REGISTRY_OUT_OF_ORDER,
        repro: "（通常のソースからは発生しません）",
        fixed: "（パイプラインのバグ。手元の修正では直せません）",
    },
    DiagnosticDoc {
        code: codes::REGISTRY_POSITION_MISMATCH,
        repro: "（通常のソースからは発生しません）",
        fixed: "（パイプラインのバグ。手元の修正では直せません）",
    },
];

const fn pair_example(kind: PairKind) -> &'static str {
    match kind {
        PairKind::Ruby => "｜青空《あおぞら》",
        PairKind::AngleQuote => "≪重要≫",
        PairKind::Tortoise => "〔Crevez chiens〕",
        PairKind::Quote => "［＃「青空」に傍点］",
        PairKind::Bracket => "［＃改ページ］",
    }
}

fn doc_for(code: &str) -> Option<&'static DiagnosticDoc> {
    DOCS.iter().find(|d| d.code == code)
}

#[expect(
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

    /// Constructor for [`Diagnostic::NonCanonicalDirective`].
    #[must_use]
    pub fn non_canonical_directive(at: Span, canonical: impl Into<Cow<'static, str>>) -> Self {
        let (offset, length) = span_to_miette_parts(at);
        Self::NonCanonicalDirective {
            at: miette::SourceSpan::new(offset.into(), length),
            canonical: canonical.into(),
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

    /// Create a container mismatch diagnostic from stable family tags.
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

    /// Constructor for [`Diagnostic::ForwardReferentNotStylable`].
    #[must_use]
    pub fn forward_referent_not_stylable(at: Span) -> Self {
        let (offset, length) = span_to_miette_parts(at);
        Self::ForwardReferentNotStylable {
            at: miette::SourceSpan::new(offset.into(), length),
            span: at,
        }
    }

    /// Create a break diagnostic with the dropped container's stable family tag.
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

    /// Create a bouten mismatch diagnostic from dot and line family tags.
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

    /// Create a diagnostic for a typed internal check.
    #[must_use]
    pub fn internal(at: Span, check: InternalCheckCode) -> Self {
        let (offset, length) = span_to_miette_parts(at);
        Self::Internal {
            at: miette::SourceSpan::new(offset.into(), length),
            check,
            span: at,
        }
    }

    /// Severity of this diagnostic.
    #[must_use]
    pub fn severity(&self) -> Severity {
        match self {
            Self::SourceContainsPua { .. }
            | Self::UnresolvedGaiji { .. }
            | Self::UnrecognisedContainerDirective { .. }
            | Self::TcyTargetNotFound { .. }
            | Self::BoutenTargetAmbiguous { .. }
            | Self::ForwardReferentNotStylable { .. }
            | Self::BreakInSingleLineContainer { .. }
            | Self::KaeritenOutsideKanbun { .. }
            | Self::NonCanonicalDirective { .. } => Severity::Warning,
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

    /// Whether this diagnostic concerns user input or a library defect.
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
            | Self::ForwardReferentNotStylable { .. }
            | Self::BreakInSingleLineContainer { .. }
            | Self::BracketedKaeritenNoPair { .. }
            | Self::KaeritenOutsideKanbun { .. }
            | Self::MismatchedBoutenContainer { .. }
            | Self::NonCanonicalDirective { .. } => DiagnosticSource::Source,
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
            | Self::ForwardReferentNotStylable { span, .. }
            | Self::BreakInSingleLineContainer { span, .. }
            | Self::BracketedKaeritenNoPair { span, .. }
            | Self::KaeritenOutsideKanbun { span, .. }
            | Self::MismatchedBoutenContainer { span, .. }
            | Self::NonCanonicalDirective { span, .. }
            | Self::Internal { span, .. } => *span,
        }
    }

    /// Shift both the byte span and rendering caret by `by` bytes, saturating at the `u32` bounds.
    #[must_use]
    pub fn shifted(mut self, by: i64) -> Self {
        let (at, span): (&mut miette::SourceSpan, &mut Span) = match &mut self {
            Self::SourceContainsPua { at, span, .. }
            | Self::UnclosedBracket { at, span, .. }
            | Self::UnmatchedClose { at, span, .. }
            | Self::AccentDecompositionApplied { at, span, .. }
            | Self::UnresolvedGaiji { at, span, .. }
            | Self::MismatchedContainerClose { at, span, .. }
            | Self::EmptyRubyReading { at, span, .. }
            | Self::NestedRuby { at, span, .. }
            | Self::UnrecognisedContainerDirective { at, span, .. }
            | Self::TcyTargetNotFound { at, span, .. }
            | Self::BoutenTargetAmbiguous { at, span, .. }
            | Self::ForwardReferentNotStylable { at, span, .. }
            | Self::BreakInSingleLineContainer { at, span, .. }
            | Self::BracketedKaeritenNoPair { at, span, .. }
            | Self::KaeritenOutsideKanbun { at, span, .. }
            | Self::MismatchedBoutenContainer { at, span, .. }
            | Self::NonCanonicalDirective { at, span, .. }
            | Self::Internal { at, span, .. } => (at, span),
        };
        *span = span.shifted(by);
        let (offset, length) = span_to_miette_parts(*span);
        *at = miette::SourceSpan::new(offset.into(), length);
        self
    }

    #[must_use]
    pub(crate) fn with_span(mut self, mapped: Span) -> Self {
        let (at, span): (&mut miette::SourceSpan, &mut Span) = match &mut self {
            Self::SourceContainsPua { at, span, .. }
            | Self::UnclosedBracket { at, span, .. }
            | Self::UnmatchedClose { at, span, .. }
            | Self::AccentDecompositionApplied { at, span, .. }
            | Self::UnresolvedGaiji { at, span, .. }
            | Self::MismatchedContainerClose { at, span, .. }
            | Self::EmptyRubyReading { at, span, .. }
            | Self::NestedRuby { at, span, .. }
            | Self::UnrecognisedContainerDirective { at, span, .. }
            | Self::TcyTargetNotFound { at, span, .. }
            | Self::BoutenTargetAmbiguous { at, span, .. }
            | Self::ForwardReferentNotStylable { at, span, .. }
            | Self::BreakInSingleLineContainer { at, span, .. }
            | Self::BracketedKaeritenNoPair { at, span, .. }
            | Self::KaeritenOutsideKanbun { at, span, .. }
            | Self::MismatchedBoutenContainer { at, span, .. }
            | Self::NonCanonicalDirective { at, span, .. }
            | Self::Internal { at, span, .. } => (at, span),
        };
        *span = mapped;
        let (offset, length) = span_to_miette_parts(mapped);
        *at = miette::SourceSpan::new(offset.into(), length);
        self
    }

    /// Stable string identifier for this diagnostic.
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
            Self::ForwardReferentNotStylable { .. } => codes::FORWARD_REFERENT_NOT_STYLABLE,
            Self::BreakInSingleLineContainer { .. } => codes::BREAK_IN_SINGLE_LINE_CONTAINER,
            Self::BracketedKaeritenNoPair { .. } => codes::BRACKETED_KAERITEN_NO_PAIR,
            Self::KaeritenOutsideKanbun { .. } => codes::KAERITEN_OUTSIDE_KANBUN,
            Self::MismatchedBoutenContainer { .. } => codes::MISMATCHED_BOUTEN_CONTAINER,
            Self::NonCanonicalDirective { .. } => codes::NON_CANONICAL_DIRECTIVE,
            Self::Internal { check, .. } => check.as_code(),
        }
    }

    /// Whether this is an advisory notation-hygiene lint.
    #[must_use]
    pub fn is_lint(&self) -> bool {
        self.code().starts_with(codes::LINT_NAMESPACE)
    }

    /// Named arguments for this diagnostic's localized message.
    ///
    /// Returns an empty vector when the message has no placeholders.
    #[must_use]
    pub fn body_args(&self) -> Vec<(&'static str, Cow<'static, str>)> {
        match self {
            Self::SourceContainsPua { codepoint, .. } => vec![
                ("codepoint", format!("{:04X}", *codepoint as u32).into()),
                ("char", codepoint.to_string().into()),
            ],
            Self::UnclosedBracket { kind, .. } => vec![
                ("open", Cow::Borrowed(kind.open_str())),
                ("close", Cow::Borrowed(kind.close_str())),
                ("example", Cow::Borrowed(pair_example(*kind))),
            ],
            Self::UnmatchedClose { kind, .. } => vec![
                ("open", Cow::Borrowed(kind.open_str())),
                ("close", Cow::Borrowed(kind.close_str())),
            ],
            Self::MismatchedContainerClose {
                open_kind,
                close_kind,
                ..
            } => vec![
                ("open_kind", Cow::Borrowed(*open_kind)),
                ("close_kind", Cow::Borrowed(*close_kind)),
            ],
            Self::BreakInSingleLineContainer { container, .. } => {
                vec![("container", Cow::Borrowed(*container))]
            }
            Self::MismatchedBoutenContainer {
                open_family,
                close_family,
                ..
            } => vec![
                ("open_family", Cow::Borrowed(*open_family)),
                ("close_family", Cow::Borrowed(*close_family)),
            ],
            Self::NonCanonicalDirective { canonical, .. } => {
                vec![("canonical", canonical.clone())]
            }
            Self::AccentDecompositionApplied { .. }
            | Self::UnresolvedGaiji { .. }
            | Self::EmptyRubyReading { .. }
            | Self::NestedRuby { .. }
            | Self::UnrecognisedContainerDirective { .. }
            | Self::TcyTargetNotFound { .. }
            | Self::BoutenTargetAmbiguous { .. }
            | Self::ForwardReferentNotStylable { .. }
            | Self::BracketedKaeritenNoPair { .. }
            | Self::KaeritenOutsideKanbun { .. }
            | Self::Internal { .. } => Vec::new(),
        }
    }

    /// Whether an editor should mark this span as unnecessary.
    #[must_use]
    pub fn is_unnecessary(&self) -> bool {
        match self {
            Self::SourceContainsPua { .. } => true,
            Self::UnclosedBracket { .. }
            | Self::UnmatchedClose { .. }
            | Self::AccentDecompositionApplied { .. }
            | Self::UnresolvedGaiji { .. }
            | Self::MismatchedContainerClose { .. }
            | Self::EmptyRubyReading { .. }
            | Self::NestedRuby { .. }
            | Self::UnrecognisedContainerDirective { .. }
            | Self::TcyTargetNotFound { .. }
            | Self::BoutenTargetAmbiguous { .. }
            | Self::ForwardReferentNotStylable { .. }
            | Self::BreakInSingleLineContainer { .. }
            | Self::BracketedKaeritenNoPair { .. }
            | Self::KaeritenOutsideKanbun { .. }
            | Self::MismatchedBoutenContainer { .. }
            | Self::NonCanonicalDirective { .. }
            | Self::Internal { .. } => false,
        }
    }

    /// Every stable diagnostic code in catalogue order.
    pub const ALL_CODES: [&'static str; 21] = [
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
        codes::FORWARD_REFERENT_NOT_STYLABLE,
        codes::BREAK_IN_SINGLE_LINE_CONTAINER,
        codes::BRACKETED_KAERITEN_NO_PAIR,
        codes::KAERITEN_OUTSIDE_KANBUN,
        codes::MISMATCHED_BOUTEN_CONTAINER,
        codes::NON_CANONICAL_DIRECTIVE,
        codes::RESIDUAL_ANNOTATION_MARKER,
        codes::UNREGISTERED_SENTINEL,
        codes::REGISTRY_OUT_OF_ORDER,
        codes::REGISTRY_POSITION_MISMATCH,
    ];

    /// Metadata for a stable code, or `None` for an unknown code.
    #[must_use]
    pub fn explain(code: &str) -> Option<DiagnosticInfo> {
        let sample = Self::sample_for_code(code)?;
        let doc = doc_for(sample.code())?;
        Some(DiagnosticInfo {
            code: sample.code(),
            severity: sample.severity(),
            source: sample.source(),
            help: MietteDiagnostic::help(&sample)
                .map(|h| h.to_string())
                .unwrap_or_default(),
            url: MietteDiagnostic::url(&sample).map(|u| u.to_string()),
            repro: doc.repro,
            fixed: doc.fixed,
            body_args: sample
                .body_args()
                .into_iter()
                .map(|(name, value)| (name, value.into_owned()))
                .collect(),
        })
    }

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
            codes::FORWARD_REFERENT_NOT_STYLABLE => Self::forward_referent_not_stylable(at),
            codes::BREAK_IN_SINGLE_LINE_CONTAINER => {
                Self::break_in_single_line_container(at, "align-end")
            }
            codes::BRACKETED_KAERITEN_NO_PAIR => Self::bracketed_kaeriten_no_pair(at),
            codes::KAERITEN_OUTSIDE_KANBUN => Self::kaeriten_outside_kanbun(at),
            codes::MISMATCHED_BOUTEN_CONTAINER => {
                Self::mismatched_bouten_container(at, "傍点", "傍線")
            }
            codes::NON_CANONICAL_DIRECTIVE => Self::non_canonical_directive(at, "中央揃え"),
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
    fn shifted_rebases_span_and_keeps_at_in_sync() {
        let diag = Diagnostic::source_contains_pua(Span::new(5, 8), '\u{E001}');
        let moved = diag.shifted(100);
        assert_eq!(moved.span(), Span::new(105, 108));
        let Diagnostic::SourceContainsPua { at, .. } = moved else {
            panic!("variant must survive the shift");
        };
        assert_eq!(at.offset(), 105);
        assert_eq!(at.len(), 3);
    }

    #[test]
    fn shifted_is_additive_inverse() {
        let diag = Diagnostic::unclosed_bracket(Span::new(40, 43), PairKind::Bracket);
        let there_and_back = diag.clone().shifted(1000).shifted(-1000);
        assert_eq!(there_and_back.span(), diag.span());
        assert_eq!(there_and_back.code(), diag.code());
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
        assert_eq!(
            format!("{diag}"),
            "source contains lexer PUA sentinel codepoint U+E002"
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
        assert_eq!(format!("{diag}"), "unclosed Aozora 〔…〕 delimiter");
    }

    #[test]
    fn unmatched_close_display_mentions_kind() {
        let diag = Diagnostic::unmatched_close(Span::new(0, 3), PairKind::Quote);
        assert_eq!(format!("{diag}"), "unmatched Aozora 」 close delimiter");
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
            codes::FORWARD_REFERENT_NOT_STYLABLE,
            "aozora::lex::forward_referent_not_stylable"
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

        let not_stylable = Diagnostic::forward_referent_not_stylable(Span::new(0, 18));
        assert_eq!(not_stylable.severity(), Severity::Warning);
        assert_eq!(not_stylable.source(), DiagnosticSource::Source);
        assert_eq!(not_stylable.code(), codes::FORWARD_REFERENT_NOT_STYLABLE);

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

    #[test]
    fn explain_covers_every_catalogued_code() {
        assert_eq!(
            Diagnostic::ALL_CODES.len(),
            21,
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
            assert!(!info.repro.trim().is_empty(), "{code}: empty repro");
            assert!(!info.fixed.trim().is_empty(), "{code}: empty fixed");
        }
    }

    #[test]
    fn severity_as_json_str_is_stable_per_variant() {
        assert_eq!(Severity::Error.as_json_str(), "error");
        assert_eq!(Severity::Warning.as_json_str(), "warning");
        assert_eq!(Severity::Note.as_json_str(), "note");
    }

    #[test]
    fn diagnostic_source_as_json_str_is_stable_per_variant() {
        assert_eq!(DiagnosticSource::Source.as_json_str(), "source");
        assert_eq!(DiagnosticSource::Internal.as_json_str(), "internal");
    }

    #[test]
    fn pair_example_is_the_canonical_form_per_family() {
        assert_eq!(pair_example(PairKind::Ruby), "｜青空《あおぞら》");
        assert_eq!(pair_example(PairKind::AngleQuote), "≪重要≫");
        assert_eq!(pair_example(PairKind::Tortoise), "〔Crevez chiens〕");
        assert_eq!(pair_example(PairKind::Quote), "［＃「青空」に傍点］");
        assert_eq!(pair_example(PairKind::Bracket), "［＃改ページ］");
    }

    #[test]
    fn doc_for_returns_the_entry_matching_the_requested_code() {
        for &code in &Diagnostic::ALL_CODES {
            assert_eq!(
                doc_for(code).expect("every catalogued code has a doc").code,
                code,
            );
        }
        assert!(doc_for("aozora::lex::does_not_exist").is_none());
    }

    #[test]
    fn docs_table_has_one_entry_per_code_in_order() {
        assert_eq!(
            DOCS.len(),
            Diagnostic::ALL_CODES.len(),
            "DOCS must have exactly one entry per ALL_CODES entry"
        );
        for (doc, &code) in DOCS.iter().zip(Diagnostic::ALL_CODES.iter()) {
            assert_eq!(doc.code, code, "DOCS order must match ALL_CODES order");
            assert!(doc_for(code).is_some(), "no DOCS entry for {code}");
        }
    }

    #[test]
    fn body_args_are_instance_specific_for_carrying_variants() {
        let bracket = Diagnostic::unclosed_bracket(Span::new(0, 1), PairKind::Bracket).body_args();
        let ruby = Diagnostic::unclosed_bracket(Span::new(0, 1), PairKind::Ruby).body_args();
        assert_ne!(bracket, ruby);
        assert!(
            bracket
                .iter()
                .any(|(k, v)| *k == "example" && v.contains('］')),
            "bracket example arg: {bracket:?}"
        );
        assert!(
            ruby.iter()
                .any(|(k, v)| *k == "example" && v.contains('》')),
            "ruby example arg: {ruby:?}"
        );
        assert!(bracket.iter().any(|(k, v)| *k == "open" && v == "［"));
        assert!(bracket.iter().any(|(k, v)| *k == "close" && v == "］"));
    }

    #[test]
    fn body_args_are_empty_for_static_body_variants() {
        assert_eq!(
            Diagnostic::empty_ruby_reading(Span::new(0, 1)).body_args(),
            Vec::<(&str, Cow<'_, str>)>::new()
        );
        assert_eq!(
            Diagnostic::internal(Span::new(0, 0), InternalCheckCode::ResidualAnnotationMarker)
                .body_args(),
            Vec::<(&str, Cow<'_, str>)>::new()
        );
    }

    #[test]
    fn source_pua_body_args_format_the_codepoint_as_hex() {
        let args = Diagnostic::source_contains_pua(Span::new(0, 3), '\u{E002}').body_args();
        assert!(
            args.iter().any(|(k, v)| *k == "codepoint" && v == "E002"),
            "codepoint arg: {args:?}"
        );
        assert!(
            args.iter().any(|(k, v)| *k == "char" && v == "\u{E002}"),
            "char arg: {args:?}"
        );
    }

    #[test]
    fn only_source_pua_is_unnecessary() {
        assert!(Diagnostic::source_contains_pua(Span::new(0, 1), '\u{E001}').is_unnecessary());
        assert!(!Diagnostic::unclosed_bracket(Span::new(0, 1), PairKind::Bracket).is_unnecessary());
        assert!(
            !Diagnostic::internal(Span::new(0, 0), InternalCheckCode::ResidualAnnotationMarker)
                .is_unnecessary()
        );
    }

    #[test]
    fn explain_rejects_unknown_and_unprefixed_codes() {
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
        assert_eq!(resid.help, unreg.help);
        assert_eq!(resid.url, unreg.url);
    }

    #[test]
    fn is_lint_selects_only_the_lint_namespace() {
        let lint = Diagnostic::non_canonical_directive(Span::new(0, 3), "ここで字下げ終わり");
        assert!(lint.is_lint(), "notation-hygiene lint must be a lint");
        assert!(lint.code().starts_with(codes::LINT_NAMESPACE));

        let lex = Diagnostic::unclosed_bracket(Span::new(0, 1), PairKind::Bracket);
        assert!(!lex.is_lint(), "a lex fault is not a lint: {}", lex.code());
        let internal =
            Diagnostic::internal(Span::new(0, 0), InternalCheckCode::ResidualAnnotationMarker);
        assert!(!internal.is_lint(), "an internal check is not a lint");
    }
}
