use std::env;
use std::sync::OnceLock;

use crate::i18n::{LanguageIdentifier, resolve as negotiate};

static UI_LANG: OnceLock<LanguageIdentifier> = OnceLock::new();

fn resolve(client_locale: Option<&str>) -> LanguageIdentifier {
    let aozora_lang = env::var("AOZORA_LANG").ok();
    let sys_lang = env::var("LANG").ok();
    negotiate(
        None,
        aozora_lang.as_deref(),
        client_locale,
        sys_lang.as_deref(),
    )
}

pub(crate) fn init_ui_lang(client_locale: Option<&str>) {
    _ = UI_LANG.get_or_init(|| resolve(client_locale));
}

#[must_use]
pub(super) fn ui_lang() -> &'static LanguageIdentifier {
    UI_LANG.get_or_init(|| resolve(None))
}
