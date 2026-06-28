//! `aozora-lsp` — Language Server for aozora-flavored-markdown.
//!
//! The server is built on top of the `aozora` library surface from
//! the sibling `aozora` repository. Three primary LSP capabilities:
//!
//! - `textDocument/publishDiagnostics` — every `aozora::Diagnostic`
//!   variant is mapped to an LSP `Diagnostic` with a byte-range span
//!   converted into line/UTF-16-column coordinates.
//! - `textDocument/formatting` — runs the `aozora` `parse ∘ serialize`
//!   round-trip (the same canonical form `aozora-fmt` produces) and
//!   returns a single document-replace `TextEdit`.
//! - `textDocument/hover` — when the cursor sits inside a
//!   `※［＃…］` gaiji reference, resolves via `aozora::encoding::gaiji`
//!   and returns a Markdown explanation.
//!
//! The stable public surface is intentionally tiny: [`Cli`] (so `xtask`
//! can generate the shell completions and man page) and [`run`], the
//! daemon entry point. The internal building blocks the handlers are made
//! of are re-exported behind the `#[doc(hidden)]` `internals` module —
//! for the crate's own tests, benches, examples, and fuzz targets only,
//! with no semver guarantee.

#![forbid(unsafe_code)]
// The stable surface is just `Cli` + `run` (both documented). The bulk of the
// crate's `pub` items exist only to be re-exported through the `#[doc(hidden)]`
// `internals` module for the crate's own tests/benches/examples, and carry no
// semver or doc guarantee — so we don't require rustdoc on them.
#![allow(
    missing_docs,
    reason = "pub items are re-exported via the doc-hidden `internals` module \
              for the crate's own tests/benches and carry no doc guarantee; \
              the stable surface (Cli + run) is documented"
)]

mod backend;
mod cli;
mod code_actions;
mod commands;
mod completion;
mod diagnostics;
mod doc_line_view;
mod document_symbol;
mod folding_range;
mod formatting;
mod gaiji_spans;
mod half_width_emmet;
mod hover;
mod inlay_hints;
mod line_index;
mod linked_editing;
mod metrics;
mod on_type_formatting;
mod paragraph;
mod parse_cache;
mod position;
mod rename;
mod semantic_tokens;
mod state;
mod structured_snippets;
mod text_edit;
mod tree_sitter_doc;

use std::io;

use tokio::io::{stdin, stdout};
use tower_lsp::{ClientSocket, LspService, Server};
use tracing_subscriber::EnvFilter;

use crate::backend::AozoraLanguageServer;

pub use cli::Cli;

/// Build the `LspService` with the aozora custom methods (`aozora/renderHtml`,
/// `aozora/gaijiSpans`) wired onto the builder — tower-lsp's `LanguageServer`
/// trait only covers spec-defined methods, so custom ones go here.
///
/// Factored out of [`run`] so the in-crate end-to-end harness
/// (`backend::e2e`) builds the server exactly the way the daemon does; the
/// custom-method routing is therefore exercised by tests and can't silently
/// drift from production.
pub(crate) fn build_service() -> (LspService<AozoraLanguageServer>, ClientSocket) {
    LspService::build(AozoraLanguageServer::new)
        .custom_method("aozora/renderHtml", AozoraLanguageServer::render_html)
        .custom_method("aozora/gaijiSpans", AozoraLanguageServer::gaiji_spans)
        .finish()
}

/// Run the `aozora-lsp` daemon: parse argv, then [`serve`] over stdio.
///
/// argv is handled first so clap prints and exits for `--version` /
/// `--help` (and on a usage error) *before* the JSON-RPC stream opens, so
/// nothing pollutes the protocol channel on stdout. `--stdio` is accepted
/// (and ignored) for editor compatibility.
pub async fn run() {
    let _cli = Cli::parse_args();
    serve().await;
}

/// Serve the language server over stdio until the client disconnects.
///
/// Installs the stderr tracing subscriber, builds the service (with the
/// `aozora/*` custom methods), and serves. This is [`run`] minus argv parsing:
/// the `aozora` umbrella binary parses argv itself (so `aozora lsp --help`
/// works and stdout stays clean) and then calls `serve` directly, while the
/// standalone `aozora-lsp` binary goes through `run`.
pub async fn serve() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .with_writer(io::stderr)
        .init();

    let stdin = stdin();
    let stdout = stdout();
    let (service, socket) = build_service();
    // tower-lsp's default concurrency cap is 4. After a didChange, VS Code
    // routinely fires 5+ concurrent requests (codeAction, inlayHint,
    // renderHtml, plus repeat codeActions either side of the cursor); the
    // 5th+ would queue behind the first four and surface as latency on
    // otherwise µs handlers. 32 keeps every realistic burst inside the
    // parallel window, and none of our handlers hold an executor thread
    // beyond their own work, so the higher cap is essentially free.
    Server::new(stdin, stdout, socket)
        .concurrency_level(32)
        .serve(service)
        .await;
}

/// Internal API surface — re-exported here **only** for the crate's own
/// integration tests, benches, examples, and fuzz targets, which compile as
/// separate crates and so can reach `pub` items but not `pub(crate)` ones.
///
/// This module is `#[doc(hidden)]` and is **not** part of the public API: it
/// carries no semver guarantee and anything in it may change or vanish in any
/// release. Public-surface tools (`cargo public-api`, `cargo semver-checks`)
/// skip `#[doc(hidden)]` items, so the stable surface stays [`Cli`] + [`run`].
///
/// The targets that consume it are gated on the `internals` Cargo feature
/// (see `Cargo.toml`), so a plain `cargo test` skips them; CI runs
/// `cargo test --features internals`.
#[doc(hidden)]
pub mod internals {
    pub use crate::code_actions::wrap_selection_actions;
    pub use crate::commands::{COMMAND_CANONICALIZE_SLUG, canonicalize_slug_edit};
    pub use crate::completion::completion_at;
    pub use crate::diagnostics::{diagnostics_for_source, diagnostics_from_aozora};
    pub use crate::doc_line_view::DocLineView;
    pub use crate::document_symbol::document_symbols;
    pub use crate::folding_range::folding_ranges;
    pub use crate::formatting::format_edits;
    pub use crate::gaiji_spans::{GaijiSpan, extract_gaiji_spans_from_tree};
    pub use crate::half_width_emmet::emmet_completions;
    pub use crate::hover::hover_at;
    pub use crate::inlay_hints::inlay_hints;
    pub use crate::line_index::LineIndex;
    pub use crate::linked_editing::linked_editing_at;
    pub use crate::on_type_formatting::{TRIGGERS as ON_TYPE_TRIGGERS, format_on_type};
    pub use crate::paragraph::{MAX_PARAGRAPH_BYTES, ParagraphBuffer, ParagraphSnapshot};
    pub use crate::parse_cache::{ParseCache, ReparseStats};
    pub use crate::position::{byte_offset_to_position, position_to_byte_offset};
    pub use crate::rename::{prepare_rename_at, rename_edit};
    pub use crate::semantic_tokens::{legend as semantic_token_legend, semantic_tokens_full};
    pub use crate::state::{DocBuffer, DocSnapshot, OpenDocument};
    pub use crate::structured_snippets::snippet_completions;
    pub use crate::text_edit::{ByteEdit, EditError, apply_edits};
    pub use crate::tree_sitter_doc::{TreeSitterDoc, input_edit};
}
