//! tower-lsp `LanguageServer` implementation for aozora documents.
//!
//! Each open document is an `Arc<OpenDocument>` in a [`DashMap`]: a
//! writer-side `DocBuffer` behind a `parking_lot::Mutex` and a
//! reader-side `DocSnapshot` in an `ArcSwap` (see [`crate::state`] for the
//! lock graph). Handlers read via `state.snapshot()` — a single atomic
//! load, wait-free, so the debounced reparse never blocks them.
//!
//! Text sync is incremental (see [`crate::capabilities`]): `did_change`
//! applies byte-range edits via `OpenDocument::apply_changes` and schedules
//! a debounced semantic re-parse + diagnostic publish on `spawn_blocking`,
//! so async hover / codeAction requests don't stall.

use std::slice;
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use tokio::task::{spawn_blocking, yield_now};
use tokio::time::sleep;

use crate::capabilities::{server_capabilities, server_info};
use crate::code_actions::{quick_fix_actions, wrap_selection_actions};
use crate::commands::{COMMAND_CANONICALIZE_SLUG, canonicalize_slug_edit};
use crate::completion::completion_at;
use crate::diagnostics::diagnostics_from_aozora;
use crate::doc_line_view::DocLineView;
use crate::formatting::format_edits;
use crate::half_width_emmet::emmet_completions;
use crate::hover::hover_at;
use crate::i18n::{init_ui_lang, ui_lang};
use crate::linked_editing::linked_editing_at;
use crate::on_type_formatting::format_on_type;
use crate::parse_cache::{MAX_DOCUMENT_BYTES, exceeds_document_cap};
use crate::rename::{prepare_rename_at, rename_edit};
use crate::state::OpenDocument;
use crate::structured_snippets::snippet_completions;
use crate::text_edit::ByteEdit;
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

use aozora_encoding::gaiji;
use aozora_i18n::{FluentArgs, LanguageIdentifier};

use crate::document_symbol::document_symbols;
use crate::folding_range::folding_ranges;
use crate::position::position_to_byte_offset;
use crate::semantic_tokens::semantic_tokens_full;

/// LSP backend for aozora documents.
///
/// `Clone` so the debounced publish task (`schedule_publish_debounced`)
/// can hold its own backend handle for the duration of the sleep
/// without keeping a borrow on the original. The fields are cheap to
/// clone — `Client` is a channel handle and `docs` is
/// `Arc<DashMap<...>>`.
#[derive(Debug, Clone)]
pub struct AozoraLanguageServer {
    client: Client,
    docs: Arc<DashMap<Url, Arc<OpenDocument>>>,
}

/// Quiet-period before the slow Rust parse + `publishDiagnostics`
/// runs. While the user is actively typing, every keystroke bumps
/// `parse_version`, and the debounced task at the tail end of the
/// burst is the only one that actually proceeds to parse — earlier
/// tasks see a stale version and exit.
const PUBLISH_DEBOUNCE_MS: u64 = 150;

/// Render a byte length as whole MiB for user-facing messages. Integer
/// division sidesteps the `clippy::cast_precision_loss` a float cast
/// would trip.
const fn as_mib(bytes: usize) -> usize {
    bytes / (1024 * 1024)
}

/// `$size` / `$limit` for the oversize catalog messages: the document's own
/// size and the cap it crossed, both in whole MiB.
fn oversize_args(byte_len: usize) -> FluentArgs<'static> {
    let mut args = FluentArgs::new();
    args.set("size", as_mib(byte_len).to_string());
    args.set("limit", as_mib(MAX_DOCUMENT_BYTES).to_string());
    args
}

/// The single informational diagnostic published for a document above
/// [`MAX_DOCUMENT_BYTES`], anchored at the start of the file.
fn oversize_notice(byte_len: usize, lang: &LanguageIdentifier) -> Diagnostic {
    Diagnostic {
        range: Range {
            start: Position::new(0, 0),
            end: Position::new(0, 0),
        },
        severity: Some(DiagnosticSeverity::INFORMATION),
        source: Some("aozora-lsp".to_owned()),
        message: aozora_i18n::tf(lang, "lsp-oversize-diagnostic", &oversize_args(byte_len)),
        ..Default::default()
    }
}

/// Inert HTML fragment returned by `aozora/renderHtml` for an oversized
/// document. The prose is the built-in catalog's and the only interpolations
/// are two integers — no document content reaches it, so it is safe in the
/// (script-free, strict-CSP) preview webview.
fn oversize_html_notice(byte_len: usize, lang: &LanguageIdentifier) -> String {
    format!(
        "<p>{}</p>",
        aozora_i18n::tf(lang, "lsp-oversize-preview", &oversize_args(byte_len)),
    )
}

impl AozoraLanguageServer {
    /// Build a new backend. Signature matches `LspService::new`'s
    /// `FnOnce(Client) -> AozoraLanguageServer` requirement, so users call this
    /// as `LspService::new(AozoraLanguageServer::new)`.
    #[must_use]
    pub fn new(client: Client) -> Self {
        Self {
            client,
            docs: Arc::new(DashMap::new()),
        }
    }

    async fn publish(&self, uri: Url) {
        let diags = self.lookup(&uri).map_or_else(Vec::new, |state| {
            let snap = state.snapshot();
            let rope = snap.doc_rope();
            // Oversized documents skip semantic analysis; surface a
            // single informational notice in place of (empty)
            // diagnostics so the absence of squiggles is explained.
            // (`len_bytes()` off the rope avoids materialising `doc_text()`
            // solely for the size check.)
            if exceeds_document_cap(rope.len_bytes()) {
                return vec![oversize_notice(rope.len_bytes(), ui_lang())];
            }
            // Map diagnostic spans against the doc-level rope (O(log n) line
            // lookups, byte-identical to a LineIndex over the raw text since
            // ropey counts only `\n`) — no per-publish line-table rebuild.
            let view = DocLineView::Rope(rope);
            state.with_parse_cache(|cache| {
                diagnostics_from_aozora(&view, cache.diagnostics(), ui_lang())
            })
        });
        self.client.publish_diagnostics(uri, diags, None).await;
    }

    /// Schedule a debounced semantic re-parse + diagnostic publish.
    /// The actual work runs after `PUBLISH_DEBOUNCE_MS` quiet time.
    /// Multiple rapid edits coalesce — only the task whose recorded
    /// `target_version` matches the doc's current `edit_version`
    /// after the sleep proceeds. Earlier tasks observe a newer
    /// version and exit silently, so a 100-keystroke burst still
    /// produces exactly one parse + one publish.
    fn schedule_publish_debounced(&self, uri: Url) {
        let Some(state) = self.lookup(&uri) else {
            return;
        };
        let target_version = state.edit_version();
        let backend = self.clone();
        let task = tokio::spawn(async move {
            sleep(Duration::from_millis(PUBLISH_DEBOUNCE_MS)).await;
            backend
                .reparse_and_publish_if_current(uri, target_version)
                .await;
        });
        // Cap in-flight debounce tasks at one per document: aborting the
        // previous pending task stops an edit flood from piling up
        // sleeping tasks. The version guard in the task body still
        // decides who actually publishes.
        state.replace_debounce_task(task.abort_handle());
    }

    /// The debounced task body — re-parse semantically through the
    /// parse cache then publish, but only if no newer edit has come in.
    async fn reparse_and_publish_if_current(&self, uri: Url, target_version: u64) {
        let Some(state) = self.lookup(&uri) else {
            return;
        };
        // Cheap early-out: a newer edit's own debounce task will publish.
        if state.edit_version() != target_version {
            return;
        }

        // Incremental reparse off the async runtime so concurrent hover /
        // codeAction requests do not stall on an executor thread.
        // `reparse_pending` drains the debounce-window edits under the
        // buffer lock and returns the exact text it parsed (so the LSP
        // position mapping is consistent) plus the edit-version that text
        // reflects.
        let parse_state = Arc::clone(&state);
        let Ok((raw, diagnostics, parsed_version)) =
            spawn_blocking(move || parse_state.reparse_pending()).await
        else {
            return;
        };

        // Publish guard (no store race — the cache is already updated):
        // if a newer edit landed during the parse, its task republishes,
        // so skip this now-superseded result rather than flash it.
        if state.edit_version() != parsed_version {
            return;
        }

        // Oversized documents skip semantic analysis inside
        // `reparse_pending` (empty diagnostics); surface the notice based
        // on the text that was actually parsed, not a possibly-stale
        // snapshot length.
        let publish_diags = if exceeds_document_cap(raw.len_bytes()) {
            vec![oversize_notice(raw.len_bytes(), ui_lang())]
        } else {
            // Map spans against the parsed rope directly — the per-keystroke
            // hot path no longer rebuilds an O(doc) line table (Mechanism A).
            diagnostics_from_aozora(&DocLineView::Rope(&raw), &diagnostics, ui_lang())
        };
        self.client
            .publish_diagnostics(uri, publish_diags, None)
            .await;
    }

    /// Lookup helper — returns an `Arc<OpenDocument>` clone so the caller
    /// can drop the dashmap shard reference immediately and operate
    /// on a wait-free snapshot. The dashmap shard read is microseconds;
    /// the Arc clone is a single atomic increment.
    fn lookup(&self, uri: &Url) -> Option<Arc<OpenDocument>> {
        self.docs.get(uri).map(|entry| Arc::clone(&*entry))
    }

    /// Custom LSP request `aozora/renderHtml`.
    ///
    /// Returns the document's HTML rendering (via `aozora`'s AST
    /// HTML renderer). The `VSCode` preview pane consumes this on
    /// every `did_change` (debounced) so the webview stays in
    /// lock-step with the editor buffer.
    ///
    /// Argument shape: `{ "uri": "file:///…" }`. Returns
    /// `{ "html": "<…>", "paused": false }` or an `invalid_params`
    /// error when no document is open at the URI.
    ///
    /// # Errors
    ///
    /// Returns [`JsonRpcError::invalid_params`] if no open document
    /// matches `params.uri`.
    pub(crate) async fn render_html(&self, params: RenderHtmlParams) -> Result<RenderHtmlResult> {
        // Wait-free snapshot — reads never contend with the writer
        // hot path. The Arc<str> clone is a single atomic bump.
        let state = self
            .lookup(&params.uri)
            .ok_or_else(|| JsonRpcError::invalid_params("no document at uri"))?;
        let text = state.snapshot().doc_text().to_string();
        // The preview drives the same O(n) renderer; skip it for
        // oversized documents and return a short inert notice instead.
        if exceeds_document_cap(text.len()) {
            return Ok(RenderHtmlResult {
                html: oversize_html_notice(text.len(), ui_lang()),
                paused: true,
            });
        }
        let html = spawn_blocking(move || {
            let document = aozora::Document::new(text);
            document.parse().to_html()
        })
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

    /// Custom LSP request `aozora/gaijiSpans`.
    ///
    /// Returns every resolvable `※［＃...］` gaiji span in the
    /// requested document, mapped to its resolved glyph and the
    /// LSP-coordinate range that the editor should fold over. The
    /// VS Code extension consumes this on every `did_change` to
    /// drive its inline-collapse decoration.
    ///
    /// Reads run lock-free against the pre-extracted
    /// [`crate::gaiji_spans::GaijiSpan`] list maintained by
    /// `OpenDocument`; no parser is invoked.
    ///
    /// # Errors
    /// Returns [`JsonRpcError::invalid_params`] if no document at
    /// `params.uri` is open.
    pub(crate) async fn gaiji_spans(&self, params: GaijiSpansParams) -> Result<GaijiSpansResult> {
        // tower-lsp's `custom_method` macro requires an async fn, but
        // the body is purely sync: the gaiji span list is pre-built
        // by `OpenDocument`, lookup is lock-free, no I/O happens. Make
        // the async signature *real* by yielding once to the tokio
        // runtime — that turns "fake async with `clippy::unused_async`"
        // into a genuine cooperative yield point, which is also what
        // a well-behaved LSP request handler should do anyway (lets
        // higher-priority tasks like `did_change` not starve when
        // many `gaiji_spans` requests pile up after a paste).
        yield_now().await;
        let state = self
            .lookup(&params.uri)
            .ok_or_else(|| JsonRpcError::invalid_params("no document at uri"))?;
        let snap = state.snapshot();
        let mut views = Vec::with_capacity(snap.doc_gaiji_spans().len());
        for span in snap.doc_gaiji_spans().values() {
            let Some(resolved) = gaiji::lookup(None, span.mencode.as_deref(), &span.description)
            else {
                continue;
            };
            let mut buf = String::with_capacity(8);
            _ = resolved.write_to(&mut buf);
            let start = snap
                .doc_line_index()
                .position(snap.doc_text(), span.start_byte as usize);
            let end = snap
                .doc_line_index()
                .position(snap.doc_text(), span.end_byte as usize);
            views.push(GaijiSpanView {
                range: Range::new(start, end),
                resolved: buf,
                description: span.description.to_string(),
                mencode: span.mencode.as_deref().map(str::to_owned),
            });
        }
        Ok(GaijiSpansResult { spans: views })
    }
}

/// Parameters for the `aozora/renderHtml` custom LSP request.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RenderHtmlParams {
    pub(crate) uri: Url,
}

/// Result for the `aozora/renderHtml` custom LSP request.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct RenderHtmlResult {
    pub(crate) html: String,
    /// True when `html` is the oversize placeholder rather than a rendering
    /// of the document. A client that treats the placeholder as the
    /// document's HTML silently loses the document — the VS Code
    /// extension's "Export HTML" would write the notice to disk and report
    /// success — so the distinction is on the wire, not left to prose
    /// sniffing.
    pub(crate) paused: bool,
}

/// Parameters for the `aozora/gaijiSpans` custom LSP request — the
/// VS Code extension polls this on every `did_change` to refresh
/// its inline-fold decorations. The extension swaps each
/// `※［＃...］` source span for its resolved character so the
/// reader sees clean prose; the source re-appears when the cursor
/// enters the span.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GaijiSpansParams {
    pub(crate) uri: Url,
}

/// One gaiji span exposed to the editor for visual collapse.
/// `range` is in LSP coordinates (line/UTF-16 column); `resolved`
/// is the rendered glyph (may be a single char or a 2-codepoint
/// combining sequence).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct GaijiSpanView {
    pub(crate) range: Range,
    pub(crate) resolved: String,
    pub(crate) description: String,
    pub(crate) mencode: Option<String>,
}

/// Result for `aozora/gaijiSpans` — every resolvable gaiji in the
/// document. Unresolved spans (description not in any table, no
/// `U+XXXX` form) are omitted because the editor has nothing to
/// substitute in their place.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct GaijiSpansResult {
    pub(crate) spans: Vec<GaijiSpanView>,
}

/// JSON shape of the `aozora.canonicalizeSlug` `execute_command`
/// argument. Lifted to the top level so the
/// `clippy::items_after_statements` lint does not fire from inside
/// the `LanguageServer::execute_command` body.
#[derive(serde::Deserialize)]
struct CanonicalizeArgs {
    uri: Url,
    range: Range,
    body: String,
}

/// Convert an LSP `TextDocumentContentChangeEvent` into a
/// [`ByteEdit`] against `source`. Returns `None` when the event
/// has no range (caller handles full-replacement separately) or when
/// either Position fails to resolve to a valid byte offset.
fn lsp_change_to_edit(source: &str, change: &TextDocumentContentChangeEvent) -> Option<ByteEdit> {
    let range = change.range?;
    let start = position_to_byte_offset(source, range.start)?;
    let end = position_to_byte_offset(source, range.end)?;
    if end < start {
        return None;
    }
    Some(ByteEdit::new(start..end, change.text.clone()))
}

#[tower_lsp::async_trait]
impl LanguageServer for AozoraLanguageServer {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        // The editor's own UI language, reported by every client that speaks
        // the spec (VS Code's `env.language`, and the same field helix /
        // neovim / Zed send). It decides the server's, once — see
        // `crate::i18n` for where it sits in the precedence chain.
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
        self.docs
            .insert(uri.clone(), OpenDocument::new(p.text_document.text));
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
        let multi = p.content_changes.len() > 1;
        for change in &p.content_changes {
            // Resolve the change against the LATEST snapshot. The LSP
            // spec applies multi-change batches in array order, with
            // each change's coordinates referring to the buffer state
            // *after* every prior change in the same batch — so the
            // 2nd+ iterations need an up-to-date snapshot. Without
            // this, a multi-change batch that paste-rewrites two
            // ranges in one notification would address the second
            // range against the pre-batch text and corrupt the buffer.
            let snap = state.snapshot();
            // LSP allows mixing incremental and full-replacement
            // events in one batch; full replacement is signalled
            // by `range == None`.
            match lsp_change_to_edit(snap.doc_text(), change) {
                Some(edit) => {
                    _ = state.apply_changes(slice::from_ref(&edit));
                }
                None if change.range.is_none() => {
                    state.replace_text(change.text.clone());
                }
                None => {
                    tracing::warn!(
                        "skipping content change with unresolvable range: {:?}",
                        change.range,
                    );
                }
            }
            // After each apply, force a synchronous snapshot rebuild
            // so the next iteration sees the post-edit text. Single-
            // change batches (the common case) skip this — the
            // debounced publish path drives the rebuild later.
            //
            // Inside tokio the rebuild blocks the async task briefly
            // (a few ms even for large docs); we accept that bound
            // because multi-change batches are rare and skipping the
            // rebuild produces silent buffer corruption.
            if multi {
                state.rebuild_snapshot_now();
            }
        }
        // Schedule the slow semantic parse + publish as a debounced
        // background task. `did_change` itself returns now (microseconds
        // later), so subsequent LSP requests are not blocked by
        // tower-lsp's notification ordering.
        self.schedule_publish_debounced(uri);
    }

    #[tracing::instrument(skip_all, fields(uri = %p.text_document.uri))]
    async fn did_close(&self, p: DidCloseTextDocumentParams) {
        let uri = p.text_document.uri;
        // Dump the per-document Metrics snapshot at INFO so a third
        // party reading the log can reconstruct the document's
        // session-long behaviour. Done BEFORE the remove so we
        // still have access to the entry.
        if let Some(state) = self.lookup(&uri) {
            let snapshot = state.metrics.snapshot();
            tracing::info!(
                target: "aozora_lsp::metrics",
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
        // Wait-free snapshot read; the parse + serialize runs on the
        // blocking pool so concurrent hover/codeAction requests on the
        // async runtime don't stall.
        let text = state.snapshot().doc_text().to_string();
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
        // Wait-free snapshot. `hover_at` only reads the slice, so the
        // Arc<str> from snapshot is sufficient with no extra clone.
        let snap = state.snapshot();
        Ok(hover_at(snap.doc_text(), position, ui_lang()))
    }

    // `inlay_hint` deliberately *not* implemented — see
    // `crate::capabilities`, which declines to advertise it.

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
        // Tree-free source scan — bounded look-window around the
        // cursor (≤ 1 KB each side). No parser invoked.
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
        // Tree-aware: lend the cached parse and ask the splice engine whether
        // this position sits on a coupled region. `with_tree` returns `None`
        // when there is no stored output, so flatten that into the response.
        let snap = state.snapshot();
        Ok(state
            .with_parse_cache(|c| {
                c.with_tree(|t| {
                    prepare_rename_at(t, snap.doc_text(), snap.doc_line_index(), position)
                })
            })
            .flatten())
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
        // The splice engine is the authority: `rename_edit` returns
        // `Ok(None)` for a non-coupled / no-op / gate-failed position and
        // `Err(SpliceError)` when the splice is honestly declined (an
        // ambiguous referent, a ruby-base literal, a 、-joined multi-target).
        let outcome = state.with_parse_cache(|c| {
            c.with_tree(|t| {
                rename_edit(
                    t,
                    snap.doc_text(),
                    snap.doc_line_index(),
                    &uri,
                    position,
                    &new_name,
                )
            })
        });
        match outcome {
            // No stored tree, or position not on a coupled region / no-op.
            None | Some(Ok(None)) => Ok(None),
            Some(Ok(Some(edit))) => Ok(Some(edit)),
            // The splice declined: surface it as an invalid-params error so the
            // editor shows the reason rather than silently doing nothing.
            Some(Err(e)) => Err(JsonRpcError::invalid_params(e.to_string())),
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
        // Tree-free: completion_at does its own bounded look-back
        // scan from the cursor (no parser needed). Removing the
        // `with_tree` call eliminates a full document re-parse on
        // every keystroke during slug completion — a major win on
        // 40 KB+ documents.
        let lang = ui_lang();
        let mut items: Vec<CompletionItem> = completion_at(snap.doc_text(), position, lang);
        // Append the half-width emmet suggestions. They are
        // independent of the parsed tree (the trigger detection is a
        // pure prefix scan), so we don't pay for a `with_tree` call
        // and the slug catalogue + emmet items merge into one
        // response — VS Code's own ranker decides ordering.
        items.extend(emmet_completions(snap.doc_text(), position, lang));
        // Plus the structured-snippet items that fire after the
        // user just typed `#` / `｜` / `《` / `※`. Each item carries
        // a snippet body with `${…}` Tab-stops so accepting expands
        // into a fully-structured form (`［＃改ページ］` etc) and
        // leaves the cursor in the next placeholder for IDE-style
        // Tab navigation (the user-asked feature, 2026-04-29).
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
        // Quick fixes for diagnostics in the request range. Each
        // diagnostic carries a `data` payload describing what kind
        // of fix is appropriate; `quick_fix_actions` decodes those
        // and returns concrete `WorkspaceEdit`s.
        let lang = ui_lang();
        let mut actions = quick_fix_actions(&uri, &p.context.diagnostics, lang);
        // Plus the wrap-selection actions when the user has a
        // non-empty selection. Both kinds are returned together so
        // the editor's lightbulb / right-click menu shows them in
        // one list.
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
        // Argument shape: a single JSON object with `uri`, `range`, `body`.
        let arg = params
            .arguments
            .into_iter()
            .next()
            .ok_or_else(|| JsonRpcError::invalid_params("expected one argument object"))?;
        let CanonicalizeArgs { uri, range, body } = serde_json::from_value(arg)
            .map_err(|err| JsonRpcError::invalid_params(err.to_string()))?;
        let Some(workspace_edit) = canonicalize_slug_edit(uri, range, &body) else {
            return Ok(None);
        };
        // Apply the edit through the client's
        // `workspace/applyEdit` RPC. Failures bubble up as
        // jsonrpc::Error.
        if let Err(err) = self.client.apply_edit(workspace_edit).await {
            tracing::warn!(error = %err, "applyEdit failed");
        }
        Ok(None)
    }

    #[tracing::instrument(skip_all, fields(uri = %p.text_document.uri))]
    async fn folding_range(&self, p: FoldingRangeParams) -> Result<Option<Vec<FoldingRange>>> {
        let uri = p.text_document.uri;
        let Some(state) = self.lookup(&uri) else {
            return Ok(None);
        };
        // Pure text-scan against the snapshot — no parser invoked.
        // Wait-free: a single ArcSwap load + a linear pass over the
        // immutable `Arc<str>`.
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
        let symbols: Vec<DocumentSymbol> =
            document_symbols(snap.doc_text(), snap.doc_line_index(), ui_lang());
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
        // Per-paragraph walks against each paragraph's tree — see
        // semantic_tokens module docs.
        let tokens: SemanticTokens = semantic_tokens_full(&snap.paragraphs);
        Ok(Some(SemanticTokensResult::Tokens(tokens)))
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    //! Multi-angle test suite for the LSP incremental sync layer.
    //!
    //! Test sections:
    //!
    //! 1. **Conversion** — `lsp_change_to_edit` handles every well-formed
    //!    LSP `Range` correctly (ASCII, multibyte, multi-line, surrogate
    //!    pairs, edge offsets) and rejects ill-formed ones.
    //! 2. **`OpenDocument` mechanics** — `apply_changes` / `replace_text`
    //!    move the buffer through the right transitions including
    //!    failure recovery.
    //! 3. **Batch semantics** — multiple `TextDocumentContentChangeEvent`
    //!    events in one notification compose to the same final state
    //!    as individual notifications, preserving LSP's source-order
    //!    rule.
    //! 4. **Edit content shapes** — pure-text fast path vs aozora-trigger
    //!    fallback path; both must remain byte-equivalent to a full parse.
    //! 5. **End-to-end equivalence** — long edit sequences and full
    //!    replacements both converge to the buffer the user actually sees.

    use super::*;
    use tower_lsp::lsp_types::{Position, Range};

    /// The locale-parameterised helpers below are pinned to English rather
    /// than read through `ui_lang()` — these tests must not depend on a
    /// process-global the rest of the binary also seeds.
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

    /// Replay a batch of LSP changes against a starting buffer using
    /// the same logic the backend uses, so tests can compare against
    /// "what the editor thinks the buffer looks like" without booting
    /// tower-lsp.
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
                None => {} // unresolvable range: skip (matches backend behaviour)
            }
        }
        state.snapshot().doc_text().to_string()
    }

    // ---------------------------------------------------------------
    // 1. Conversion: lsp_change_to_edit
    // ---------------------------------------------------------------

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
        // 「あ」 is 1 UTF-16 unit, 3 UTF-8 bytes.
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
        // end < start must be refused rather than producing a backwards
        // splice that would corrupt the buffer.
        let change = synth_change(
            Some(Range::new(Position::new(0, 5), Position::new(0, 2))),
            "x",
        );
        assert!(lsp_change_to_edit("hello world", &change).is_none());
    }

    #[test]
    fn lsp_change_to_edit_handles_pure_deletion() {
        let source = "abcdef";
        // Delete bytes 2..4 ("cd").
        let change = synth_change(
            Some(Range::new(Position::new(0, 2), Position::new(0, 4))),
            "",
        );
        let edit = lsp_change_to_edit(source, &change).expect("convert");
        assert_eq!(edit.range, 2..4);
        assert_eq!(edit.new_text, "");
    }

    // ---------------------------------------------------------------
    // 2. OpenDocument mechanics
    // ---------------------------------------------------------------

    #[test]
    fn doc_state_new_populates_cache() {
        let state = OpenDocument::new("hello".to_owned());
        // Plain text emits zero diagnostics — the cache surfaces an
        // empty slice but is *populated* (no longer "first reparse"
        // pending).
        state.with_parse_cache(|cache| {
            assert!(cache.diagnostics().is_empty());
        });
        assert_eq!(&**state.snapshot().doc_text(), "hello");
    }

    #[test]
    fn doc_state_apply_changes_updates_text() {
        let state = OpenDocument::new("hello world".to_owned());
        let edit = ByteEdit::new(6..11, "rust".to_owned());
        state.apply_changes(&[edit]);
        assert_eq!(&**state.snapshot().doc_text(), "hello rust");
    }

    #[test]
    fn doc_state_apply_changes_rejects_invalid_edit_keeps_text() {
        let state = OpenDocument::new("hi".to_owned());
        let edit = ByteEdit::new(0..99, "x".to_owned());
        let result = state.apply_changes(&[edit]);
        assert!(result.is_none(), "out-of-bounds edit must be rejected");
        assert_eq!(&**state.snapshot().doc_text(), "hi");
    }

    #[test]
    fn doc_state_apply_changes_rejects_non_char_boundary_edit() {
        let state = OpenDocument::new("あ".to_owned()); // 3 bytes
        let edit = ByteEdit::new(1..2, "x".to_owned());
        let result = state.apply_changes(&[edit]);
        assert!(result.is_none(), "cross-boundary edit must be rejected");
        assert_eq!(
            &**state.snapshot().doc_text(),
            "あ",
            "non-boundary edit must be rejected",
        );
    }

    #[test]
    fn doc_state_replace_text_updates_buffer() {
        let state = OpenDocument::new("hello".to_owned());
        state.replace_text("｜青梅《おうめ》".to_owned());
        assert_eq!(&**state.snapshot().doc_text(), "｜青梅《おうめ》");
    }

    // ---------------------------------------------------------------
    // 3. Batch semantics
    // ---------------------------------------------------------------

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

    // ---------------------------------------------------------------
    // 4. Edit content shapes
    // ---------------------------------------------------------------

    #[test]
    fn edit_inserting_aozora_trigger_reparses() {
        let state = OpenDocument::new("plain text".to_owned());
        let edit = ByteEdit::new(5..6, "｜青梅《おうめ》".to_owned());
        state.apply_changes(&[edit]);
        // `apply_changes` is the fast path — text + TS edit only. The
        // semantic re-parse runs in a debounced background task in
        // production. For this unit test (no async runtime) we drive
        // it synchronously through the same entry point the debounced
        // task uses.
        state.run_parse_cache_reparse();
        state.with_parse_cache(|cache| {
            let inline = cache
                .with_tree(|t| t.lex_output().registry.count_kind(aozora::Sentinel::Inline))
                .expect("populated");
            assert_eq!(inline, 1);
            assert!(cache.diagnostics().is_empty());
        });
    }

    #[test]
    fn pua_collision_edit_surfaces_diagnostic() {
        let state = OpenDocument::new("plain".to_owned());
        let edit = ByteEdit::new(0..0, "\u{E001}".to_owned());
        state.apply_changes(&[edit]);
        // See note in `edit_inserting_aozora_trigger_reparses` — the
        // semantic re-parse is deferred to the debounced background
        // task in production.
        state.run_parse_cache_reparse();
        state.with_parse_cache(|cache| {
            assert!(
                !cache.diagnostics().is_empty(),
                "PUA injection must produce diagnostics; got {:?}",
                cache.diagnostics(),
            );
        });
    }

    // ---------------------------------------------------------------
    // 5. End-to-end
    // ---------------------------------------------------------------

    #[test]
    fn sequence_of_incremental_edits_converges_to_full_text() {
        let state = OpenDocument::new(String::new());
        for (i, ch) in "hello world".chars().enumerate() {
            let edit = ByteEdit::new(i..i, ch.to_string());
            state.apply_changes(&[edit]);
        }
        assert_eq!(&**state.snapshot().doc_text(), "hello world");
    }

    /// Replay-style helper that mirrors the production `did_change`
    /// loop *including* the post-edit `rebuild_snapshot_now()` so the
    /// next iteration's snapshot reflects every prior apply. The
    /// production loop is bounded by `multi`, but the test driver
    /// always rebuilds since we want a deterministic final state.
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
        state.snapshot().doc_text().to_string()
    }

    /// Regression: `did_change` defers snapshot rebuilds onto a tokio
    /// blocking task, so the 2nd change in a multi-change batch saw
    /// the *pre-batch* snapshot text. With the in-batch rebuild
    /// added, the second change resolves against the post-1st-change
    /// text — matching LSP's "apply in array order" semantics. We
    /// rebuild eagerly between every iteration in this test driver
    /// to mirror the multi-change branch deterministically.
    #[test]
    fn multi_change_batch_resolves_against_post_prior_change_text() {
        // Insert at byte 0, then insert at byte 1 (which only exists
        // after the first insert). Without the rebuild, the second
        // edit would be evaluated against the original text where
        // byte 1 means a different position.
        let initial = "abc";
        let changes = vec![
            // Change 0: insert "X" at start. Post-1st text: "Xabc".
            synth_change(
                Some(Range::new(Position::new(0, 0), Position::new(0, 0))),
                "X",
            ),
            // Change 1: insert "Y" at column 4 of the post-1st text
            // (= byte 4 = end of "Xabc"). The pre-batch text is
            // only 3 chars wide, so column 4 there clamps to EOF; if
            // the snapshot rebuild were skipped the apply would
            // either reject the edit or land it in the wrong spot.
            synth_change(
                Some(Range::new(Position::new(0, 4), Position::new(0, 4))),
                "Y",
            ),
        ];
        let final_text = replay_lsp_changes_with_sync_rebuild(initial, &changes);
        assert_eq!(final_text, "XabcY");
    }

    /// The same batch driven through the *production* code path with
    /// `AozoraLanguageServer::did_change` would also need the in-batch rebuild;
    /// pin a mid-batch insert that's only valid against the
    /// post-1st-change text, exercised through `OpenDocument` directly.
    #[test]
    fn multi_change_batch_dependent_offsets_round_trip_via_doc_state() {
        // Initial: "本文" (6 bytes). Change 0 inserts "｜" (3 bytes)
        // at the start. Change 1 inserts "" + "あ"《"a"》 form needs
        // an offset only present after the first insert. Pin the
        // expected final text so any drift fails loudly.
        let initial = "本文";
        let changes = vec![
            synth_change(
                Some(Range::new(Position::new(0, 0), Position::new(0, 0))),
                "｜",
            ),
            // Column 1 of post-1st text = 1 char in (just past `｜`).
            synth_change(
                Some(Range::new(Position::new(0, 1), Position::new(0, 1))),
                "X",
            ),
        ];
        let final_text = replay_lsp_changes_with_sync_rebuild(initial, &changes);
        assert_eq!(final_text, "｜X本文");
    }

    /// `DocSnapshot` rebuild between iterations must be a no-op for
    /// single-change batches — we don't want to pay the rebuild cost
    /// when the next iteration won't run. Pin that the rebuild path
    /// produces the same final state as the no-rebuild path for a
    /// single change.
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

    // ---------------------------------------------------------------
    // 6. Byte-length → MiB helper + oversize notice construction
    // ---------------------------------------------------------------

    /// Pin `as_mib` to exact whole-MiB integer division. The single
    /// `2 MiB → 2` assertion alone distinguishes the correct
    /// `bytes / (1024 * 1024)` from every mutation cargo-mutants
    /// applies to it:
    ///
    /// * `-> 0` and `-> 1` constant returns (2 ≠ 0, 2 ≠ 1),
    /// * `/` → `%` (`2 MiB % 1 MiB == 0`),
    /// * `/` → `*` (a value in the trillions),
    /// * `1024 * 1024` divisor with `*` → `+` (divisor `2048` → `1024`),
    /// * `1024 * 1024` divisor with `*` → `/` (divisor `1` → `2 MiB`).
    #[test]
    fn as_mib_is_whole_mebibyte_floor_division() {
        assert_eq!(as_mib(2 * 1024 * 1024), 2);
        // A second, larger reference point rules out any accidental
        // affine relationship that happens to pass at 2 MiB.
        assert_eq!(as_mib(48 * 1024 * 1024), 48);
        // Whole-MiB floor: one byte under a MiB truncates to zero, and
        // exactly one MiB is one.
        assert_eq!(as_mib(1024 * 1024 - 1), 0);
        assert_eq!(as_mib(1024 * 1024), 1);
    }

    /// Pin the mutable fields of the oversize notice `Diagnostic`.
    /// Deleting the `source` field defaults it to `None`; deleting the
    /// `message` field defaults it to the empty string — both are
    /// caught here. The `severity` and `range` are pinned for the
    /// contract even though they are not the mutation targets.
    ///
    /// (Deleting the `range` field is an *equivalent* mutation:
    /// `Range::default()` is byte-identical to the explicit
    /// `Range::new(Position::new(0, 0), Position::new(0, 0))`, so no
    /// test can observe the difference — this assertion documents the
    /// anchor but cannot kill that mutant.)
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

    /// Both oversize messages resolve out of the catalog with their `$size` /
    /// `$limit` bound. A missing key surfaces as the bare key (`aozora_i18n`'s
    /// gap signal) and an unbound placeable as `{$size}` — neither carries the
    /// numbers, so asserting on those catches either.
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

    /// The preview notice is a single inert paragraph — the webview renders it
    /// as HTML, so nothing but the catalog's prose may reach it.
    #[test]
    fn oversize_html_notice_is_one_inert_paragraph() {
        let html = oversize_html_notice(20 * 1024 * 1024, &en());
        assert!(html.starts_with("<p>") && html.ends_with("</p>"), "{html}");
        assert!(!html[3..html.len() - 4].contains('<'), "{html}");
    }
}

#[cfg(test)]
mod e2e {
    //! In-process **end-to-end** tests that drive the real [`LspService`]
    //! as a [`tower::Service`] — the same path tower-lsp's stdio runloop
    //! uses, minus the stdin/stdout framing. This exercises the async
    //! `LanguageServer` handler bodies, the custom-method router
    //! (`aozora/renderHtml`, `aozora/gaijiSpans`), the
    //! `initialize → initialized` state transition that ungates
    //! server→client traffic, the debounced publish path, and the
    //! loopback `ClientSocket` — none of which the pure-helper tests in
    //! `mod tests` (which deliberately skip booting tower-lsp) can reach.
    //!
    //! The harness splits the loopback socket and answers any
    //! server-issued request (e.g. `workspace/applyEdit` from
    //! `execute_command`) with a success response, so handlers that await
    //! a client reply make progress instead of hanging.

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

    /// A live `LspService` plus the server→client traffic a background
    /// drain task has collected.
    struct TestServer {
        service: LspService<AozoraLanguageServer>,
        /// Every server→client message observed so far (publishDiagnostics,
        /// logMessage, applyEdit, …), in arrival order.
        outbound: Arc<Mutex<Vec<Request>>>,
        next_id: i64,
    }

    impl TestServer {
        /// Build the service through the same `crate::build_service` the
        /// daemon uses, then spawn the loopback drain + auto-responder.
        fn new() -> Self {
            let (service, socket) = crate::build_service();
            let outbound = Arc::new(Mutex::new(Vec::new()));
            let collected = Arc::clone(&outbound);
            let (mut requests, mut responses) = socket.split();
            tokio::spawn(async move {
                while let Some(req) = requests.next().await {
                    let id = req.id().cloned();
                    collected.lock().push(req);
                    // A server→client *request* (has an id) expects a reply;
                    // a permissive success body keeps `apply_edit` and any
                    // future client request from blocking forever.
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

        /// Drive one message through the tower stack (ready → call).
        async fn call(&mut self, req: Request) -> Option<Response> {
            poll_fn(|cx| self.service.poll_ready(cx))
                .await
                .expect("service ready");
            self.service.call(req).await.expect("service call")
        }

        /// Issue a request (auto-incrementing id) and return the raw
        /// jsonrpc result so callers can assert on error paths too.
        async fn try_request(&mut self, method: &'static str, params: Value) -> Result<Value> {
            self.next_id += 1;
            // `Value::Null` means "omit params" (e.g. `shutdown`), matching a
            // real client — the router rejects an explicit `params: null`.
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

        /// Issue a request expected to succeed, returning its result value.
        async fn request(&mut self, method: &'static str, params: Value) -> Value {
            self.try_request(method, params)
                .await
                .expect("request result is Ok")
        }

        /// Fire a notification (no id); asserts the server returns no response.
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

        /// `initialize` + `initialized` so the client send-gate opens.
        ///
        /// Reports a locale like a real client does, which fixes the UI
        /// language these tests run against instead of leaving it to the
        /// `LANG` of whoever runs them.
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

        /// Spin (real time, generously bounded — covers the 150 ms publish
        /// debounce) until `f` extracts a value from the collected
        /// server→client traffic.
        #[allow(
            clippy::future_not_send,
            reason = "test-only harness driven by block_on in #[tokio::test]; never spawned, so Send is not required"
        )]
        async fn wait_until<T>(&self, f: impl Fn(&[Request]) -> Option<T>) -> T {
            for _ in 0..400 {
                // Bind so the parking_lot guard drops at the end of this
                // statement — never held across the `.await` below.
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

        /// Count how many server→client messages of `method` arrived.
        fn outbound_count(&self, method: &str) -> usize {
            self.outbound
                .lock()
                .iter()
                .filter(|r| r.method() == method)
                .count()
        }
    }

    const PUBLISH: &str = "textDocument/publishDiagnostics";

    /// Pull the `diagnostics` array out of a `publishDiagnostics` request.
    fn published_diagnostics(req: &Request) -> Vec<Value> {
        req.params()
            .and_then(|p| p.get("diagnostics"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    }

    /// True when `diag` is the single informational oversize notice
    /// (`oversize_notice`) rather than a real analysis diagnostic. Used
    /// by the boundary tests to tell "analysed normally" apart from
    /// "replaced by the oversize notice". The MiB figures it quotes are the
    /// one thing no analysis diagnostic says — and they read the same in
    /// every locale, so this does not pin the notice's prose.
    fn is_oversize_notice(diag: &Value) -> bool {
        diag["severity"] == json!(3 /* INFORMATION */)
            && diag["message"].as_str().is_some_and(|m| m.contains("MiB"))
    }

    /// Fetch a document's rendered HTML body (custom `aozora/renderHtml`).
    /// A read-back probe for tests that need to inspect the settled
    /// buffer text after a `did_change`.
    #[allow(
        clippy::future_not_send,
        reason = "test-only helper driven by block_on in #[tokio::test]; never spawned, so Send is not required"
    )]
    async fn rendered_html(server: &mut TestServer) -> String {
        let resp = server
            .request("aozora/renderHtml", json!({ "uri": URI }))
            .await;
        resp["html"].as_str().unwrap_or_default().to_owned()
    }

    /// Wait until at least `n` `publishDiagnostics` have been observed,
    /// returning the `n`-th (1-indexed) one.
    #[allow(
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

    // -----------------------------------------------------------------
    // Lifecycle + capabilities
    // -----------------------------------------------------------------

    /// The handshake hands back exactly what [`crate::capabilities`]
    /// declares. The equality pins the seam; the field assertions pin the
    /// contents against the wire, naming each provider by the key a client
    /// reads — so deleting one from `server_capabilities` fails here, in a
    /// test that runs without the `internals` feature (the capability
    /// snapshot in `tests/snapshots.rs` is behind it, and a self-referential
    /// `to_value(server_capabilities())` compare would move with the deletion).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn initialize_advertises_capabilities_and_server_info() {
        let mut server = TestServer::new();
        let caps = server.handshake().await;
        let advertised = &caps["capabilities"];
        assert_eq!(
            *advertised,
            serde_json::to_value(server_capabilities()).expect("capabilities serialize"),
        );

        // Incremental sync (kind 2), so a keystroke ships a delta not the file.
        assert_eq!(advertised["textDocumentSync"], json!(2));
        // The provider set clients dispatch on: each must be present, and
        // deleting its field from `server_capabilities` must reach this test.
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
        // The deliberate omission is a contract too (see `crate::capabilities`).
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

    // -----------------------------------------------------------------
    // Read-only request handlers
    // -----------------------------------------------------------------

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

    // -----------------------------------------------------------------
    // Diagnostics publish (requires the initialized send-gate)
    // -----------------------------------------------------------------

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
            diags
                .iter()
                .any(|d| d["severity"] == json!(2 /* WARNING */)),
            "expected a warning diagnostic, got: {diags:?}",
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn did_change_applies_edit_and_republishes() {
        let mut server = TestServer::new();
        server.handshake().await;
        server.did_open("abc").await;
        // Wait for the open-time publish so we can detect the *second*,
        // debounced publish the change schedules.
        server
            .wait_until(|reqs| (reqs.iter().any(|r| r.method() == PUBLISH)).then_some(()))
            .await;

        // Incremental insert "d" at end → "abcd".
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

        // The debounced reparse must republish (exercises the
        // schedule → reparse_and_publish_if_current path).
        server
            .wait_until(|reqs| {
                (reqs.iter().filter(|r| r.method() == PUBLISH).count() >= 2).then_some(())
            })
            .await;

        // The post-change buffer is the canonical plain ASCII "abcd", so a
        // formatting request returns zero edits — a read-back probe that the
        // incremental edit landed cleanly in the snapshot.
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

        // didClose publishes an empty diagnostic set to clear squiggles.
        let cleared = server
            .wait_until(|reqs| {
                reqs.iter()
                    .filter(|r| r.method() == PUBLISH)
                    .nth(before)
                    .cloned()
            })
            .await;
        assert!(published_diagnostics(&cleared).is_empty());
    }

    // -----------------------------------------------------------------
    // The remaining read-only request handlers (shape-level wiring;
    // each module's own unit tests pin the detailed output).
    // -----------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn completion_handler_returns_items_at_trigger() {
        let mut server = TestServer::new();
        server.handshake().await;
        server.did_open("｜青空《あおぞら》").await;
        // Inside the reading `《…》` the slug/emmet catalogues fire.
        let resp = server
            .request(
                "textDocument/completion",
                json!({
                    "textDocument": { "uri": URI },
                    "position": { "line": 0, "character": 1 },
                }),
            )
            .await;
        // Either an array or a CompletionList object, or null — assert the
        // handler produced well-formed JSON (no error path).
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
        // Typing `[` converts to full-width `［`.
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

        // linkedEditingRange: cursor inside a paired delimiter.
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
        // initialize advertises a rename provider with prepareProvider true.
        assert_eq!(
            caps["capabilities"]["renameProvider"]["prepareProvider"],
            json!(true),
            "renameProvider must advertise prepareProvider: {caps}",
        );

        server
            .did_open("前\n［＃ここから2字下げ］\n本文\n［＃ここで字下げ終わり］\n後")
            .await;

        // prepareRename on the open marker → non-null RangeWithPlaceholder.
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

        // prepareRename on plain text → null.
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

        // rename newName "［＃ここから罫囲み］" → WorkspaceEdit mentioning 罫囲み終わり.
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

    // -----------------------------------------------------------------
    // execute_command
    // -----------------------------------------------------------------

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
                        "range": {
                            "start": { "line": 0, "character": 0 },
                            "end":   { "line": 0, "character": 6 },
                        },
                        "body": "［＃ぼうてん］",
                    }],
                }),
            )
            .await;
        assert!(result.is_null(), "command returns null on success");
        // The server must have asked the client to apply the canonicalised edit.
        let apply = server
            .wait_until(|reqs| {
                reqs.iter()
                    .find(|r| r.method() == "workspace/applyEdit")
                    .cloned()
            })
            .await;
        let new_text = apply.params().and_then(|p| {
            p["edit"]["changes"]
                .as_object()
                .and_then(|m| m.values().next())
                .and_then(|edits| edits.get(0))
                .and_then(|e| e["newText"].as_str())
                .map(str::to_owned)
        });
        assert_eq!(new_text.as_deref(), Some("［＃傍点］"));
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
                        "range": {
                            "start": { "line": 0, "character": 0 },
                            "end":   { "line": 0, "character": 4 },
                        },
                        "body": "［＃傍点］",
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
        // Right command, but the argument object is missing `range`/`body`,
        // so `serde_json::from_value::<CanonicalizeArgs>` fails.
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

    // -----------------------------------------------------------------
    // Custom methods
    // -----------------------------------------------------------------

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

    // -----------------------------------------------------------------
    // Oversize document path (>16 MiB): semantic analysis is skipped and
    // an informational notice replaces diagnostics / preview HTML.
    // -----------------------------------------------------------------

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
        assert_eq!(diags[0]["severity"], json!(3 /* INFORMATION */));

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

    /// The counterpart to the oversize path: an analysed document renders its
    /// own HTML and says so, so `paused` actually discriminates.
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

    /// The `initialized` handler logs a readiness line to the client.
    /// Replacing its body with `()` drops that message.
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

    // -----------------------------------------------------------------
    // did_change match-arm + multi-change semantics
    // -----------------------------------------------------------------

    /// A content change with no `range` is a whole-document replacement,
    /// routed through the `None if change.range.is_none()` arm to
    /// `replace_text`. Forcing that guard to `false` would drop the
    /// change and leave the buffer stale.
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

    /// An inverted range (`end < start`) makes `lsp_change_to_edit`
    /// return `None` while `change.range.is_some()`, so the correct
    /// guard `change.range.is_none()` is false and the change is
    /// skipped. Forcing the guard to `true` would wrongly call
    /// `replace_text(change.text)` and clobber the buffer.
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

    /// A two-change batch whose SECOND change addresses a column that
    /// only exists after the FIRST change is applied. `multi =
    /// content_changes.len() > 1` must be TRUE so the in-loop
    /// `rebuild_snapshot_now()` refreshes the snapshot between the
    /// changes; mutating `> 1` to `== 1` or `< 1` makes `multi` false
    /// for a 2-change batch, so change 1 resolves column 4 against the
    /// pre-batch 3-char text and the buffer is corrupted.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn did_change_multi_dependent_batch_rebuilds_between_changes() {
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

    // -----------------------------------------------------------------
    // Handlers whose whole body a "-> Ok(None)" mutation could stub out
    // -----------------------------------------------------------------

    /// `linkedEditingRange` on a ruby-open delimiter must return the
    /// paired open/close ranges — a bare `Ok(None)` stub would drop them.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn linked_editing_returns_pair_on_ruby_delimiter() {
        let mut server = TestServer::new();
        server.handshake().await;
        server.did_open("｜青空《あおぞら》").await;
        // 《 sits at UTF-16 column 3; 》 at column 8.
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

    /// `completion` right after a half-width `[` must surface the emmet
    /// full-width `［` item — a bare `Ok(None)` stub would drop it.
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

    /// `foldingRange` over a `ここから … 終わり` block returns exactly one
    /// fold with a specific span. Pins the whole handler against the
    /// `Ok(None)`, `Ok(Some(vec![]))`, and `Ok(Some(vec![Default::default()]))`
    /// stubs: `None` → null, empty vec → wrong length, and the default
    /// fold has `startLine`/`endLine` 0, not the real 1..3.
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

    /// `documentSymbol` over a heading returns exactly that heading — a
    /// bare `Ok(None)` stub would drop the outline.
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

    // -----------------------------------------------------------------
    // The `exceeds_document_cap` size gate — behavioural side.
    //
    // The exclusive boundary itself (`len > cap`) is pinned cheaply on the
    // pure predicate in `parse_cache::tests` (integer lengths, no fixture),
    // which kills every `>`→`>=`/`==`/`<` mutant at all five call sites. What
    // still needs a real over-limit buffer is the BEHAVIOUR on the oversize
    // side: the debounced reparse must post the notice instead of analysing.
    // -----------------------------------------------------------------

    /// A document one byte ABOVE the limit must always take the oversize
    /// branch in the debounced reparse. `raw.len_bytes() > MAX` mutated
    /// to `<` would push an over-limit buffer into full analysis. This
    /// stays cheap on the correct path — an over-limit buffer skips the
    /// parse entirely, so only the mutant does real work.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn over_limit_reparse_posts_oversize_notice() {
        let mut server = TestServer::new();
        server.handshake().await;
        // One byte over MAX, as moderate-length lines (see
        // `exact_max_document_is_analysed_not_noticed`): the correct path
        // skips analysis entirely, and only the mutant that flips the gate
        // parses, so keep even that off any per-line quadratic path.
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
