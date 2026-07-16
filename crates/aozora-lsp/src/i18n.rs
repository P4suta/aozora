//! The server's UI language, resolved from the `initialize` handshake and
//! shared by every handler.
//!
//! Unlike the CLI — which resolves `--lang > AOZORA_LANG > config.lang > LANG`
//! per invocation — the language server has no `--lang` flag and no project
//! config to consult. What it has instead is the client: every LSP client
//! reports its own UI language as `InitializeParams::locale`, and an editor's
//! language setting is a far better signal than the shell environment the
//! daemon happened to be launched from (`LANG` is routinely unset on macOS and
//! Windows). So the chain here is `AOZORA_LANG > client locale > LANG > en`,
//! with `AOZORA_LANG` still on top as the explicit escape hatch for a user who
//! wants the server in a language their editor is not in.
//!
//! [`init_ui_lang`] fixes the value, once, from [`crate::backend`]'s
//! `initialize` handler; [`ui_lang`] is what every later read goes through. A
//! client that never handshakes — or one that omits `locale` — still gets a
//! language: the first [`ui_lang`] read resolves from the environment alone.
//!
//! The pure provider functions (`hover_at`, `diagnostics_from_aozora`, the
//! code-action / completion builders, …) take an explicit
//! `&LanguageIdentifier` so they stay locale-parameterised and unit-testable
//! in isolation; only the [`crate::backend`] handlers reach for [`ui_lang`],
//! the single resolved value they thread down. The lookup itself goes through
//! the shared [`aozora_i18n`] catalog — the same `.ftl` bundles the CLI uses,
//! so there is one place to translate a string, not two.

use std::env;
use std::sync::OnceLock;

use aozora_i18n::LanguageIdentifier;

/// The server's UI language, decided by whichever of [`init_ui_lang`] /
/// [`ui_lang`] runs first.
static UI_LANG: OnceLock<LanguageIdentifier> = OnceLock::new();

/// Resolve `AOZORA_LANG > client_locale > LANG > en`.
///
/// The client's handshake locale takes the shared chain's `config_lang` slot:
/// below the explicit `AOZORA_LANG` override, above the OS `LANG`. Clients
/// send both BCP-47 (`zh-Hans`) and POSIX (`zh_CN.UTF-8`) shapes and
/// occasionally a tag we have no catalog for; `aozora_i18n::resolve` parses,
/// negotiates, and falls back to English for all of it, so no mapping table
/// lives here.
fn resolve(client_locale: Option<&str>) -> LanguageIdentifier {
    // Bind the owned `String`s so their `as_deref()` borrows outlive the call
    // (a temporary would be dropped before `resolve` reads it). The LSP has no
    // `--lang` flag, so the chain's top slot is simply absent here.
    let aozora_lang = env::var("AOZORA_LANG").ok();
    let sys_lang = env::var("LANG").ok();
    aozora_i18n::resolve(
        None,
        aozora_lang.as_deref(),
        client_locale,
        sys_lang.as_deref(),
    )
}

/// Fix the server's UI language from the client's `initialize` locale.
///
/// Called from the `initialize` handler, before any request handler can run.
/// Later calls are no-ops: a language server serves one client for its
/// lifetime, so the first handshake decides.
pub(crate) fn init_ui_lang(client_locale: Option<&str>) {
    _ = UI_LANG.get_or_init(|| resolve(client_locale));
}

/// The server's resolved UI language.
///
/// Every backend handler passes this to the locale-parameterised provider it
/// calls, so the whole server speaks one language for its lifetime. A read
/// that beats `init_ui_lang` — a client that skips the handshake — resolves
/// from the environment alone rather than failing.
#[must_use]
pub fn ui_lang() -> &'static LanguageIdentifier {
    UI_LANG.get_or_init(|| resolve(None))
}
