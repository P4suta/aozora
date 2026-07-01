//! Renderer-agnostic catalogue for aozora lexer diagnostics.
//!
//! [`describe`] turns an [`aozora::Diagnostic`] into a [`Described`] — a
//! severity-neutral record that carries the diagnostic's stable `code`, its
//! rich Japanese message, the source [`Span`], and the quick-fix
//! [`DiagnosticPayload`] — **without depending on any LSP types**. The
//! `aozora-lsp` crate adapts a [`Described`] into a `tower_lsp` `Diagnostic`;
//! the `aozora` CLI renders one to the terminal. Both reach the same single
//! source of truth here, so the diagnostic catalogue never forks.
//!
//! [`CATALOGUE`] / [`lookup`] back the `aozora explain <code>` command with
//! long-form prose, a minimal reproduction, and the corrected form for every
//! diagnostic code.
//!
//! ## Message style
//!
//! Each diagnostic message is written for the *typesetter*, not the parser
//! author. Three things every variant should answer:
//!
//! 1. **何が起きた** — plain summary in the first sentence
//! 2. **何が問題** — why this matters in plain Japanese
//! 3. **どう直す** — at least one concrete example of the corrected form,
//!    written in actual aozora notation
//!
//! `unnecessary` is set when the lint is "unnecessary" (an editor can grey out
//! unnecessary code). `payload` carries enough context for a quick-fix to be
//! constructed without re-parsing.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use aozora::{Diagnostic as AozoraDiagnostic, Document, InternalCheckCode, PairKind, Span};
use serde::{Deserialize, Serialize};

/// Compiles and runs the fenced Rust example in this crate's `README.md` as a
/// doctest, so the documented public API can't silently drift from the code.
/// `#[cfg(doctest)]` means the item exists only while rustdoc collects
/// doctests — it never reaches a normal build.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

/// Severity of a diagnostic, independent of any client's severity enum.
///
/// The `aozora-lsp` adapter maps this onto `tower_lsp`'s `DiagnosticSeverity`;
/// the CLI maps it onto a colour and a label. The aozora lexer only ever emits
/// errors and warnings, so those are the only two variants.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// A hard error: the document cannot be parsed/rendered as intended.
    Error,
    /// A warning: parsing continues, but something is likely wrong.
    Warning,
}

/// Serialised payload attached to a diagnostic. Lets a quick-fix handler build
/// an edit without re-parsing or re-classifying the offending span.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum DiagnosticPayload {
    /// `UnclosedBracket` — the open delimiter is here; the missing close is one
    /// of the chars in `expected_close`.
    UnclosedBracket {
        /// Which delimiter pair was left open.
        pair_kind: SerializablePairKind,
        /// The close delimiter that would balance it.
        expected_close: String,
    },
    /// `UnmatchedClose` — the close delimiter is here without a matching open.
    UnmatchedClose {
        /// Which delimiter pair the stray close belongs to.
        pair_kind: SerializablePairKind,
    },
    /// `SourceContainsPua` — a private-use codepoint clashes with the lexer's
    /// sentinel reservations.
    SourceContainsPua {
        /// The offending Unicode scalar value.
        codepoint: u32,
    },
    /// `ResidualAnnotationMarker` — `［＃...］` pair survived classification
    /// (likely a typo or unsupported keyword).
    ResidualAnnotationMarker,
    /// `NonCanonicalDirective` — the flagged `［＃…］` body is a verified
    /// near-miss; `canonical` is the catalogue spelling the quick-fix
    /// substitutes (bracket span and all).
    NonCanonicalDirective {
        /// The canonical directive body (without the `［＃` / `］` delimiters).
        canonical: String,
    },
}

/// Stringified [`PairKind`] for `serde_json` round-tripping.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SerializablePairKind {
    /// `［` … `］`
    Bracket,
    /// `《` … `》`
    Ruby,
    /// `≪` … `≫`
    AngleQuote,
    /// `〔` … `〕`
    Tortoise,
    /// `「` … `」`
    Quote,
}

impl From<PairKind> for SerializablePairKind {
    fn from(k: PairKind) -> Self {
        // `PairKind` is `#[non_exhaustive]`, so we have to handle
        // future-added variants. Merging `Bracket` with the wildcard
        // (`PairKind::Bracket | _`) makes the fallback explicit
        // without giving clippy two arms with identical bodies — the
        // pre-merge shape (`PairKind::Bracket => Self::Bracket` as a
        // distinct arm plus a separate `_ => Self::Bracket`) tripped
        // `clippy::match_same_arms` on the duplicate body.
        match k {
            PairKind::Ruby => Self::Ruby,
            PairKind::AngleQuote => Self::AngleQuote,
            PairKind::Tortoise => Self::Tortoise,
            PairKind::Quote => Self::Quote,
            PairKind::Bracket | _ => Self::Bracket,
        }
    }
}

impl SerializablePairKind {
    /// Human-readable open delimiter literal (`［`, `《`, `≪`, `〔`, `「`).
    #[must_use]
    pub const fn open_str(self) -> &'static str {
        match self {
            Self::Bracket => "［",
            Self::Ruby => "《",
            Self::AngleQuote => "≪",
            Self::Tortoise => "〔",
            Self::Quote => "「",
        }
    }

    /// Human-readable close delimiter literal.
    #[must_use]
    pub const fn close_str(self) -> &'static str {
        match self {
            Self::Bracket => "］",
            Self::Ruby => "》",
            Self::AngleQuote => "≫",
            Self::Tortoise => "〕",
            Self::Quote => "」",
        }
    }
}

/// One fully-described diagnostic, independent of any renderer.
///
/// Built by [`describe`] from an [`aozora::Diagnostic`]. The `aozora-lsp`
/// adapter turns this into an LSP `Diagnostic`; the CLI renders it to the
/// terminal.
#[derive(Debug, Clone)]
pub struct Described {
    /// Byte-range span into the original source buffer.
    pub span: Span,
    /// The verbose Japanese message (何が起きた / 何が問題 / どう直す).
    pub message: String,
    /// Stable diagnostic code, e.g. `"aozora::unclosed-bracket"`.
    pub code: &'static str,
    /// Error or warning.
    pub severity: Severity,
    /// `true` when the lint marks code as unnecessary (an editor can grey it
    /// out; LSP maps this to `DiagnosticTag::UNNECESSARY`).
    pub unnecessary: bool,
    /// Context for a quick-fix, when one is applicable.
    pub payload: Option<DiagnosticPayload>,
}

/// Parse `source` and describe every diagnostic it produces.
///
/// This is the terminal/CLI entry point: it returns renderer-neutral
/// [`Described`] records with no LSP types involved.
#[must_use]
pub fn describe_source(source: &str) -> Vec<Described> {
    Document::new(source)
        .parse()
        .diagnostics()
        .iter()
        .map(describe)
        .collect()
}

/// Describe a single [`aozora::Diagnostic`].
///
/// Top-level dispatcher: unpacks the diagnostic variant and delegates to a
/// per-variant helper, keeping this function short and each catalogue entry
/// independently navigable.
#[must_use]
pub fn describe(d: &AozoraDiagnostic) -> Described {
    match d {
        AozoraDiagnostic::SourceContainsPua {
            span, codepoint, ..
        } => describe_source_contains_pua(*span, *codepoint),
        AozoraDiagnostic::UnclosedBracket { span, kind, .. } => {
            describe_unclosed_bracket(*span, *kind)
        }
        AozoraDiagnostic::UnmatchedClose { span, kind, .. } => {
            describe_unmatched_close(*span, *kind)
        }
        AozoraDiagnostic::NonCanonicalDirective {
            span, canonical, ..
        } => describe_non_canonical_directive(*span, canonical),
        // pipeline-internal sanity checks dispatch on the typed
        // `InternalCheckCode`; each fires a "pipeline bug, please
        // report" style message with the appropriate code.
        AozoraDiagnostic::Internal { span, check, .. } => match check {
            InternalCheckCode::ResidualAnnotationMarker => {
                describe_residual_annotation_marker(*span)
            }
            InternalCheckCode::UnregisteredSentinel => describe_unregistered_sentinel(*span),
            InternalCheckCode::RegistryOutOfOrder => describe_registry_out_of_order(*span),
            InternalCheckCode::RegistryPositionMismatch => {
                describe_registry_position_mismatch(*span)
            }
            // `InternalCheckCode` is `#[non_exhaustive]`; an unknown
            // future variant falls through to a generic warning.
            _ => describe_unknown(d),
        },
        // `aozora::Diagnostic` is `#[non_exhaustive]`; an unknown
        // future variant falls through to a generic warning so the
        // client still sees a marker.
        other => describe_unknown(other),
    }
}

fn describe_source_contains_pua(span: Span, codepoint: char) -> Described {
    Described {
        span,
        message: format!(
            "私用領域文字 `U+{cp:04X}` がソースに紛れ込んでいます。\n\n\
             この文字 (`{ch}`) は青空文庫の通常テキストには現れない予約コードポイントで、aozora-lex の内部マーカー (U+E001..U+E004) と衝突します。\n\
             通常はテキストエディタの非表示文字設定や、コピペ時の不可視サニタイズで混入します。\n\n\
             直し方: 該当の 1 文字を削除してください。",
            cp = codepoint as u32,
            ch = codepoint,
        ),
        code: "aozora::source-contains-pua",
        severity: Severity::Warning,
        unnecessary: true,
        payload: Some(DiagnosticPayload::SourceContainsPua {
            codepoint: codepoint as u32,
        }),
    }
}

fn describe_unclosed_bracket(span: Span, kind: PairKind) -> Described {
    let pk: SerializablePairKind = kind.into();
    let open = pk.open_str();
    let close = pk.close_str();
    let example = example_for(pk);
    Described {
        span,
        message: format!(
            "閉じられていない `{open}` があります。\n\n\
             どこかに対応する `{close}` を必ず置いてください。aozora 記法では一行内で閉じるのが基本です。\n\n\
             例: `{example}`",
        ),
        code: "aozora::unclosed-bracket",
        severity: Severity::Error,
        unnecessary: false,
        payload: Some(DiagnosticPayload::UnclosedBracket {
            pair_kind: pk,
            expected_close: close.to_owned(),
        }),
    }
}

fn describe_unmatched_close(span: Span, kind: PairKind) -> Described {
    let pk: SerializablePairKind = kind.into();
    let open = pk.open_str();
    let close = pk.close_str();
    Described {
        span,
        message: format!(
            "対応する `{open}` のない `{close}` です。\n\n\
             考えられる原因:\n\
             1. 余分な `{close}` を打ってしまった → 削除する\n\
             2. 前にあるはずの `{open}` が欠けている → 適切な位置に追加する\n\
             3. その間に別の `{close}` があり、ペアが一段ずれた → 該当箇所のペアを見直す\n\n\
             右クリックの Quick Fix から「`{close}` を削除する」を選べます。",
        ),
        code: "aozora::unmatched-close",
        severity: Severity::Error,
        unnecessary: false,
        payload: Some(DiagnosticPayload::UnmatchedClose { pair_kind: pk }),
    }
}

fn describe_residual_annotation_marker(span: Span) -> Described {
    Described {
        span,
        message: "未分類の `［＃...］` 注記です。\n\n\
                 注記辞典 (`gaiji_chuki.pdf`) のキーワードに合致しなかったか、誤字の可能性があります。\n\n\
                 確認手順:\n\
                 1. ［＃ の中身が `改ページ` / `中央揃え` などの登録済みキーワードと一致するか確認\n\
                 2. `第3水準1-...` のような JIS X 0213 mencode を付け忘れていないか確認\n\
                 3. それでも不明な場合は description-only 形式 (`※［＃「説明」］`) でひとまず通せます"
            .to_owned(),
        code: "aozora::residual-annotation-marker",
        severity: Severity::Warning,
        unnecessary: false,
        payload: Some(DiagnosticPayload::ResidualAnnotationMarker),
    }
}

fn describe_unregistered_sentinel(span: Span) -> Described {
    Described {
        span,
        message: "未登録の私用領域 sentinel が検出されました (pipeline 内部の整合性エラー)。\n\n\
             これは aozora-pipeline のバグの可能性が高いです。再現手順を添えて issue で報告してください。"
            .to_owned(),
        code: "aozora::unregistered-sentinel",
        severity: Severity::Error,
        unnecessary: false,
        payload: None,
    }
}

fn describe_registry_out_of_order(span: Span) -> Described {
    Described {
        span,
        message:
            "プレースホルダーレジストリの順序が崩れています (pipeline 内部の整合性エラー)。\n\n\
             aozora-pipeline のバグの可能性があります。"
                .to_owned(),
        code: "aozora::registry-out-of-order",
        severity: Severity::Error,
        unnecessary: false,
        payload: None,
    }
}

fn describe_registry_position_mismatch(span: Span) -> Described {
    Described {
        span,
        message: "プレースホルダーレジストリの位置情報が期待と異なっています (pipeline 内部の整合性エラー)。\n\n\
             aozora-pipeline のバグの可能性があります。"
            .to_owned(),
        code: "aozora::registry-position-mismatch",
        severity: Severity::Error,
        unnecessary: false,
        payload: None,
    }
}

fn describe_non_canonical_directive(span: Span, canonical: &str) -> Described {
    Described {
        span,
        message: format!(
            "非正規の綴りの ［＃…］ 注記です。正規形は `［＃{canonical}］` です。\n\n\
             この注記の中身は、登録済みの記法を非正規な綴り（送り仮名・同義語・綴りゆれ）で書いたものと判定され、Unknown 注記のまま保持されています。パーサは中身を書き換えません。\n\n\
             直し方: `［＃{canonical}］` に書き換えてください。`aozora fmt --fix-notation` で自動修正できます。",
        ),
        code: "aozora::lint::non_canonical_directive",
        severity: Severity::Warning,
        unnecessary: false,
        payload: Some(DiagnosticPayload::NonCanonicalDirective {
            canonical: canonical.to_owned(),
        }),
    }
}

fn describe_unknown(other: &AozoraDiagnostic) -> Described {
    Described {
        span: Span::new(0, 0),
        message: format!(
            "未対応の aozora 診断です: {other:?}\n\n\
             aozora-lsp と aozora-lex のバージョンが揃っていない可能性があります。"
        ),
        code: "aozora::unknown-diagnostic",
        severity: Severity::Warning,
        unnecessary: false,
        payload: None,
    }
}

/// Per-kind canonical example used in the unclosed-bracket message and reused
/// as the `fixed` form in the [`CATALOGUE`].
#[must_use]
pub const fn example_for(kind: SerializablePairKind) -> &'static str {
    match kind {
        SerializablePairKind::Bracket => "［＃改ページ］",
        SerializablePairKind::Ruby => "｜青空《あおぞら》",
        SerializablePairKind::AngleQuote => "≪重要≫",
        SerializablePairKind::Tortoise => "〔Crevez chiens〕",
        SerializablePairKind::Quote => "［＃「青空」に傍点］",
    }
}

/// A long-form explanation entry for one diagnostic code, backing the
/// `aozora explain <code>` command.
#[derive(Debug, Clone, Copy)]
pub struct CatalogueEntry {
    /// The stable diagnostic code this entry explains.
    pub code: &'static str,
    /// One-line title.
    pub title: &'static str,
    /// Long-form why/how prose (may span several paragraphs).
    pub explain: &'static str,
    /// A minimal reproduction in aozora notation.
    pub repro: &'static str,
    /// The corrected form.
    pub fixed: &'static str,
}

/// Every diagnostic code, with its long-form explanation. Backs
/// `aozora explain`. There is exactly one entry per code [`describe`] can emit.
pub const CATALOGUE: &[CatalogueEntry] = &[
    CatalogueEntry {
        code: "aozora::source-contains-pua",
        title: "私用領域文字がソースに紛れ込んでいる",
        explain: "aozora-lex は変換処理の途中で U+E001..U+E004 の私用領域 (PUA) コードポイントを内部マーカー (sentinel) として予約しています。ソース中に同じ領域の文字が含まれていると、この内部マーカーと衝突し、解析が破綻します。\n\n\
                  この文字は通常の青空文庫テキストには現れません。多くはエディタの不可視文字設定や、別アプリからのコピー&ペーストで紛れ込みます。\n\n\
                  直し方: 該当の 1 文字を削除してください。エディタの「不可視文字を表示」機能で位置を特定できます。",
        repro: "（不可視の U+E001 などが混入した行）",
        fixed: "（その 1 文字を削除した行）",
    },
    CatalogueEntry {
        code: "aozora::unclosed-bracket",
        title: "閉じられていない開き括弧",
        explain: "aozora 記法では `［` … `］`、`《` … `》`、`「` … `」` などの括弧ペアは原則として同じ行の中で閉じる必要があります。閉じ括弧が見つからないまま行が終わると、注記やルビの範囲が確定できず、後段の整形・HTML 化が破綻します。\n\n\
                  ヒント: 注記 `［＃…］` の中身が `改ページ` などの登録済みキーワードか確認してください。未登録の場合は aozora::residual-annotation-marker も併せて出ることがあります。",
        repro: "本文［＃改ページ",
        fixed: "本文［＃改ページ］",
    },
    CatalogueEntry {
        code: "aozora::unmatched-close",
        title: "対応する開き括弧のない閉じ括弧",
        explain: "閉じ括弧 (`］` `》` `」` など) に対応する開き括弧が見つかりません。\n\n\
                  考えられる原因:\n\
                  1. 余分な閉じ括弧を打ってしまった → 削除する\n\
                  2. 前にあるはずの開き括弧が欠けている → 適切な位置に追加する\n\
                  3. 途中に別の閉じ括弧があり、ペアが一段ずれた → 該当箇所のペアを見直す",
        repro: "本文 ］",
        fixed: "本文",
    },
    CatalogueEntry {
        code: "aozora::residual-annotation-marker",
        title: "未分類の ［＃…］ 注記",
        explain: "`［＃…］` の注記が、注記辞典 (gaiji_chuki) のどのキーワードにも一致しませんでした。誤字か、未対応の注記の可能性があります。\n\n\
                  確認手順:\n\
                  1. ［＃ の中身が `改ページ` `中央揃え` などの登録済みキーワードと一致するか確認\n\
                  2. `第3水準1-…` のような JIS X 0213 面区点コードを付け忘れていないか確認\n\
                  3. それでも不明なら、説明のみ形式 (`※［＃「説明」］`) でひとまず通せます",
        repro: "本文［＃なぞの注記］",
        fixed: "本文［＃改ページ］",
    },
    CatalogueEntry {
        code: "aozora::unregistered-sentinel",
        title: "未登録の内部 sentinel（パイプライン内部エラー）",
        explain: "解析パイプライン内部の整合性チェックが、レジストリに登録されていない私用領域 sentinel を検出しました。通常のソースからは到達しないコードで、aozora のパイプライン側のバグである可能性が高いです。\n\n\
                  再現手順を添えて issue で報告してください: https://github.com/P4suta/aozora-tools/issues",
        repro: "（通常のソースからは発生しません）",
        fixed: "（パイプラインのバグ。手元の修正では直せません）",
    },
    CatalogueEntry {
        code: "aozora::registry-out-of-order",
        title: "プレースホルダーレジストリの順序破壊（パイプライン内部エラー）",
        explain: "プレースホルダーレジストリの並び順が、期待される昇順から崩れています。通常のソースからは到達しない内部整合性エラーで、aozora のパイプライン側のバグである可能性が高いです。\n\n\
                  再現手順を添えて issue で報告してください: https://github.com/P4suta/aozora-tools/issues",
        repro: "（通常のソースからは発生しません）",
        fixed: "（パイプラインのバグ。手元の修正では直せません）",
    },
    CatalogueEntry {
        code: "aozora::registry-position-mismatch",
        title: "プレースホルダーレジストリの位置不一致（パイプライン内部エラー）",
        explain: "プレースホルダーが記録していた位置情報が、復元時の実際の位置と一致しませんでした。通常のソースからは到達しない内部整合性エラーで、aozora のパイプライン側のバグである可能性が高いです。\n\n\
                  再現手順を添えて issue で報告してください: https://github.com/P4suta/aozora-tools/issues",
        repro: "（通常のソースからは発生しません）",
        fixed: "（パイプラインのバグ。手元の修正では直せません）",
    },
    CatalogueEntry {
        code: "aozora::lint::non_canonical_directive",
        title: "非正規の綴りの ［＃…］ 注記",
        explain: "`［＃…］` の中身が、登録済みの記法を非正規な綴り（送り仮名・同義語・綴りゆれ）で書いたものと判定されました。例: `字下げ終わり`（正規形 `ここで字下げ終わり`）、`黒丸傍点`（正規形 `丸傍点`）、`中央寄せ`（正規形 `中央揃え`）。\n\n\
                  パーサはこれを勝手に解釈せず Unknown 注記のまま保持します（入力者の誤記を記法として黙認しないため）。これは助言のみの警告で、`--strict` でない限り終了コードには影響しません。\n\n\
                  直し方: 提示された正規形に書き換えてください。`aozora fmt --fix-notation` で一括自動修正できます。",
        repro: "本文［＃字下げ終わり］",
        fixed: "本文［＃ここで字下げ終わり］",
    },
    CatalogueEntry {
        code: "aozora::unknown-diagnostic",
        title: "未対応の診断",
        explain: "aozora-tools が認識していない種類の診断を、upstream の aozora パーサが返しました。aozora-tools と aozora パーサのバージョンが揃っていない可能性があります。\n\n\
                  ツールを最新版に更新するか、バージョンの不一致がないか確認してください。",
        repro: "（バージョン不一致時に発生）",
        fixed: "（ツールのバージョンを揃えて再実行）",
    },
];

/// Look up a [`CatalogueEntry`] by its exact `code` string.
#[must_use]
pub fn lookup(code: &str) -> Option<&'static CatalogueEntry> {
    CATALOGUE.iter().find(|e| e.code == code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_has_no_diagnostics() {
        assert!(describe_source("hello world").is_empty());
    }

    #[test]
    fn canonical_ruby_has_no_diagnostics() {
        assert!(describe_source("｜日本《にほん》").is_empty());
    }

    #[test]
    fn source_contains_pua_is_an_unnecessary_warning_with_payload() {
        let described = describe_source("abc\u{E001}def");
        let pua = described
            .iter()
            .find(|d| d.code == "aozora::source-contains-pua")
            .expect("PUA warning expected");
        assert!(pua.message.contains("削除"), "msg: {}", pua.message);
        assert_eq!(pua.severity, Severity::Warning);
        assert!(pua.unnecessary, "PUA lint should be marked unnecessary");
        assert!(pua.payload.is_some(), "payload should be attached");
    }

    #[test]
    fn unclosed_bracket_message_carries_example_and_close_char() {
        // `［＃改ページ` (no closing ］) — must surface as UnclosedBracket.
        let described = describe_source("本文［＃改ページ");
        let unclosed = described
            .iter()
            .find(|d| d.code == "aozora::unclosed-bracket")
            .expect("UnclosedBracket expected on missing ］");
        assert_eq!(unclosed.severity, Severity::Error);
        assert!(unclosed.message.contains('］'), "{}", unclosed.message);
        assert!(
            unclosed.message.contains("例:"),
            "message must include a concrete example: {}",
            unclosed.message,
        );
        assert!(unclosed.payload.is_some(), "payload required for quick-fix");
    }

    #[test]
    fn unmatched_close_message_lists_three_causes() {
        let described = describe_source("本文 ］");
        let unmatched = described
            .iter()
            .find(|d| d.code == "aozora::unmatched-close")
            .expect("UnmatchedClose expected on stray ］");
        assert!(unmatched.message.contains("削除"), "{}", unmatched.message);
        assert!(
            unmatched.message.contains("欠けている"),
            "{}",
            unmatched.message
        );
    }

    #[test]
    fn non_canonical_directive_carries_canonical_payload() {
        // `字下げ終わり` is a near-miss of the block close `ここで字下げ終わり`.
        let described = describe_source("本文［＃字下げ終わり］");
        let lint = described
            .iter()
            .find(|d| d.code == "aozora::lint::non_canonical_directive")
            .expect("non-canonical-directive lint expected");
        assert_eq!(lint.severity, Severity::Warning);
        assert!(
            lint.message.contains("ここで字下げ終わり"),
            "message must name the canonical form: {}",
            lint.message
        );
        match &lint.payload {
            Some(DiagnosticPayload::NonCanonicalDirective { canonical }) => {
                assert_eq!(canonical, "ここで字下げ終わり");
            }
            other => panic!("expected NonCanonicalDirective payload, got {other:?}"),
        }
    }

    #[test]
    fn pair_kind_maps_to_serializable_for_every_variant() {
        assert_eq!(
            SerializablePairKind::from(PairKind::Bracket),
            SerializablePairKind::Bracket
        );
        assert_eq!(
            SerializablePairKind::from(PairKind::Ruby),
            SerializablePairKind::Ruby
        );
        assert_eq!(
            SerializablePairKind::from(PairKind::AngleQuote),
            SerializablePairKind::AngleQuote
        );
        assert_eq!(
            SerializablePairKind::from(PairKind::Tortoise),
            SerializablePairKind::Tortoise
        );
        assert_eq!(
            SerializablePairKind::from(PairKind::Quote),
            SerializablePairKind::Quote
        );
    }

    #[test]
    fn delimiters_and_examples_cover_every_pair_kind() {
        use SerializablePairKind::{AngleQuote, Bracket, Quote, Ruby, Tortoise};
        let cases = [
            (Bracket, "［", "］"),
            (Ruby, "《", "》"),
            (AngleQuote, "≪", "≫"),
            (Tortoise, "〔", "〕"),
            (Quote, "「", "」"),
        ];
        for (pk, open, close) in cases {
            assert_eq!(pk.open_str(), open, "open_str for {pk:?}");
            assert_eq!(pk.close_str(), close, "close_str for {pk:?}");
            assert!(
                example_for(pk).contains(open),
                "example for {pk:?} should use its opener {open}",
            );
        }
    }

    /// The four `Internal`/`describe_*` consistency-error helpers fire on
    /// pipeline bugs that aren't reachable from ordinary source, so drive them
    /// directly to pin their codes / severities / non-empty bodies.
    #[test]
    fn internal_consistency_descriptions_carry_codes_and_severity() {
        let span = Span::new(0, 0);
        let cases = [
            (
                describe_residual_annotation_marker(span),
                "aozora::residual-annotation-marker",
                Severity::Warning,
            ),
            (
                describe_unregistered_sentinel(span),
                "aozora::unregistered-sentinel",
                Severity::Error,
            ),
            (
                describe_registry_out_of_order(span),
                "aozora::registry-out-of-order",
                Severity::Error,
            ),
            (
                describe_registry_position_mismatch(span),
                "aozora::registry-position-mismatch",
                Severity::Error,
            ),
        ];
        for (described, code, severity) in cases {
            assert_eq!(described.code, code);
            assert_eq!(described.severity, severity);
            assert!(!described.message.is_empty(), "{code} needs a message");
        }
    }

    #[test]
    fn unknown_fallback_describes_any_diagnostic_generically() {
        // Route a real (known) diagnostic through the unknown fallback to
        // exercise its generic `{other:?}` formatting body — the variant it
        // normally catches (a future aozora variant) can't be constructed.
        let doc = Document::new("abc\u{E001}def");
        let tree = doc.parse();
        let real = tree.diagnostics().first().expect("a diagnostic");
        let described = describe_unknown(real);
        assert_eq!(described.code, "aozora::unknown-diagnostic");
        assert!(
            described.message.contains("未対応"),
            "{}",
            described.message
        );
        assert_eq!(described.span.start, 0);
    }

    #[test]
    fn payload_round_trips_through_json() {
        let payload = DiagnosticPayload::UnclosedBracket {
            pair_kind: SerializablePairKind::Bracket,
            expected_close: "］".to_owned(),
        };
        let json = serde_json::to_value(&payload).unwrap();
        let back: DiagnosticPayload = serde_json::from_value(json).unwrap();
        match back {
            DiagnosticPayload::UnclosedBracket {
                pair_kind,
                expected_close,
            } => {
                assert_eq!(pair_kind, SerializablePairKind::Bracket);
                assert_eq!(expected_close, "］");
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    // ---- catalogue (explain) ----

    #[test]
    fn catalogue_has_an_entry_for_every_emittable_code() {
        // Every code `describe` can emit must have a long-form explanation.
        const CODES: &[&str] = &[
            "aozora::source-contains-pua",
            "aozora::unclosed-bracket",
            "aozora::unmatched-close",
            "aozora::residual-annotation-marker",
            "aozora::unregistered-sentinel",
            "aozora::registry-out-of-order",
            "aozora::registry-position-mismatch",
            "aozora::lint::non_canonical_directive",
            "aozora::unknown-diagnostic",
        ];
        for code in CODES {
            assert!(lookup(code).is_some(), "no catalogue entry for {code}");
        }
        assert_eq!(
            CATALOGUE.len(),
            CODES.len(),
            "catalogue has entries for codes not in the emittable set (or vice versa)"
        );
    }

    #[test]
    fn catalogue_codes_are_unique_and_well_formed() {
        for entry in CATALOGUE {
            assert!(
                entry.code.starts_with("aozora::"),
                "code should be namespaced: {}",
                entry.code
            );
            assert!(!entry.title.is_empty(), "{} needs a title", entry.code);
            assert!(
                !entry.explain.is_empty(),
                "{} needs explain text",
                entry.code
            );
            let duplicates = CATALOGUE.iter().filter(|e| e.code == entry.code).count();
            assert_eq!(duplicates, 1, "duplicate catalogue code: {}", entry.code);
        }
    }

    #[test]
    fn lookup_misses_return_none() {
        assert!(lookup("aozora::does-not-exist").is_none());
        assert!(lookup("").is_none());
    }
}
