//! The server's UI language, resolved once at startup and shared by every
//! handler.
//!
//! Unlike the CLI — which resolves `--lang > AOZORA_LANG > config.lang > LANG`
//! per invocation — the language server is a long-lived daemon with no `--lang`
//! flag and no project config to consult. Its only inputs are the process
//! environment it was launched with: `AOZORA_LANG` (the explicit override) and
//! `LANG` (the POSIX locale), in that order, falling back to English.
//!
//! Resolution happens exactly once, on first use, via a process-wide
//! [`LazyLock`]. The editor launches one `aozora-lsp` per workspace and its
//! environment does not change under it, so a single resolution at startup is
//! both correct and cheaper than re-reading the environment on every hover.
//!
//! The pure provider functions (`hover_at`, `diagnostics_from_aozora`, the
//! code-action / completion builders, …) take an explicit
//! `&LanguageIdentifier` so they stay locale-parameterised and unit-testable
//! in isolation; only the [`crate::backend`] handlers reach for [`ui_lang`],
//! the single resolved value they thread down. The lookup itself goes through
//! the shared [`aozora_i18n`] catalog — the same `.ftl` bundles the CLI uses,
//! so there is one place to translate a string, not two.

use std::env;
use std::sync::LazyLock;

use aozora_i18n::LanguageIdentifier;

/// The server's UI language, resolved once from the environment.
static UI_LANG: LazyLock<LanguageIdentifier> = LazyLock::new(|| {
    // Bind the owned `String`s so their `as_deref()` borrows outlive the call
    // (a temporary would be dropped before `resolve` reads it). The LSP has no
    // `--lang` flag and no config file, so the two higher precedence sources
    // the CLI consults are simply absent here.
    let aozora_lang = env::var("AOZORA_LANG").ok();
    let sys_lang = env::var("LANG").ok();
    aozora_i18n::resolve(None, aozora_lang.as_deref(), None, sys_lang.as_deref())
});

/// The server's resolved UI language — `AOZORA_LANG > LANG > en`, decided once
/// at startup.
///
/// Every backend handler passes this to the locale-parameterised provider it
/// calls, so the whole server speaks one language for its lifetime.
#[must_use]
pub(crate) fn ui_lang() -> &'static LanguageIdentifier {
    &UI_LANG
}
