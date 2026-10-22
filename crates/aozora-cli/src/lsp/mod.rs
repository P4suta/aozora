//! Serves the language server over stdio through [`run`] and [`serve`].

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

const REQUEST_CONCURRENCY: usize = 32;

/// Builds the service with standard and aozora-specific methods.
pub(crate) fn build_service() -> (LspService<AozoraLanguageServer>, ClientSocket) {
    LspService::build(AozoraLanguageServer::new)
        .custom_method("aozora/renderHtml", AozoraLanguageServer::render_html)
        .custom_method("aozora/gaijiSpans", AozoraLanguageServer::gaiji_spans)
        .finish()
}

/// Serves the language server over stdio until the client disconnects.
pub(crate) async fn serve() {
    let stdin = stdin();
    let stdout = stdout();
    let (service, socket) = build_service();
    Server::new(stdin, stdout, socket)
        .concurrency_level(REQUEST_CONCURRENCY)
        .serve(service)
        .await;
}

/// Run the aozora language server over stdio.
#[derive(Debug, Parser)]
#[command(
    long_about = "Run the aozora language server. Speaks LSP over stdio: stdout \
                  carries the JSON-RPC wire protocol and logs go to stderr.\n\n\
                  Environment variables:\n  \
                  AOZORA_LOG                tracing filter, e.g. `aozora_cli=debug` (default: warn).\n  \
                  AOZORA_LSP_SLOW_PARSE_US  per-parse latency in microseconds above which a slow-parse warning is logged (default: 100000)."
)]
pub(crate) struct LspArgs {
    /// Speak the Language Server Protocol over stdio.
    /// Accepts this flag for editor compatibility because stdio is the only transport.
    #[arg(long)]
    stdio: bool,
}

pub(crate) fn run(_args: &LspArgs) -> Result<ExitCode> {
    let runtime = Builder::new_multi_thread().enable_all().build()?;
    runtime.block_on(serve());
    Ok(ExitCode::SUCCESS)
}
