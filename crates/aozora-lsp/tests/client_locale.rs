//! The `initialize` handshake decides the server's UI language.
//!
//! This assertion owns a whole test binary because the resolved language is a
//! process-global fixed by whoever asks first: every other test handshakes
//! without a locale, so sharing their process would make the outcome depend on
//! test order. One process, one handshake, one language — deterministic.

use aozora_lsp::internals::{AozoraLanguageServer, LanguageIdentifier, ui_lang};
use tower_lsp::lsp_types::InitializeParams;
use tower_lsp::{LanguageServer, LspService};

/// A client that reports itself as Japanese gets a Japanese server, whatever
/// the daemon's own environment says. `LANG` is unset on a typical macOS /
/// Windows editor host, so resolving from the environment alone answered a
/// Japanese editor in English — the language the client sends is the only
/// signal that is actually about the reader.
#[tokio::test]
async fn initialize_locale_decides_the_ui_language() {
    let (service, _socket) = LspService::new(AozoraLanguageServer::new);
    let params = InitializeParams {
        locale: Some("ja".to_owned()),
        ..InitializeParams::default()
    };
    service
        .inner()
        .initialize(params)
        .await
        .expect("initialize succeeds");

    let ja: LanguageIdentifier = "ja".parse().expect("ja parses");
    assert_eq!(
        *ui_lang(),
        ja,
        "the client's locale must decide the server's UI language",
    );
}
