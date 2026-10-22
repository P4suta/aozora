use std::slice;
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use tokio::task::{spawn_blocking, yield_now};
use tokio::time::sleep;

use super::server_locale::{init_ui_lang, ui_lang};
use crate::lsp::capabilities::{server_capabilities, server_info};
use crate::lsp::code_actions::{quick_fix_actions, wrap_selection_actions};
use crate::lsp::commands::{COMMAND_CANONICALIZE_SLUG, canonicalize_slug_edit};
use crate::lsp::completion::completion_at;
use crate::lsp::diagnostics::diagnostics_from_aozora;
use crate::lsp::doc_line_view::DocLineView;
use crate::lsp::document_limit::{MAX_DOCUMENT_BYTES, exceeds_document_cap};
use crate::lsp::formatting::format_edits;
use crate::lsp::half_width_emmet::emmet_completions;
use crate::lsp::hover::hover_at;
use crate::lsp::linked_editing::linked_editing_at;
use crate::lsp::on_type_formatting::format_on_type;
use crate::lsp::rename::{prepare_rename_at, rename_edit};
use crate::lsp::state::OpenDocument;
use crate::lsp::structured_snippets::snippet_completions;
use crate::lsp::text_edit::ByteEdit;
use tower_lsp::jsonrpc::{Error as JsonRpcError, Result};
use tower_lsp::lsp_types::{
    CodeActionParams, CodeActionResponse, CompletionItem, CompletionParams, CompletionResponse,
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DocumentFormattingParams, DocumentOnTypeFormattingParams,
    DocumentSymbol, DocumentSymbolParams, DocumentSymbolResponse, ExecuteCommandParams,
    FoldingRange, FoldingRangeParams, Hover, HoverParams, InitializeParams, InitializeResult,
    InitializedParams, LinkedEditingRangeParams, LinkedEditingRanges, MessageType, Position,
    PrepareRenameResponse, Range, RenameParams, SemanticTokens, SemanticTokensParams,
    SemanticTokensResult, TextDocumentContentChangeEvent, TextDocumentPositionParams, TextEdit,
    Url, WorkspaceEdit,
};
use tower_lsp::{Client, LanguageServer};

use crate::i18n::{FluentArgs, LanguageIdentifier, tf};
use crate::lsp::document_symbol::document_symbols;
use crate::lsp::folding_range::folding_ranges;
use crate::lsp::position::{byte_offset_to_position, position_to_byte_offset};
use crate::lsp::semantic_tokens::semantic_tokens_full;
use aozora::resolve_gaiji;

#[derive(Debug, Clone)]
pub(crate) struct AozoraLanguageServer {
    client: Client,
    docs: Arc<DashMap<Url, Arc<OpenDocument>>>,
}

const PUBLISH_DEBOUNCE_MS: u64 = 150;

const fn as_mib(bytes: usize) -> usize {
    bytes / (1024 * 1024)
}

fn oversize_args(byte_len: usize) -> FluentArgs<'static> {
    let mut args = FluentArgs::new();
    args.set("size", as_mib(byte_len).to_string());
    args.set("limit", as_mib(MAX_DOCUMENT_BYTES).to_string());
    args
}

fn oversize_notice(byte_len: usize, lang: &LanguageIdentifier) -> Diagnostic {
    Diagnostic {
        severity: Some(DiagnosticSeverity::INFORMATION),
        source: Some("aozora-lsp".to_owned()),
        message: tf(lang, "lsp-oversize-diagnostic", &oversize_args(byte_len)),
        ..Default::default()
    }
}

fn oversize_html_notice(byte_len: usize, lang: &LanguageIdentifier) -> String {
    format!(
        "<p>{}</p>",
        tf(lang, "lsp-oversize-preview", &oversize_args(byte_len)),
    )
}

impl AozoraLanguageServer {
    #[must_use]
    pub(crate) fn new(client: Client) -> Self {
        Self {
            client,
            docs: Arc::new(DashMap::new()),
        }
    }

    async fn publish(&self, uri: Url) {
        let diags = self.lookup(&uri).map_or_else(Vec::new, |state| {
            let snap = state.snapshot();
            let rope = snap.doc_rope();
            if exceeds_document_cap(rope.len_bytes()) {
                return vec![oversize_notice(rope.len_bytes(), ui_lang())];
            }
            let view = DocLineView::Rope(rope);
            diagnostics_from_aozora(&view, snap.core().diagnostics(), ui_lang())
        });
        self.client.publish_diagnostics(uri, diags, None).await;
    }

    fn schedule_publish_debounced(&self, uri: Url) {
        let Some(state) = self.lookup(&uri) else {
            return;
        };
        let target_version = state.edit_version();
        let backend = self.clone();
        let task = tokio::spawn(async move {
            sleep(Duration::from_millis(PUBLISH_DEBOUNCE_MS)).await;
            backend.publish_if_current(uri, target_version).await;
        });
        state.replace_debounce_task(task.abort_handle());
    }

    async fn publish_if_current(&self, uri: Url, target_version: u64) {
        let Some(state) = self.lookup(&uri) else {
            return;
        };
        if state.edit_version() != target_version {
            return;
        }

        let publication_state = Arc::clone(&state);
        let Ok((raw, diagnostics, parsed_version)) =
            spawn_blocking(move || publication_state.publication_snapshot()).await
        else {
            return;
        };

        if state.edit_version() != parsed_version {
            return;
        }

        let publish_diags = if exceeds_document_cap(raw.len_bytes()) {
            vec![oversize_notice(raw.len_bytes(), ui_lang())]
        } else {
            diagnostics_from_aozora(&DocLineView::Rope(&raw), &diagnostics, ui_lang())
        };
        self.client
            .publish_diagnostics(uri, publish_diags, None)
            .await;
    }

    fn lookup(&self, uri: &Url) -> Option<Arc<OpenDocument>> {
        self.docs.get(uri).map(|entry| Arc::clone(&*entry))
    }

    pub(crate) async fn render_html(&self, params: RenderHtmlParams) -> Result<RenderHtmlResult> {
        let state = self
            .lookup(&params.uri)
            .ok_or_else(|| JsonRpcError::invalid_params("no document at uri"))?;
        let snapshot = state.snapshot();
        let text_len = snapshot.doc_text().len();
        if exceeds_document_cap(text_len) {
            return Ok(RenderHtmlResult {
                html: oversize_html_notice(text_len, ui_lang()),
                paused: true,
            });
        }
        let core = snapshot.core().clone();
        let html = spawn_blocking(move || core.to_html())
            .await
            .map_err(|join_err| {
                let mut err = JsonRpcError::internal_error();
                err.message = format!("renderHtml panicked: {join_err}").into();
                err
            })?;
        Ok(RenderHtmlResult {
            html,
            paused: false,
        })
    }

    pub(crate) async fn gaiji_spans(&self, params: GaijiSpansParams) -> Result<GaijiSpansResult> {
        yield_now().await;
        let state = self
            .lookup(&params.uri)
            .ok_or_else(|| JsonRpcError::invalid_params("no document at uri"))?;
        let snap = state.snapshot();
        let mut views = Vec::with_capacity(snap.doc_gaiji_spans().len());
        for span in snap.doc_gaiji_spans().values() {
            let Some(resolved) = resolve_gaiji(span.mencode.as_deref(), &span.description) else {
                continue;
            };
            let start = snap
                .doc_line_index()
                .position(snap.doc_text(), span.start_byte as usize);
            let end = snap
                .doc_line_index()
                .position(snap.doc_text(), span.end_byte as usize);
            views.push(GaijiSpanView {
                range: Range::new(start, end),
                resolved,
            });
        }
        Ok(GaijiSpansResult { spans: views })
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RenderHtmlParams {
    pub(crate) uri: Url,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct RenderHtmlResult {
    pub(crate) html: String,
    pub(crate) paused: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GaijiSpansParams {
    pub(crate) uri: Url,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct GaijiSpanView {
    pub(crate) range: Range,
    pub(crate) resolved: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct GaijiSpansResult {
    pub(crate) spans: Vec<GaijiSpanView>,
}

#[derive(serde::Deserialize)]
struct CanonicalizeArgs {
    uri: Url,
    version: i32,
    range: Range,
    body: String,
}

enum ValidatedLspChange {
    Edit(ByteEdit),
    Replace(String),
}

fn validate_lsp_changes(
    source: &str,
    changes: &[TextDocumentContentChangeEvent],
) -> Option<Vec<ValidatedLspChange>> {
    let mut working = source.to_owned();
    let mut validated = Vec::with_capacity(changes.len());
    for change in changes {
        if change.range.is_none() {
            working.clone_from(&change.text);
            u32::try_from(working.len()).ok()?;
            validated.push(ValidatedLspChange::Replace(change.text.clone()));
            continue;
        }
        let edit = lsp_change_to_edit(&working, change)?;
        working.replace_range(edit.range.clone(), &edit.new_text);
        u32::try_from(working.len()).ok()?;
        validated.push(ValidatedLspChange::Edit(edit));
    }
    Some(validated)
}

fn lsp_change_to_edit(source: &str, change: &TextDocumentContentChangeEvent) -> Option<ByteEdit> {
    let range = change.range?;
    let start = position_to_byte_offset(source, range.start)?;
    let end = position_to_byte_offset(source, range.end)?;
    if end < start {
        return None;
    }
    Some(ByteEdit::new(start..end, change.text.clone()))
}

fn exact_position_to_byte_offset(source: &str, position: Position) -> Option<usize> {
    let offset = position_to_byte_offset(source, position)?;
    (byte_offset_to_position(source, offset) == position).then_some(offset)
}

#[tower_lsp::async_trait]
impl LanguageServer for AozoraLanguageServer {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        init_ui_lang(params.locale.as_deref());
        Ok(InitializeResult {
            capabilities: server_capabilities(),
            server_info: Some(server_info()),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "aozora-lsp ready")
            .await;
    }

    #[tracing::instrument(skip_all, fields(uri = %p.text_document.uri, text_bytes = p.text_document.text.len()))]
    async fn did_open(&self, p: DidOpenTextDocumentParams) {
        let uri = p.text_document.uri;
        self.docs.insert(
            uri.clone(),
            OpenDocument::new_versioned(p.text_document.text, p.text_document.version),
        );
        self.publish(uri).await;
    }

    #[tracing::instrument(
        skip_all,
        fields(
            uri = %p.text_document.uri,
            version = p.text_document.version,
            change_count = p.content_changes.len(),
        ),
    )]
    async fn did_change(&self, p: DidChangeTextDocumentParams) {
        let uri = p.text_document.uri;
        let Some(state) = self.lookup(&uri) else {
            return;
        };
        let update = state.lock_lsp_update();
        if p.text_document.version <= state.lsp_version() {
            tracing::warn!(
                current_version = state.lsp_version(),
                received_version = p.text_document.version,
                "ignoring stale text document change",
            );
            return;
        }
        let snapshot = state.snapshot();
        let Some(changes) = validate_lsp_changes(snapshot.doc_text(), &p.content_changes) else {
            tracing::warn!("rejecting invalid text document change batch");
            drop(update);
            self.schedule_publish_debounced(uri);
            return;
        };
        for change in changes {
            match change {
                ValidatedLspChange::Edit(edit) => {
                    if state.apply_changes(slice::from_ref(&edit)).is_none() {
                        tracing::error!("validated text document edit was rejected by core");
                        return;
                    }
                }
                ValidatedLspChange::Replace(text) => {
                    state.replace_text(text);
                }
            }
        }
        state.set_lsp_version(p.text_document.version);
        drop(update);
        self.schedule_publish_debounced(uri);
    }

    #[tracing::instrument(skip_all, fields(uri = %p.text_document.uri))]
    async fn did_close(&self, p: DidCloseTextDocumentParams) {
        let uri = p.text_document.uri;
        if let Some(state) = self.lookup(&uri) {
            let snapshot = state.metrics.snapshot();
            tracing::info!(
                target: "aozora_cli::lsp::metrics",
                uri = %uri,
                ?snapshot,
                "doc lifecycle metrics",
            );
        }
        self.docs.remove(&uri);
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }

    #[tracing::instrument(skip_all, fields(uri = %p.text_document.uri))]
    async fn formatting(&self, p: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let uri = p.text_document.uri;
        let Some(state) = self.lookup(&uri) else {
            return Ok(None);
        };
        let text = state.snapshot().doc_text().to_owned();
        let edits = spawn_blocking(move || format_edits(&text))
            .await
            .map_err(|join_err| {
                let mut err = JsonRpcError::internal_error();
                err.message = format!("formatting panicked: {join_err}").into();
                err
            })?;
        Ok(Some(edits))
    }

    #[tracing::instrument(
        skip_all,
        fields(
            uri = %p.text_document_position.text_document.uri,
            ch = %p.ch,
        ),
    )]
    async fn on_type_formatting(
        &self,
        p: DocumentOnTypeFormattingParams,
    ) -> Result<Option<Vec<TextEdit>>> {
        let uri = p.text_document_position.text_document.uri;
        let position = p.text_document_position.position;
        let Some(state) = self.lookup(&uri) else {
            return Ok(None);
        };
        let snap = state.snapshot();
        let edits = format_on_type(snap.doc_text(), position, &p.ch);
        if edits.is_empty() {
            Ok(None)
        } else {
            Ok(Some(edits))
        }
    }

    #[tracing::instrument(
        skip_all,
        fields(
            uri = %p.text_document_position_params.text_document.uri,
            line = p.text_document_position_params.position.line,
            character = p.text_document_position_params.position.character,
        ),
    )]
    async fn hover(&self, p: HoverParams) -> Result<Option<Hover>> {
        let uri = p.text_document_position_params.text_document.uri;
        let position = p.text_document_position_params.position;
        let Some(state) = self.lookup(&uri) else {
            return Ok(None);
        };
        let snap = state.snapshot();
        Ok(hover_at(snap.core(), position, ui_lang()))
    }

    #[tracing::instrument(
        skip_all,
        fields(
            uri = %p.text_document_position_params.text_document.uri,
            line = p.text_document_position_params.position.line,
            character = p.text_document_position_params.position.character,
        ),
    )]
    async fn linked_editing_range(
        &self,
        p: LinkedEditingRangeParams,
    ) -> Result<Option<LinkedEditingRanges>> {
        let uri = p.text_document_position_params.text_document.uri;
        let position = p.text_document_position_params.position;
        let Some(state) = self.lookup(&uri) else {
            return Ok(None);
        };
        let snap = state.snapshot();
        Ok(linked_editing_at(
            snap.doc_text(),
            snap.doc_line_index(),
            position,
        ))
    }

    #[tracing::instrument(
        skip_all,
        fields(
            uri = %p.text_document.uri,
            line = p.position.line,
            character = p.position.character,
        ),
    )]
    async fn prepare_rename(
        &self,
        p: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        let uri = p.text_document.uri;
        let position = p.position;
        let Some(state) = self.lookup(&uri) else {
            return Ok(None);
        };
        let snap = state.snapshot();
        Ok(prepare_rename_at(
            snap.core(),
            snap.doc_text(),
            snap.doc_line_index(),
            position,
        ))
    }

    #[tracing::instrument(
        skip_all,
        fields(
            uri = %p.text_document_position.text_document.uri,
            line = p.text_document_position.position.line,
            character = p.text_document_position.position.character,
        ),
    )]
    async fn rename(&self, p: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let uri = p.text_document_position.text_document.uri;
        let position = p.text_document_position.position;
        let new_name = p.new_name;
        let Some(state) = self.lookup(&uri) else {
            return Ok(None);
        };
        let snap = state.snapshot();
        let outcome = rename_edit(
            snap.core(),
            snap.doc_text(),
            snap.doc_line_index(),
            &uri,
            position,
            &new_name,
        );
        match outcome {
            Ok(None) => Ok(None),
            Ok(Some(edit)) => Ok(Some(edit)),
            Err(e) => Err(JsonRpcError::invalid_params(e.to_string())),
        }
    }

    #[tracing::instrument(
        skip_all,
        fields(
            uri = %p.text_document_position.text_document.uri,
            line = p.text_document_position.position.line,
            character = p.text_document_position.position.character,
        ),
    )]
    async fn completion(&self, p: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = p.text_document_position.text_document.uri;
        let position = p.text_document_position.position;
        let Some(state) = self.lookup(&uri) else {
            return Ok(None);
        };
        let snap = state.snapshot();
        let lang = ui_lang();
        let mut items: Vec<CompletionItem> = completion_at(snap.doc_text(), position, lang);
        items.extend(emmet_completions(snap.doc_text(), position, lang));
        items.extend(snippet_completions(snap.doc_text(), position, lang));
        if items.is_empty() {
            Ok(None)
        } else {
            Ok(Some(CompletionResponse::Array(items)))
        }
    }

    #[tracing::instrument(skip_all, fields(uri = %p.text_document.uri))]
    async fn code_action(&self, p: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let uri = p.text_document.uri;
        let Some(state) = self.lookup(&uri) else {
            return Ok(None);
        };
        let lang = ui_lang();
        let mut actions = quick_fix_actions(&uri, &p.context.diagnostics, lang);
        let snap = state.snapshot();
        actions.extend(wrap_selection_actions(
            snap.doc_text(),
            snap.doc_line_index(),
            &uri,
            p.range,
            lang,
        ));
        if actions.is_empty() {
            Ok(None)
        } else {
            Ok(Some(actions))
        }
    }

    async fn execute_command(
        &self,
        params: ExecuteCommandParams,
    ) -> Result<Option<serde_json::Value>> {
        if params.command != COMMAND_CANONICALIZE_SLUG {
            return Err(JsonRpcError::method_not_found());
        }
        let arg = params
            .arguments
            .into_iter()
            .next()
            .ok_or_else(|| JsonRpcError::invalid_params("expected one argument object"))?;
        let CanonicalizeArgs {
            uri,
            version,
            range,
            body,
        } = serde_json::from_value(arg)
            .map_err(|err| JsonRpcError::invalid_params(err.to_string()))?;
        let state = self
            .lookup(&uri)
            .ok_or_else(|| JsonRpcError::invalid_params("no document at uri"))?;
        let update = state.lock_lsp_update();
        if state.lsp_version() != version {
            return Err(JsonRpcError::invalid_params(
                "document version changed; retry canonicalization",
            ));
        }
        let snapshot = state.snapshot();
        let start = exact_position_to_byte_offset(snapshot.doc_text(), range.start)
            .ok_or_else(|| JsonRpcError::invalid_params("invalid canonicalization range"))?;
        let end = exact_position_to_byte_offset(snapshot.doc_text(), range.end)
            .ok_or_else(|| JsonRpcError::invalid_params("invalid canonicalization range"))?;
        let selected = snapshot
            .doc_text()
            .get(start..end)
            .ok_or_else(|| JsonRpcError::invalid_params("invalid canonicalization range"))?;
        if selected != body {
            return Err(JsonRpcError::invalid_params(
                "selected text changed; retry canonicalization",
            ));
        }
        let workspace_edit = canonicalize_slug_edit(uri, version, range, selected);
        drop(update);
        let Some(workspace_edit) = workspace_edit else {
            return Ok(None);
        };
        let response = self
            .client
            .apply_edit(workspace_edit)
            .await
            .map_err(|err| {
                let mut rpc_error = JsonRpcError::internal_error();
                rpc_error.message = format!("applyEdit failed: {err}").into();
                rpc_error
            })?;
        if !response.applied {
            return Err(JsonRpcError::invalid_params(
                response
                    .failure_reason
                    .unwrap_or_else(|| "client rejected versioned workspace edit".to_owned()),
            ));
        }
        Ok(None)
    }

    #[tracing::instrument(skip_all, fields(uri = %p.text_document.uri))]
    async fn folding_range(&self, p: FoldingRangeParams) -> Result<Option<Vec<FoldingRange>>> {
        let uri = p.text_document.uri;
        let Some(state) = self.lookup(&uri) else {
            return Ok(None);
        };
        let snap = state.snapshot();
        let ranges = folding_ranges(snap.doc_text());
        if ranges.is_empty() {
            Ok(None)
        } else {
            Ok(Some(ranges))
        }
    }

    #[tracing::instrument(skip_all, fields(uri = %p.text_document.uri))]
    async fn document_symbol(
        &self,
        p: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = p.text_document.uri;
        let Some(state) = self.lookup(&uri) else {
            return Ok(None);
        };
        let snap = state.snapshot();
        let symbols: Vec<DocumentSymbol> = document_symbols(snap.doc_text(), ui_lang());
        if symbols.is_empty() {
            Ok(None)
        } else {
            Ok(Some(DocumentSymbolResponse::Nested(symbols)))
        }
    }

    #[tracing::instrument(skip_all, fields(uri = %p.text_document.uri))]
    async fn semantic_tokens_full(
        &self,
        p: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let uri = p.text_document.uri;
        let Some(state) = self.lookup(&uri) else {
            return Ok(None);
        };
        let snap = state.snapshot();
        let tokens: SemanticTokens = semantic_tokens_full(snap.core());
        Ok(Some(SemanticTokensResult::Tokens(tokens)))
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use tower_lsp::lsp_types::{Position, Range};

    fn en() -> LanguageIdentifier {
        "en".parse().expect("en parses")
    }

    fn synth_change(range: Option<Range>, text: &str) -> TextDocumentContentChangeEvent {
        TextDocumentContentChangeEvent {
            range,
            range_length: None,
            text: text.to_owned(),
        }
    }

    fn replay_lsp_changes(initial: &str, changes: &[TextDocumentContentChangeEvent]) -> String {
        let state = OpenDocument::new(initial.to_owned());
        for change in changes {
            let snap = state.snapshot();
            match lsp_change_to_edit(snap.doc_text(), change) {
                Some(edit) => {
                    _ = state.apply_changes(slice::from_ref(&edit));
                }
                None if change.range.is_none() => {
                    state.replace_text(change.text.clone());
                }
                None => {}
            }
        }
        state.snapshot().doc_text().to_owned()
    }

    #[test]
    fn lsp_change_to_edit_returns_none_for_full_replacement() {
        let change = synth_change(None, "new text");
        assert!(lsp_change_to_edit("anything", &change).is_none());
    }

    #[test]
    fn lsp_change_to_edit_handles_basic_range() {
        let source = "hello world";
        let change = synth_change(
            Some(Range::new(Position::new(0, 6), Position::new(0, 11))),
            "rust",
        );
        let edit = lsp_change_to_edit(source, &change).expect("convert");
        assert_eq!(edit.range, 6..11);
        assert_eq!(edit.new_text, "rust");
    }

    #[test]
    fn lsp_change_to_edit_handles_multibyte_position() {
        let source = "あいう";
        let change = synth_change(
            Some(Range::new(Position::new(0, 1), Position::new(0, 2))),
            "X",
        );
        let edit = lsp_change_to_edit(source, &change).expect("convert");
        assert_eq!(edit.range, 3..6);
        assert_eq!(edit.new_text, "X");
    }

    #[test]
    fn lsp_change_to_edit_rejects_inverted_range() {
        let change = synth_change(
            Some(Range::new(Position::new(0, 5), Position::new(0, 2))),
            "x",
        );
        assert!(lsp_change_to_edit("hello world", &change).is_none());
    }

    #[test]
    fn lsp_change_to_edit_handles_pure_deletion() {
        let source = "abcdef";
        let change = synth_change(
            Some(Range::new(Position::new(0, 2), Position::new(0, 4))),
            "",
        );
        let edit = lsp_change_to_edit(source, &change).expect("convert");
        assert_eq!(edit.range, 2..4);
        assert_eq!(edit.new_text, "");
    }

    #[test]
    fn doc_state_new_populates_snapshot() {
        let state = OpenDocument::new("hello".to_owned());
        let snapshot = state.snapshot();
        assert!(snapshot.core().diagnostics().is_empty());
        assert_eq!(snapshot.doc_text(), "hello");
    }

    #[test]
    fn doc_state_apply_changes_updates_text() {
        let state = OpenDocument::new("hello world".to_owned());
        let edit = ByteEdit::new(6..11, "rust".to_owned());
        state.apply_changes(&[edit]);
        assert_eq!(state.snapshot().doc_text(), "hello rust");
    }

    #[test]
    fn doc_state_apply_changes_rejects_invalid_edit_keeps_text() {
        let state = OpenDocument::new("hi".to_owned());
        let edit = ByteEdit::new(0..99, "x".to_owned());
        let result = state.apply_changes(&[edit]);
        assert!(result.is_none(), "out-of-bounds edit must be rejected");
        assert_eq!(state.snapshot().doc_text(), "hi");
    }

    #[test]
    fn doc_state_apply_changes_rejects_non_char_boundary_edit() {
        let state = OpenDocument::new("あ".to_owned());
        let edit = ByteEdit::new(1..2, "x".to_owned());
        let result = state.apply_changes(&[edit]);
        assert!(result.is_none(), "cross-boundary edit must be rejected");
        assert_eq!(
            state.snapshot().doc_text(),
            "あ",
            "non-boundary edit must be rejected",
        );
    }

    #[test]
    fn doc_state_replace_text_updates_buffer() {
        let state = OpenDocument::new("hello".to_owned());
        state.replace_text("｜青梅《おうめ》".to_owned());
        assert_eq!(state.snapshot().doc_text(), "｜青梅《おうめ》");
    }

    #[test]
    fn two_events_in_one_batch_apply_in_source_order() {
        let initial = "abcdef";
        let changes = vec![
            synth_change(
                Some(Range::new(Position::new(0, 1), Position::new(0, 2))),
                "BB",
            ),
            synth_change(
                Some(Range::new(Position::new(0, 4), Position::new(0, 5))),
                "DD",
            ),
        ];
        let final_text = replay_lsp_changes(initial, &changes);
        assert_eq!(final_text, "aBBcDDef");
    }

    #[test]
    fn full_replacement_after_incremental_in_same_batch_wins() {
        let initial = "old text";
        let changes = vec![
            synth_change(
                Some(Range::new(Position::new(0, 0), Position::new(0, 0))),
                "PREFIX:",
            ),
            synth_change(None, "FRESH"),
        ];
        let final_text = replay_lsp_changes(initial, &changes);
        assert_eq!(final_text, "FRESH");
    }

    #[test]
    fn edit_inserting_aozora_trigger_reparses() {
        let state = OpenDocument::new("plain text".to_owned());
        let edit = ByteEdit::new(5..6, "｜青梅《おうめ》".to_owned());
        state.apply_changes(&[edit]);
        let snapshot = state.snapshot();
        assert_eq!(snapshot.core().nodes().len(), 1);
        assert!(snapshot.core().diagnostics().is_empty());
    }

    #[test]
    fn pua_collision_edit_surfaces_diagnostic() {
        let state = OpenDocument::new("plain".to_owned());
        let edit = ByteEdit::new(0..0, "\u{E001}".to_owned());
        state.apply_changes(&[edit]);
        let snapshot = state.snapshot();
        assert!(
            !snapshot.core().diagnostics().is_empty(),
            "PUA injection must produce diagnostics; got {:?}",
            snapshot.core().diagnostics(),
        );
    }

    #[test]
    fn sequence_of_incremental_edits_converges_to_full_text() {
        let state = OpenDocument::new(String::new());
        for (i, ch) in "hello world".chars().enumerate() {
            let edit = ByteEdit::new(i..i, ch.to_string());
            state.apply_changes(&[edit]);
        }
        assert_eq!(state.snapshot().doc_text(), "hello world");
    }

    fn replay_lsp_changes_with_sync_rebuild(
        initial: &str,
        changes: &[TextDocumentContentChangeEvent],
    ) -> String {
        let state = OpenDocument::new(initial.to_owned());
        for change in changes {
            let snap = state.snapshot();
            match lsp_change_to_edit(snap.doc_text(), change) {
                Some(edit) => {
                    _ = state.apply_changes(slice::from_ref(&edit));
                }
                None if change.range.is_none() => {
                    state.replace_text(change.text.clone());
                }
                None => {}
            }
            state.rebuild_snapshot_now();
        }
        state.snapshot().doc_text().to_owned()
    }

    #[test]
    fn multi_change_batch_resolves_against_post_prior_change_text() {
        let initial = "abc";
        let changes = vec![
            synth_change(
                Some(Range::new(Position::new(0, 0), Position::new(0, 0))),
                "X",
            ),
            synth_change(
                Some(Range::new(Position::new(0, 4), Position::new(0, 4))),
                "Y",
            ),
        ];
        let final_text = replay_lsp_changes_with_sync_rebuild(initial, &changes);
        assert_eq!(final_text, "XabcY");
    }

    #[test]
    fn multi_change_batch_dependent_offsets_round_trip_via_doc_state() {
        let initial = "本文";
        let changes = vec![
            synth_change(
                Some(Range::new(Position::new(0, 0), Position::new(0, 0))),
                "｜",
            ),
            synth_change(
                Some(Range::new(Position::new(0, 1), Position::new(0, 1))),
                "X",
            ),
        ];
        let final_text = replay_lsp_changes_with_sync_rebuild(initial, &changes);
        assert_eq!(final_text, "｜X本文");
    }

    #[test]
    fn single_change_batch_does_not_need_in_batch_rebuild() {
        let initial = "abc";
        let changes = vec![synth_change(
            Some(Range::new(Position::new(0, 1), Position::new(0, 2))),
            "X",
        )];
        let with_rebuild = replay_lsp_changes_with_sync_rebuild(initial, &changes);
        let no_rebuild = replay_lsp_changes(initial, &changes);
        assert_eq!(with_rebuild, no_rebuild);
        assert_eq!(with_rebuild, "aXc");
    }

    #[test]
    fn as_mib_is_whole_mebibyte_floor_division() {
        assert_eq!(as_mib(2 * 1024 * 1024), 2);
        assert_eq!(as_mib(48 * 1024 * 1024), 48);
        assert_eq!(as_mib(1024 * 1024 - 1), 0);
        assert_eq!(as_mib(1024 * 1024), 1);
    }

    #[test]
    fn oversize_notice_pins_source_severity_and_message() {
        let diag = oversize_notice(20 * 1024 * 1024, &en());
        assert_eq!(diag.source.as_deref(), Some("aozora-lsp"));
        assert!(
            diag.message.contains("20 MiB") && diag.message.contains("16 MiB"),
            "message must quote the document size and the limit: {}",
            diag.message,
        );
        assert_eq!(diag.severity, Some(DiagnosticSeverity::INFORMATION));
        assert_eq!(
            diag.range,
            Range::new(Position::new(0, 0), Position::new(0, 0)),
        );
    }

    #[test]
    fn oversize_messages_resolve_in_every_locale() {
        for tag in ["en", "ja", "zh"] {
            let lang: LanguageIdentifier = tag.parse().expect("test locale tag parses");
            for text in [
                oversize_notice(20 * 1024 * 1024, &lang).message,
                oversize_html_notice(20 * 1024 * 1024, &lang),
            ] {
                assert!(
                    text.contains("20") && text.contains("16"),
                    "{tag}: size and limit must be bound: {text}",
                );
                assert!(!text.contains('{'), "{tag}: unbound placeable: {text}");
            }
        }
    }

    #[test]
    fn oversize_html_notice_is_one_inert_paragraph() {
        let html = oversize_html_notice(20 * 1024 * 1024, &en());
        assert!(html.starts_with("<p>") && html.ends_with("</p>"), "{html}");
        assert!(!html[3..html.len() - 4].contains('<'), "{html}");
    }
}

#[cfg(test)]
mod e2e {

    use super::*;

    use std::future::poll_fn;
    use std::time::Duration as StdDuration;

    use futures::{SinkExt, StreamExt};
    use parking_lot::Mutex;
    use serde_json::{Value, json};
    use tower::Service;
    use tower_lsp::LspService;
    use tower_lsp::jsonrpc::{ErrorCode, Request, Response};

    const URI: &str = "file:///doc.aozora";

    struct TestServer {
        service: LspService<AozoraLanguageServer>,
        outbound: Arc<Mutex<Vec<Request>>>,
        next_id: i64,
    }

    impl TestServer {
        fn new() -> Self {
            let (service, socket) = super::super::build_service();
            let outbound = Arc::new(Mutex::new(Vec::new()));
            let collected = Arc::clone(&outbound);
            let (mut requests, mut responses) = socket.split();
            tokio::spawn(async move {
                while let Some(req) = requests.next().await {
                    let id = req.id().cloned();
                    collected.lock().push(req);
                    if let Some(id) = id {
                        let resp = Response::from_parts(id, Ok(json!({ "applied": true })));
                        _ = responses.send(resp).await;
                    }
                }
            });
            Self {
                service,
                outbound,
                next_id: 0,
            }
        }

        async fn call(&mut self, req: Request) -> Option<Response> {
            poll_fn(|cx| self.service.poll_ready(cx))
                .await
                .expect("service ready");
            self.service.call(req).await.expect("service call")
        }

        async fn try_request(&mut self, method: &'static str, params: Value) -> Result<Value> {
            self.next_id += 1;
            let mut builder = Request::build(method).id(self.next_id);
            if !params.is_null() {
                builder = builder.params(params);
            }
            let resp = self
                .call(builder.finish())
                .await
                .expect("request yields a response");
            resp.into_parts().1
        }

        async fn request(&mut self, method: &'static str, params: Value) -> Value {
            self.try_request(method, params)
                .await
                .expect("request result is Ok")
        }

        async fn notify(&mut self, method: &'static str, params: Value) {
            let mut builder = Request::build(method);
            if !params.is_null() {
                builder = builder.params(params);
            }
            assert!(
                self.call(builder.finish()).await.is_none(),
                "notification must not yield a response",
            );
        }

        async fn handshake(&mut self) -> Value {
            let caps = self
                .request("initialize", json!({ "capabilities": {}, "locale": "en" }))
                .await;
            self.notify("initialized", json!({})).await;
            caps
        }

        async fn did_open(&mut self, text: &str) {
            self.notify(
                "textDocument/didOpen",
                json!({
                    "textDocument": {
                        "uri": URI,
                        "languageId": "aozora",
                        "version": 1,
                        "text": text,
                    }
                }),
            )
            .await;
        }

        #[expect(
            clippy::future_not_send,
            reason = "test-only harness driven by block_on in #[tokio::test]; never spawned, so Send is not required"
        )]
        async fn wait_until<T>(&self, f: impl Fn(&[Request]) -> Option<T>) -> T {
            for _ in 0..400 {
                let found = f(&self.outbound.lock());
                if let Some(found) = found {
                    return found;
                }
                sleep(StdDuration::from_millis(5)).await;
            }
            let seen: Vec<String> = self
                .outbound
                .lock()
                .iter()
                .map(|r| r.method().to_owned())
                .collect();
            panic!("timed out waiting on outbound traffic; saw: {seen:?}");
        }

        fn outbound_count(&self, method: &str) -> usize {
            self.outbound
                .lock()
                .iter()
                .filter(|r| r.method() == method)
                .count()
        }
    }

    const PUBLISH: &str = "textDocument/publishDiagnostics";

    fn published_diagnostics(req: &Request) -> Vec<Value> {
        req.params()
            .and_then(|p| p.get("diagnostics"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    }

    fn is_oversize_notice(diag: &Value) -> bool {
        diag["severity"] == json!(3) && diag["message"].as_str().is_some_and(|m| m.contains("MiB"))
    }

    async fn rendered_html(server: &mut TestServer) -> String {
        let resp = server
            .request("aozora/renderHtml", json!({ "uri": URI }))
            .await;
        resp["html"].as_str().unwrap_or_default().to_owned()
    }

    #[expect(
        clippy::future_not_send,
        reason = "test-only helper driven by block_on in #[tokio::test]; never spawned, so Send is not required"
    )]
    async fn nth_publish(server: &TestServer, n: usize) -> Request {
        server
            .wait_until(|reqs| {
                reqs.iter()
                    .filter(|r| r.method() == PUBLISH)
                    .nth(n - 1)
                    .cloned()
            })
            .await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn initialize_advertises_capabilities_and_server_info() {
        let mut server = TestServer::new();
        let caps = server.handshake().await;
        let advertised = &caps["capabilities"];
        assert_eq!(
            *advertised,
            serde_json::to_value(server_capabilities()).expect("capabilities serialize"),
        );

        assert_eq!(advertised["textDocumentSync"], json!(2));
        assert_eq!(advertised["documentFormattingProvider"], json!(true));
        assert_eq!(advertised["hoverProvider"], json!(true));
        assert_eq!(advertised["documentSymbolProvider"], json!(true));
        assert_eq!(advertised["foldingRangeProvider"], json!(true));
        assert_eq!(advertised["linkedEditingRangeProvider"], json!(true));
        assert_eq!(advertised["renameProvider"]["prepareProvider"], json!(true));
        assert!(
            advertised["completionProvider"]["triggerCharacters"].is_array(),
            "completion must advertise trigger characters: {caps}",
        );
        assert_eq!(
            advertised["completionProvider"]["resolveProvider"],
            json!(false),
        );
        assert!(
            advertised["documentOnTypeFormattingProvider"]["firstTriggerCharacter"].is_string(),
            "on-type formatting must advertise a first trigger character: {caps}",
        );
        assert_eq!(
            advertised["executeCommandProvider"]["commands"],
            json!([COMMAND_CANONICALIZE_SLUG]),
        );
        assert_eq!(
            advertised["codeActionProvider"]["codeActionKinds"],
            json!(["quickfix", "refactor.rewrite"]),
        );
        assert!(
            advertised["semanticTokensProvider"]["legend"]["tokenTypes"].is_array(),
            "semantic tokens must advertise a legend: {caps}",
        );
        assert!(
            advertised["inlayHintProvider"].is_null(),
            "inlay hints are deliberately not advertised: {caps}",
        );

        assert_eq!(
            caps["serverInfo"],
            serde_json::to_value(server_info()).expect("server info serializes"),
        );
        assert_eq!(caps["serverInfo"]["name"], json!("aozora-lsp"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn formatting_returns_edit_for_noncanonical_and_empty_when_canonical() {
        let mut server = TestServer::new();
        server.handshake().await;

        server.did_open("｜日本《にほん》").await;
        let edits = server
            .request(
                "textDocument/formatting",
                json!({
                    "textDocument": { "uri": URI },
                    "options": { "tabSize": 2, "insertSpaces": true },
                }),
            )
            .await;
        let edits = edits.as_array().expect("formatting yields an array");
        assert_eq!(edits.len(), 1, "non-canonical ruby produces one edit");
        assert!(
            edits[0]["newText"]
                .as_str()
                .is_some_and(|t| t.starts_with("日本《にほん》") && !t.contains('｜'))
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn hover_resolves_gaiji_and_returns_null_outside() {
        let mut server = TestServer::new();
        server.handshake().await;
        server
            .did_open("語※［＃「木＋吶のつくり」、第3水準1-85-54］で")
            .await;

        let hover = server
            .request(
                "textDocument/hover",
                json!({
                    "textDocument": { "uri": URI },
                    "position": { "line": 0, "character": 3 },
                }),
            )
            .await;
        let md = hover["contents"]["value"]
            .as_str()
            .expect("markdown hover body");
        assert!(md.contains('枘') || md.contains("6798"), "got: {md}");

        let outside = server
            .request(
                "textDocument/hover",
                json!({
                    "textDocument": { "uri": URI },
                    "position": { "line": 0, "character": 0 },
                }),
            )
            .await;
        assert!(outside.is_null(), "hover outside a gaiji is null");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn request_on_unopened_document_is_null() {
        let mut server = TestServer::new();
        server.handshake().await;
        let hover = server
            .request(
                "textDocument/hover",
                json!({
                    "textDocument": { "uri": "file:///missing.aozora" },
                    "position": { "line": 0, "character": 0 },
                }),
            )
            .await;
        assert!(hover.is_null());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn did_open_publishes_diagnostics_for_pua_collision() {
        let mut server = TestServer::new();
        server.handshake().await;
        server.did_open("oops\u{E001}here").await;

        let publish = server
            .wait_until(|reqs| reqs.iter().find(|r| r.method() == PUBLISH).cloned())
            .await;
        let diags = published_diagnostics(&publish);
        assert!(
            diags.iter().any(|d| d["severity"] == json!(2)),
            "expected a warning diagnostic, got: {diags:?}",
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn did_change_applies_edit_and_republishes() {
        let mut server = TestServer::new();
        server.handshake().await;
        server.did_open("abc").await;
        server
            .wait_until(|reqs| (reqs.iter().any(|r| r.method() == PUBLISH)).then_some(()))
            .await;

        server
            .notify(
                "textDocument/didChange",
                json!({
                    "textDocument": { "uri": URI, "version": 2 },
                    "contentChanges": [{
                        "range": {
                            "start": { "line": 0, "character": 3 },
                            "end":   { "line": 0, "character": 3 },
                        },
                        "text": "d",
                    }],
                }),
            )
            .await;

        server
            .wait_until(|reqs| {
                (reqs.iter().filter(|r| r.method() == PUBLISH).count() >= 2).then_some(())
            })
            .await;

        let edits = server
            .request(
                "textDocument/formatting",
                json!({
                    "textDocument": { "uri": URI },
                    "options": { "tabSize": 2, "insertSpaces": true },
                }),
            )
            .await;
        assert_eq!(edits.as_array().map(Vec::len), Some(0));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn did_close_clears_diagnostics() {
        let mut server = TestServer::new();
        server.handshake().await;
        server.did_open("oops\u{E001}here").await;
        server
            .wait_until(|reqs| reqs.iter().find(|r| r.method() == PUBLISH).cloned())
            .await;
        let before = server.outbound_count(PUBLISH);

        server
            .notify(
                "textDocument/didClose",
                json!({ "textDocument": { "uri": URI } }),
            )
            .await;

        let cleared = server
            .wait_until(|reqs| {
                reqs.iter()
                    .filter(|r| r.method() == PUBLISH)
                    .nth(before)
                    .cloned()
            })
            .await;
        assert_eq!(published_diagnostics(&cleared), Vec::<Value>::new());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn completion_handler_returns_items_at_trigger() {
        let mut server = TestServer::new();
        server.handshake().await;
        server.did_open("｜青空《あおぞら》").await;
        let resp = server
            .request(
                "textDocument/completion",
                json!({
                    "textDocument": { "uri": URI },
                    "position": { "line": 0, "character": 1 },
                }),
            )
            .await;
        assert!(resp.is_array() || resp.is_object() || resp.is_null());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn on_type_formatting_converts_half_width_bracket() {
        let mut server = TestServer::new();
        server.handshake().await;
        server.did_open("[").await;
        let resp = server
            .request(
                "textDocument/onTypeFormatting",
                json!({
                    "textDocument": { "uri": URI },
                    "position": { "line": 0, "character": 1 },
                    "ch": "[",
                    "options": { "tabSize": 2, "insertSpaces": true },
                }),
            )
            .await;
        let edits = resp.as_array().expect("onType yields edits");
        assert!(
            edits.iter().any(|e| e["newText"].as_str() == Some("［")),
            "expected a full-width bracket edit, got: {resp}",
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn linked_editing_document_symbol_folding_semantic_tokens_drive() {
        let mut server = TestServer::new();
        server.handshake().await;
        server
            .did_open("｜青空《あおぞら》\n［＃大見出し］序章\n本文")
            .await;

        let linked = server
            .request(
                "textDocument/linkedEditingRange",
                json!({
                    "textDocument": { "uri": URI },
                    "position": { "line": 0, "character": 4 },
                }),
            )
            .await;
        assert!(linked.is_object() || linked.is_null());

        let symbols = server
            .request(
                "textDocument/documentSymbol",
                json!({ "textDocument": { "uri": URI } }),
            )
            .await;
        assert!(symbols.is_array() || symbols.is_null());

        let folding = server
            .request(
                "textDocument/foldingRange",
                json!({ "textDocument": { "uri": URI } }),
            )
            .await;
        assert!(folding.is_array() || folding.is_null());

        let tokens = server
            .request(
                "textDocument/semanticTokens/full",
                json!({ "textDocument": { "uri": URI } }),
            )
            .await;
        assert!(
            tokens["data"].is_array(),
            "semanticTokens/full returns a data array, got: {tokens}",
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rename_provider_prepare_and_apply_drive() {
        let mut server = TestServer::new();
        let caps = server.handshake().await;
        assert_eq!(
            caps["capabilities"]["renameProvider"]["prepareProvider"],
            json!(true),
            "renameProvider must advertise prepareProvider: {caps}",
        );

        server
            .did_open("前\n［＃ここから2字下げ］\n本文\n［＃ここで字下げ終わり］\n後")
            .await;

        let prep = server
            .request(
                "textDocument/prepareRename",
                json!({
                    "textDocument": { "uri": URI },
                    "position": { "line": 1, "character": 0 },
                }),
            )
            .await;
        assert!(
            prep["placeholder"]
                .as_str()
                .is_some_and(|s| s.contains("ここから")),
            "prepareRename on the open marker must return a placeholder, got: {prep}",
        );

        let prep_plain = server
            .request(
                "textDocument/prepareRename",
                json!({
                    "textDocument": { "uri": URI },
                    "position": { "line": 0, "character": 0 },
                }),
            )
            .await;
        assert!(
            prep_plain.is_null(),
            "plain text is not renameable: {prep_plain}"
        );

        let rename = server
            .request(
                "textDocument/rename",
                json!({
                    "textDocument": { "uri": URI },
                    "position": { "line": 1, "character": 0 },
                    "newName": "［＃ここから罫囲み］",
                }),
            )
            .await;
        let texts: Vec<String> = rename["changes"]
            .as_object()
            .and_then(|m| m.values().next())
            .and_then(|edits| edits.as_array())
            .map(|edits| {
                edits
                    .iter()
                    .filter_map(|e| e["newText"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            texts.iter().any(|t| t.contains("罫囲み終わり")),
            "rename must rewrite the close marker to 罫囲み終わり, got: {rename}",
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn code_action_returns_wrap_actions_for_selection() {
        let mut server = TestServer::new();
        server.handshake().await;
        server.did_open("青空文庫").await;
        let resp = server
            .request(
                "textDocument/codeAction",
                json!({
                    "textDocument": { "uri": URI },
                    "range": {
                        "start": { "line": 0, "character": 0 },
                        "end":   { "line": 0, "character": 2 },
                    },
                    "context": { "diagnostics": [] },
                }),
            )
            .await;
        assert!(resp.is_array() || resp.is_null());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn execute_command_canonicalize_applies_workspace_edit() {
        let mut server = TestServer::new();
        server.handshake().await;
        server.did_open("［＃ぼうてん］").await;

        let result = server
            .request(
                "workspace/executeCommand",
                json!({
                    "command": COMMAND_CANONICALIZE_SLUG,
                    "arguments": [{
                        "uri": URI,
                        "version": 1,
                        "range": {
                            "start": { "line": 0, "character": 0 },
                            "end":   { "line": 0, "character": 7 },
                        },
                        "body": "［＃ぼうてん］",
                    }],
                }),
            )
            .await;
        assert!(result.is_null(), "command returns null on success");
        let apply = server
            .wait_until(|reqs| {
                reqs.iter()
                    .find(|r| r.method() == "workspace/applyEdit")
                    .cloned()
            })
            .await;
        let new_text = apply.params().and_then(|p| {
            p["edit"]["documentChanges"]
                .get(0)
                .and_then(|change| change["edits"].get(0))
                .and_then(|e| e["newText"].as_str())
                .map(str::to_owned)
        });
        assert_eq!(new_text.as_deref(), Some("［＃傍点］"));
        assert_eq!(
            apply
                .params()
                .and_then(|p| p["edit"]["documentChanges"][0]["textDocument"]["version"].as_i64()),
            Some(1),
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn execute_command_already_canonical_is_noop() {
        let mut server = TestServer::new();
        server.handshake().await;
        server.did_open("［＃傍点］").await;
        let result = server
            .request(
                "workspace/executeCommand",
                json!({
                    "command": COMMAND_CANONICALIZE_SLUG,
                    "arguments": [{
                        "uri": URI,
                        "version": 1,
                        "range": {
                            "start": { "line": 0, "character": 0 },
                            "end":   { "line": 0, "character": 5 },
                        },
                        "body": "［＃傍点］",
                    }],
                }),
            )
            .await;
        assert!(result.is_null());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn execute_command_rejects_stale_document_version() {
        let mut server = TestServer::new();
        server.handshake().await;
        server.did_open("［＃ぼうてん］").await;
        let err = server
            .try_request(
                "workspace/executeCommand",
                json!({
                    "command": COMMAND_CANONICALIZE_SLUG,
                    "arguments": [{
                        "uri": URI,
                        "version": 2,
                        "range": {
                            "start": { "line": 0, "character": 0 },
                            "end":   { "line": 0, "character": 7 },
                        },
                        "body": "［＃ぼうてん］",
                    }],
                }),
            )
            .await
            .expect_err("stale command must error");
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert_eq!(server.outbound_count("workspace/applyEdit"), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn execute_command_rejects_changed_selection_body() {
        let mut server = TestServer::new();
        server.handshake().await;
        server.did_open("［＃ぼうてん］").await;
        let err = server
            .try_request(
                "workspace/executeCommand",
                json!({
                    "command": COMMAND_CANONICALIZE_SLUG,
                    "arguments": [{
                        "uri": URI,
                        "version": 1,
                        "range": {
                            "start": { "line": 0, "character": 0 },
                            "end":   { "line": 0, "character": 7 },
                        },
                        "body": "［＃傍点］",
                    }],
                }),
            )
            .await
            .expect_err("changed body must error");
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert_eq!(server.outbound_count("workspace/applyEdit"), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn execute_command_rejects_invalid_selection_range() {
        let mut server = TestServer::new();
        server.handshake().await;
        server.did_open("［＃ぼうてん］").await;
        let err = server
            .try_request(
                "workspace/executeCommand",
                json!({
                    "command": COMMAND_CANONICALIZE_SLUG,
                    "arguments": [{
                        "uri": URI,
                        "version": 1,
                        "range": {
                            "start": { "line": 0, "character": 0 },
                            "end":   { "line": 0, "character": 99 },
                        },
                        "body": "［＃ぼうてん］",
                    }],
                }),
            )
            .await
            .expect_err("invalid range must error");
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert_eq!(server.outbound_count("workspace/applyEdit"), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn execute_command_observes_the_version_after_a_change_batch() {
        let mut server = TestServer::new();
        server.handshake().await;
        server.did_open("a［＃ぼうてん］").await;
        server
            .notify(
                "textDocument/didChange",
                json!({
                    "textDocument": { "uri": URI, "version": 2 },
                    "contentChanges": [
                        {
                            "range": {
                                "start": { "line": 0, "character": 0 },
                                "end":   { "line": 0, "character": 0 },
                            },
                            "text": "x",
                        },
                        {
                            "range": {
                                "start": { "line": 0, "character": 1 },
                                "end":   { "line": 0, "character": 1 },
                            },
                            "text": "y",
                        },
                    ],
                }),
            )
            .await;
        let result = server
            .request(
                "workspace/executeCommand",
                json!({
                    "command": COMMAND_CANONICALIZE_SLUG,
                    "arguments": [{
                        "uri": URI,
                        "version": 2,
                        "range": {
                            "start": { "line": 0, "character": 3 },
                            "end":   { "line": 0, "character": 10 },
                        },
                        "body": "［＃ぼうてん］",
                    }],
                }),
            )
            .await;
        assert!(result.is_null());
        let apply = server
            .wait_until(|reqs| {
                reqs.iter()
                    .find(|request| request.method() == "workspace/applyEdit")
                    .cloned()
            })
            .await;
        assert_eq!(
            apply.params().and_then(
                |params| params["edit"]["documentChanges"][0]["textDocument"]["version"].as_i64()
            ),
            Some(2),
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stale_change_notification_cannot_roll_back_document_state() {
        let mut server = TestServer::new();
        server.handshake().await;
        server.did_open("old").await;
        server
            .notify(
                "textDocument/didChange",
                json!({
                    "textDocument": { "uri": URI, "version": 3 },
                    "contentChanges": [{ "text": "［＃ぼうてん］" }],
                }),
            )
            .await;
        server
            .notify(
                "textDocument/didChange",
                json!({
                    "textDocument": { "uri": URI, "version": 2 },
                    "contentChanges": [{ "text": "stale" }],
                }),
            )
            .await;
        let result = server
            .request(
                "workspace/executeCommand",
                json!({
                    "command": COMMAND_CANONICALIZE_SLUG,
                    "arguments": [{
                        "uri": URI,
                        "version": 3,
                        "range": {
                            "start": { "line": 0, "character": 0 },
                            "end":   { "line": 0, "character": 7 },
                        },
                        "body": "［＃ぼうてん］",
                    }],
                }),
            )
            .await;
        assert!(result.is_null());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn invalid_change_batch_leaves_source_and_version_unchanged() {
        let mut server = TestServer::new();
        server.handshake().await;
        server.did_open("［＃ぼうてん］").await;
        server
            .notify(
                "textDocument/didChange",
                json!({
                    "textDocument": { "uri": URI, "version": 2 },
                    "contentChanges": [
                        {
                            "range": {
                                "start": { "line": 0, "character": 0 },
                                "end":   { "line": 0, "character": 0 },
                            },
                            "text": "x",
                        },
                        {
                            "range": {
                                "start": { "line": 9, "character": 0 },
                                "end":   { "line": 9, "character": 0 },
                            },
                            "text": "y",
                        },
                    ],
                }),
            )
            .await;
        let result = server
            .request(
                "workspace/executeCommand",
                json!({
                    "command": COMMAND_CANONICALIZE_SLUG,
                    "arguments": [{
                        "uri": URI,
                        "version": 1,
                        "range": {
                            "start": { "line": 0, "character": 0 },
                            "end":   { "line": 0, "character": 7 },
                        },
                        "body": "［＃ぼうてん］",
                    }],
                }),
            )
            .await;
        assert!(result.is_null());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn execute_command_unknown_is_method_not_found() {
        let mut server = TestServer::new();
        server.handshake().await;
        let err = server
            .try_request(
                "workspace/executeCommand",
                json!({ "command": "aozora.nope", "arguments": [] }),
            )
            .await
            .expect_err("unknown command must error");
        assert_eq!(err.code, ErrorCode::MethodNotFound);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn execute_command_missing_argument_is_invalid_params() {
        let mut server = TestServer::new();
        server.handshake().await;
        let err = server
            .try_request(
                "workspace/executeCommand",
                json!({ "command": COMMAND_CANONICALIZE_SLUG, "arguments": [] }),
            )
            .await
            .expect_err("missing argument must error");
        assert_eq!(err.code, ErrorCode::InvalidParams);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn execute_command_malformed_argument_is_invalid_params() {
        let mut server = TestServer::new();
        server.handshake().await;
        let err = server
            .try_request(
                "workspace/executeCommand",
                json!({
                    "command": COMMAND_CANONICALIZE_SLUG,
                    "arguments": [{ "uri": URI }],
                }),
            )
            .await
            .expect_err("malformed argument must error");
        assert_eq!(err.code, ErrorCode::InvalidParams);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn execute_command_missing_version_is_invalid_params() {
        let mut server = TestServer::new();
        server.handshake().await;
        server.did_open("［＃ぼうてん］").await;
        let err = server
            .try_request(
                "workspace/executeCommand",
                json!({
                    "command": COMMAND_CANONICALIZE_SLUG,
                    "arguments": [{
                        "uri": URI,
                        "range": {
                            "start": { "line": 0, "character": 0 },
                            "end":   { "line": 0, "character": 7 },
                        },
                        "body": "［＃ぼうてん］",
                    }],
                }),
            )
            .await
            .expect_err("missing version must error");
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert_eq!(server.outbound_count("workspace/applyEdit"), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn render_html_returns_markup_and_errors_on_unopened() {
        let mut server = TestServer::new();
        server.handshake().await;
        server.did_open("｜青空《あおぞら》").await;

        let ok = server
            .request("aozora/renderHtml", json!({ "uri": URI }))
            .await;
        assert!(
            ok["html"]
                .as_str()
                .is_some_and(|h| h.contains("<ruby") || h.contains("あおぞら")),
            "expected ruby HTML, got: {ok}",
        );

        let err = server
            .try_request("aozora/renderHtml", json!({ "uri": "file:///nope.aozora" }))
            .await
            .expect_err("unopened uri must error");
        assert_eq!(err.code, ErrorCode::InvalidParams);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn gaiji_spans_returns_resolved_spans_and_errors_on_unopened() {
        let mut server = TestServer::new();
        server.handshake().await;
        server
            .did_open("語※［＃「木＋吶のつくり」、第3水準1-85-54］で")
            .await;

        let ok = server
            .request("aozora/gaijiSpans", json!({ "uri": URI }))
            .await;
        let spans = ok["spans"].as_array().expect("spans array");
        assert!(!spans.is_empty(), "the gaiji must surface as a span");

        let err = server
            .try_request("aozora/gaijiSpans", json!({ "uri": "file:///nope.aozora" }))
            .await
            .expect_err("unopened uri must error");
        assert_eq!(err.code, ErrorCode::InvalidParams);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn oversize_document_gets_notice_not_analysis() {
        let mut server = TestServer::new();
        server.handshake().await;
        let huge = "a".repeat(MAX_DOCUMENT_BYTES + 1);
        server.did_open(&huge).await;

        let publish = server
            .wait_until(|reqs| reqs.iter().find(|r| r.method() == PUBLISH).cloned())
            .await;
        let diags = published_diagnostics(&publish);
        assert_eq!(diags.len(), 1, "oversize doc gets exactly one notice");
        assert_eq!(diags[0]["severity"], json!(3));

        let render = server
            .request("aozora/renderHtml", json!({ "uri": URI }))
            .await;
        assert_eq!(
            render["paused"],
            json!(true),
            "oversize render must flag the placeholder so the client does not \
             mistake it for the document: {render}",
        );
        assert!(
            render["html"]
                .as_str()
                .is_some_and(|h| h.starts_with("<p>")),
            "oversize render returns the inert notice, got: {render}",
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rendered_document_is_not_flagged_paused() {
        let mut server = TestServer::new();
        server.handshake().await;
        server.did_open("｜青空《あおぞら》").await;
        let render = server
            .request("aozora/renderHtml", json!({ "uri": URI }))
            .await;
        assert_eq!(render["paused"], json!(false), "{render}");
        assert!(
            render["html"]
                .as_str()
                .is_some_and(|h| h.contains("あおぞら")),
            "{render}",
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_succeeds() {
        let mut server = TestServer::new();
        server.handshake().await;
        let resp = server.request("shutdown", json!(null)).await;
        assert!(resp.is_null());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn initialized_logs_ready_message() {
        let mut server = TestServer::new();
        server.handshake().await;
        let log = server
            .wait_until(|reqs| {
                reqs.iter()
                    .find(|r| {
                        r.method() == "window/logMessage"
                            && r.params()
                                .and_then(|p| p.get("message"))
                                .and_then(Value::as_str)
                                == Some("aozora-lsp ready")
                    })
                    .cloned()
            })
            .await;
        assert_eq!(log.method(), "window/logMessage");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn did_change_full_replacement_replaces_buffer() {
        let mut server = TestServer::new();
        server.handshake().await;
        server.did_open("hello").await;
        nth_publish(&server, 1).await;

        server
            .notify(
                "textDocument/didChange",
                json!({
                    "textDocument": { "uri": URI, "version": 2 },
                    "contentChanges": [{ "text": "REPLACEDBODY" }],
                }),
            )
            .await;
        nth_publish(&server, 2).await;

        let body = rendered_html(&mut server).await;
        assert!(
            body.contains("REPLACEDBODY"),
            "range-less change must fully replace the buffer: {body}",
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn did_change_inverted_range_is_skipped_not_applied() {
        let mut server = TestServer::new();
        server.handshake().await;
        server.did_open("hello world").await;
        nth_publish(&server, 1).await;

        server
            .notify(
                "textDocument/didChange",
                json!({
                    "textDocument": { "uri": URI, "version": 2 },
                    "contentChanges": [{
                        "range": {
                            "start": { "line": 0, "character": 5 },
                            "end":   { "line": 0, "character": 2 },
                        },
                        "text": "ZZCLOBBERZZ",
                    }],
                }),
            )
            .await;
        nth_publish(&server, 2).await;

        let body = rendered_html(&mut server).await;
        assert!(
            !body.contains("ZZCLOBBERZZ"),
            "an unresolvable-range change must be dropped, not applied: {body}",
        );
        assert!(
            body.contains("hello"),
            "the original text must survive: {body}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn did_change_applies_a_batch_in_order() {
        let mut server = TestServer::new();
        server.handshake().await;
        server.did_open("abc").await;
        nth_publish(&server, 1).await;

        server
            .notify(
                "textDocument/didChange",
                json!({
                    "textDocument": { "uri": URI, "version": 2 },
                    "contentChanges": [
                        {
                            "range": {
                                "start": { "line": 0, "character": 0 },
                                "end":   { "line": 0, "character": 0 },
                            },
                            "text": "X",
                        },
                        {
                            "range": {
                                "start": { "line": 0, "character": 4 },
                                "end":   { "line": 0, "character": 4 },
                            },
                            "text": "Y",
                        },
                    ],
                }),
            )
            .await;
        nth_publish(&server, 2).await;

        let body = rendered_html(&mut server).await;
        assert!(
            body.contains("XabcY"),
            "change 1 must apply against post-change-0 text (expected XabcY): {body}",
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn linked_editing_returns_pair_on_ruby_delimiter() {
        let mut server = TestServer::new();
        server.handshake().await;
        server.did_open("｜青空《あおぞら》").await;
        let linked = server
            .request(
                "textDocument/linkedEditingRange",
                json!({
                    "textDocument": { "uri": URI },
                    "position": { "line": 0, "character": 3 },
                }),
            )
            .await;
        let ranges = linked["ranges"].as_array().expect("linked ranges array");
        assert_eq!(ranges.len(), 2, "open+close endpoints: {linked}");
        assert_eq!(ranges[1]["start"]["character"], json!(8), "{linked}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn completion_returns_emmet_item_on_half_width_bracket() {
        let mut server = TestServer::new();
        server.handshake().await;
        server.did_open("[").await;
        let resp = server
            .request(
                "textDocument/completion",
                json!({
                    "textDocument": { "uri": URI },
                    "position": { "line": 0, "character": 1 },
                }),
            )
            .await;
        let items = resp.as_array().expect("completion array response");
        assert!(!items.is_empty(), "expected completion items: {resp}");
        assert!(
            items.iter().any(|it| it["label"] == json!("［")),
            "expected the full-width bracket emmet item: {resp}",
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn folding_range_returns_exact_block_fold() {
        let mut server = TestServer::new();
        server.handshake().await;
        server
            .did_open("本文\n［＃ここから字下げ］\n字下げ内\n［＃ここで字下げ終わり］\n後")
            .await;
        let folding = server
            .request(
                "textDocument/foldingRange",
                json!({ "textDocument": { "uri": URI } }),
            )
            .await;
        let ranges = folding.as_array().expect("folding array");
        assert_eq!(ranges.len(), 1, "one block fold expected: {folding}");
        assert_eq!(ranges[0]["startLine"], json!(1), "{folding}");
        assert_eq!(ranges[0]["endLine"], json!(3), "{folding}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn document_symbol_returns_heading_symbol() {
        let mut server = TestServer::new();
        server.handshake().await;
        server.did_open("［＃「序章」は大見出し］\n本文").await;
        let symbols = server
            .request(
                "textDocument/documentSymbol",
                json!({ "textDocument": { "uri": URI } }),
            )
            .await;
        let arr = symbols.as_array().expect("nested document-symbol array");
        assert_eq!(arr.len(), 1, "one heading symbol expected: {symbols}");
        assert_eq!(arr[0]["name"], json!("序章"), "{symbols}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn over_limit_reparse_posts_oversize_notice() {
        let mut server = TestServer::new();
        server.handshake().await;
        let line: String = "a".repeat(63) + "\n";
        let mut big = line.repeat(MAX_DOCUMENT_BYTES / 64);
        big.push('a');
        assert_eq!(big.len(), MAX_DOCUMENT_BYTES + 1);
        server.did_open(&big).await;
        nth_publish(&server, 1).await;

        server
            .notify(
                "textDocument/didChange",
                json!({
                    "textDocument": { "uri": URI, "version": 2 },
                    "contentChanges": [{
                        "range": {
                            "start": { "line": 0, "character": 0 },
                            "end":   { "line": 0, "character": 1 },
                        },
                        "text": "b",
                    }],
                }),
            )
            .await;
        let second = nth_publish(&server, 2).await;
        let diags = published_diagnostics(&second);
        assert_eq!(
            diags.len(),
            1,
            "over-limit reparse posts exactly the oversize notice: {diags:?}",
        );
        assert!(
            is_oversize_notice(&diags[0]),
            "the sole diagnostic must be the oversize notice: {diags:?}",
        );
    }
}
