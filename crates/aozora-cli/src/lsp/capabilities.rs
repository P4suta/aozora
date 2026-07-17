//! What the server advertises in its `initialize` response.
//!
//! A declaration, not a handler: the value depends on nothing but this
//! crate's own constants — no request, no document, no `self` — so it lives
//! apart from [`crate::lsp::backend`]'s request bodies, where "what does this
//! server offer?" is answerable without scrolling past two thousand lines of
//! handlers.
//!
//! `tests/snapshots.rs` snapshots [`server_capabilities`] serialized.
//! `lsp-types` omits `None` options from the JSON, so the key set of that
//! snapshot *is* the advertised set: gaining or losing a capability lands in
//! review as a diff.

use tower_lsp::lsp_types::{
    CodeActionKind, CodeActionOptions, CodeActionProviderCapability, CompletionOptions,
    DocumentOnTypeFormattingOptions, ExecuteCommandOptions, FoldingRangeProviderCapability,
    HoverProviderCapability, LinkedEditingRangeServerCapabilities, OneOf, RenameOptions,
    SemanticTokensFullOptions, SemanticTokensLegend, SemanticTokensOptions,
    SemanticTokensServerCapabilities, ServerCapabilities, ServerInfo, TextDocumentSyncCapability,
    TextDocumentSyncKind, WorkDoneProgressOptions,
};

use crate::lsp::commands::COMMAND_CANONICALIZE_SLUG;
use crate::lsp::on_type_formatting::TRIGGERS as ON_TYPE_TRIGGERS;
use crate::lsp::semantic_tokens::legend as semantic_token_legend;

/// Every capability this server offers a client.
///
/// Allocates (the trigger lists, the token legend), so this cannot be a
/// `const` — it is rebuilt per handshake, of which there is one.
#[must_use]
pub(super) fn server_capabilities() -> ServerCapabilities {
    ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(
            TextDocumentSyncKind::INCREMENTAL,
        )),
        document_formatting_provider: Some(OneOf::Left(true)),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        // `inlay_hint_provider` deliberately omitted — the VS Code
        // extension already renders the resolved character inline via
        // the `gaijiFold.ts` decoration, so an LSP inlay would add a
        // second `→ X` beside it. The client cannot suppress the inlay
        // from the decoration side, and the server cannot emit these
        // selectively because only the client knows the cursor, so the
        // duplication is not fixable from either end. Clients that want
        // the data use `aozora/gaijiSpans`.
        linked_editing_range_provider: Some(LinkedEditingRangeServerCapabilities::Simple(true)),
        // Coupled rename (the LSP face of the #202 splice engine):
        // renaming one site of a coupling (a container open marker, a
        // forward-reference / heading-hint / margin-note directive)
        // edits its partner coherently. `prepare_provider` advertises
        // `textDocument/prepareRename`, which gates the rename to
        // coupled regions only.
        rename_provider: Some(OneOf::Right(RenameOptions {
            prepare_provider: Some(true),
            work_done_progress_options: WorkDoneProgressOptions::default(),
        })),
        completion_provider: Some(CompletionOptions {
            // Two completion paths share the trigger list:
            //
            // * Slug catalogue (`crate::lsp::completion`) — fires
            //   on `＃` (after `［`) or `#` (after `[`), and
            //   on `「` for forward-reference quotes
            //   (`［＃「target」に傍点］`).
            // * Half-width emmet (`crate::lsp::half_width_emmet`)
            //   — fires on `[`, `]`, `<`, `>`, `|`, `*`. Each
            //   suggests the corresponding full-width glyph
            //   (`［`, `］`, `《...》`, `》`, `｜`, `※`) and
            //   on accept replaces the typed prefix verbatim.
            //   The completion path is the secondary surface;
            //   the primary surface is `onTypeFormatting`
            //   below, which converts on every keystroke
            //   without needing the user to dismiss a popup.
            trigger_characters: Some(vec![
                "＃".to_owned(),
                "#".to_owned(),
                "「".to_owned(),
                "[".to_owned(),
                "]".to_owned(),
                "<".to_owned(),
                ">".to_owned(),
                "|".to_owned(),
                "*".to_owned(),
                // Structured-snippet triggers — fire after
                // `onTypeFormatting` has converted the
                // half-width form. The completion handler
                // routes these to `crate::lsp::structured_snippets`.
                "｜".to_owned(),
                "《".to_owned(),
                "※".to_owned(),
            ]),
            resolve_provider: Some(false),
            ..Default::default()
        }),
        // The primary half-width → full-width conversion
        // surface. VS Code fires `onTypeFormatting` the
        // moment any of these chars is typed and applies the
        // returned `TextEdit` immediately — no popup, no
        // accept keystroke. See `crate::lsp::on_type_formatting`
        // for the rationale and safety analysis. Requires
        // `editor.formatOnType: true` on the client; the
        // VS Code extension sets that as a default for the
        // `aozora` language.
        document_on_type_formatting_provider: Some(DocumentOnTypeFormattingOptions {
            first_trigger_character: ON_TYPE_TRIGGERS[0].to_owned(),
            more_trigger_character: Some(
                ON_TYPE_TRIGGERS[1..]
                    .iter()
                    .map(|&s| s.to_owned())
                    .collect(),
            ),
        }),
        execute_command_provider: Some(ExecuteCommandOptions {
            commands: vec![COMMAND_CANONICALIZE_SLUG.to_owned()],
            ..Default::default()
        }),
        code_action_provider: Some(CodeActionProviderCapability::Options(CodeActionOptions {
            // Advertised so VS Code shows the actions
            // under right-click → Refactor and the
            // Ctrl+. lightbulb. Resolve is not yet wired
            // because every action ships a complete
            // edit; resolve_provider stays None until a
            // future heavier action (e.g. "rename slug
            // across document") needs lazy loading.
            code_action_kinds: Some(vec![
                CodeActionKind::QUICKFIX,
                CodeActionKind::REFACTOR_REWRITE,
            ]),
            ..CodeActionOptions::default()
        })),
        folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),
        document_symbol_provider: Some(OneOf::Left(true)),
        semantic_tokens_provider: Some(SemanticTokensServerCapabilities::SemanticTokensOptions(
            SemanticTokensOptions {
                work_done_progress_options: WorkDoneProgressOptions::default(),
                legend: SemanticTokensLegend {
                    token_types: semantic_token_legend(),
                    token_modifiers: Vec::new(),
                },
                range: Some(false),
                full: Some(SemanticTokensFullOptions::Bool(true)),
            },
        )),
        ..Default::default()
    }
}

/// How the server names itself in the handshake — the string clients print in
/// their "language server" status line, plus this crate's version.
#[must_use]
pub(super) fn server_info() -> ServerInfo {
    ServerInfo {
        name: "aozora-lsp".to_owned(),
        version: Some(env!("CARGO_PKG_VERSION").to_owned()),
    }
}
