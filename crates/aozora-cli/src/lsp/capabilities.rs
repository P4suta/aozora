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

#[must_use]
pub(super) fn server_capabilities() -> ServerCapabilities {
    ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(
            TextDocumentSyncKind::INCREMENTAL,
        )),
        document_formatting_provider: Some(OneOf::Left(true)),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        linked_editing_range_provider: Some(LinkedEditingRangeServerCapabilities::Simple(true)),
        rename_provider: Some(OneOf::Right(RenameOptions {
            prepare_provider: Some(true),
            work_done_progress_options: WorkDoneProgressOptions::default(),
        })),
        completion_provider: Some(CompletionOptions {
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
                "｜".to_owned(),
                "《".to_owned(),
                "※".to_owned(),
            ]),
            resolve_provider: Some(false),
            ..Default::default()
        }),
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

#[must_use]
pub(super) fn server_info() -> ServerInfo {
    ServerInfo {
        name: "aozora-lsp".to_owned(),
        version: Some(env!("CARGO_PKG_VERSION").to_owned()),
    }
}
