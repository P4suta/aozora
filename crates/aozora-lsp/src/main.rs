//! `aozora-lsp` daemon entry point. Speaks LSP over stdio.
//!
//! All the wiring lives in [`aozora_lsp::run`]; `main` only spins up the
//! tokio runtime and awaits it. Logging goes to stderr because stdout is
//! reserved for the LSP JSON-RPC wire protocol. Set
//! `RUST_LOG=aozora_lsp=debug` (or similar) to see tracing events; the
//! default filter is `warn` so quiet editor integrations stay quiet.
//!
//! This binary can be invoked directly (`aozora-lsp --stdio`) or reached
//! through the main CLI: `aozora lsp` locates and execs this daemon,
//! forwarding argv. Either entry point runs the same server.

#![forbid(unsafe_code)]

#[tokio::main]
async fn main() {
    aozora_lsp::run().await;
}
