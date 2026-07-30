//! The in-process aozora language server (the `aozora lsp` subcommand).
//!
//! A `tower-lsp` server over the `aozora` crate: its parse tree, its canonical
//! `parse ∘ serialize` form, and gaiji resolution. What the
//! server advertises is `capabilities.rs` — pinned by a snapshot test, so this
//! page does not keep a second list of it.
//!
//! [`serve`] serves LSP over stdio until the client disconnects; [`run`] is the
//! `aozora lsp` subcommand — it spins up the tokio runtime and awaits it. Every
//! building block behind the handlers is a private sibling module.
#![allow(
    missing_docs,
    reason = "the language-server building blocks are private in-binary modules; \
              their pub items are reached only by sibling handlers and in-module tests"
)]

use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;

mod backend;
mod capabilities;
mod code_actions;
mod commands;
mod completion;
mod diagnostics;
#[cfg(test)]
mod differential;
mod doc_line_view;
mod document_limit;
mod document_symbol;
mod folding_range;
mod formatting;
#[cfg(test)]
mod fuzz_regressions;
mod gaiji_spans;
#[cfg(test)]
mod guardian;
mod half_width_emmet;
mod hover;
mod line_index;
mod linked_editing;
mod metrics;
mod on_type_formatting;
mod position;
#[cfg(test)]
mod property_invariants;
mod rename;
mod semantic_tokens;
mod server_locale;
#[cfg(test)]
mod smoke;
#[cfg(test)]
mod snapshots;
mod state;
mod structured_snippets;
mod text_edit;

use tokio::io::{stdin, stdout};
use tokio::runtime::Builder;
use tower_lsp::{ClientSocket, LspService, Server};

use crate::lsp::backend::AozoraLanguageServer;

/// Build the `LspService` with the aozora custom methods (`aozora/renderHtml`,
/// `aozora/gaijiSpans`) wired onto the builder — tower-lsp's `LanguageServer`
/// trait only covers spec-defined methods, so custom ones go here.
///
/// Factored out of [`serve`] so the in-module end-to-end harness
/// (`backend::e2e`) builds the server exactly the way the daemon does; the
/// custom-method routing is therefore exercised by tests and can't silently
/// drift from production.
pub(crate) fn build_service() -> (LspService<AozoraLanguageServer>, ClientSocket) {
    LspService::build(AozoraLanguageServer::new)
        .custom_method("aozora/renderHtml", AozoraLanguageServer::render_html)
        .custom_method("aozora/gaijiSpans", AozoraLanguageServer::gaiji_spans)
        .finish()
}

/// Serve the language server over stdio until the client disconnects.
///
/// Builds the service (with the `aozora/*` custom methods) and serves. The
/// `aozora lsp` subcommand awaits this on the tokio runtime; the stderr
/// tracing subscriber is installed once by the CLI shell (`crate::logging`)
/// before dispatch, and stdout carries the JSON-RPC wire protocol.
pub(crate) async fn serve() {
    let stdin = stdin();
    let stdout = stdout();
    let (service, socket) = build_service();
    // tower-lsp's default concurrency cap is 4. After a didChange, VS Code
    // routinely fires 5+ concurrent requests (codeAction, gaijiSpans,
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

/// `aozora lsp [--stdio]` — run the aozora language server in-process, speaking
/// LSP over stdio until the client disconnects.
///
/// `--stdio` is accepted (and ignored) for editor compatibility: stdio is the
/// only supported transport, so the flag is a no-op. The stderr tracing
/// subscriber and human-message defaults are already installed by the CLI shell
/// before this runs; stdout carries nothing but the JSON-RPC wire protocol.
#[derive(Debug, Parser)]
#[command(
    long_about = "Run the aozora language server. Speaks LSP over stdio: stdout \
                  carries the JSON-RPC wire protocol and logs go to stderr.\n\n\
                  Environment variables:\n  \
                  AOZORA_LOG                tracing filter, e.g. `aozora_cli=debug` (default: warn).\n  \
                  AOZORA_LSP_SLOW_PARSE_US  per-parse latency in microseconds above which a slow-parse warning is logged (default: 100000)."
)]
pub(crate) struct LspArgs {
    /// Speak LSP over stdio. Accepted for editor compatibility; this is the
    /// only supported transport, so the flag is a no-op.
    #[arg(long)]
    stdio: bool,
}

/// Build a multi-threaded tokio runtime and drive [`serve`] to completion.
///
/// The CLI shell's `main` is synchronous, so the runtime is created here rather
/// than via `#[tokio::main]`; the server owns the process for the editor
/// session's lifetime and returns success when the client disconnects.
pub(crate) fn run(_args: &LspArgs) -> Result<ExitCode> {
    let runtime = Builder::new_multi_thread().enable_all().build()?;
    runtime.block_on(serve());
    Ok(ExitCode::SUCCESS)
}
