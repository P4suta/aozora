#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "embedded locale resources and identifiers are build-time constants"
)]

use std::slice::from_ref;
use std::sync::LazyLock;

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentError, FluentResource};
use fluent_langneg::{NegotiationStrategy, negotiate_languages};

pub(crate) use fluent_bundle::FluentArgs;
pub(crate) use unic_langid::LanguageIdentifier;

const SOURCES: &[(&str, &str)] = &[
    ("en", include_str!("../locales/en.ftl")),
    ("ja", include_str!("../locales/ja.ftl")),
    ("zh", include_str!("../locales/zh.ftl")),
];

type Bundle = FluentBundle<FluentResource>;

struct Catalog {
    lang: LanguageIdentifier,
    bundle: Bundle,
}

static CATALOGS: LazyLock<Vec<Catalog>> = LazyLock::new(|| {
    SOURCES
        .iter()
        .map(|&(tag, ftl)| build_catalog(tag, ftl))
        .collect()
});

static AVAILABLE: LazyLock<Vec<LanguageIdentifier>> =
    LazyLock::new(|| CATALOGS.iter().map(|c| c.lang.clone()).collect());

fn english() -> LanguageIdentifier {
    "en".parse().expect("`en` is a valid language identifier")
}

fn build_catalog(tag: &str, ftl: &str) -> Catalog {
    let lang: LanguageIdentifier = tag
        .parse()
        .unwrap_or_else(|_| panic!("built-in locale tag `{tag}` is a valid language identifier"));
    let resource = FluentResource::try_new(ftl.to_owned())
        .unwrap_or_else(|(_, errors)| panic!("built-in `{tag}.ftl` has parse errors: {errors:?}"));
    let mut bundle = Bundle::new_concurrent(vec![lang.clone()]);
    bundle.set_use_isolating(false);
    bundle
        .add_resource(resource)
        .unwrap_or_else(|errors| panic!("built-in `{tag}.ftl` has key collisions: {errors:?}"));
    Catalog { lang, bundle }
}

#[must_use]
pub(crate) fn resolve(
    explicit: Option<&str>,
    aozora_lang_env: Option<&str>,
    config_lang: Option<&str>,
    sys_lang: Option<&str>,
) -> LanguageIdentifier {
    for source in [explicit, aozora_lang_env, config_lang, sys_lang] {
        let Some(raw) = source else { continue };
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        return parse_locale(trimmed).map_or_else(english, |requested| negotiate(&requested));
    }
    english()
}

fn parse_locale(raw: &str) -> Option<LanguageIdentifier> {
    let head = raw.split(['.', '@']).next().unwrap_or(raw);
    head.replace('_', "-").parse().ok()
}

fn negotiate(requested: &LanguageIdentifier) -> LanguageIdentifier {
    let default = english();
    let negotiated = negotiate_languages(
        from_ref(requested),
        &AVAILABLE,
        Some(&default),
        NegotiationStrategy::Filtering,
    );
    negotiated
        .first()
        .map_or_else(|| default.clone(), |best| (*best).clone())
}

#[must_use]
pub(crate) fn t(lang: &LanguageIdentifier, key: &str) -> String {
    lookup(lang, key, None)
}

#[must_use]
pub(crate) fn tf(lang: &LanguageIdentifier, key: &str, args: &FluentArgs<'_>) -> String {
    lookup(lang, key, Some(args))
}

#[must_use]
pub(crate) fn is_english(lang: &LanguageIdentifier) -> bool {
    *lang == english()
}

fn diag_slug(code: &str) -> String {
    code.rsplit_once("::")
        .map_or(code, |(_, tail)| tail)
        .replace('_', "-")
}

#[must_use]
pub(crate) fn diag_title(lang: &LanguageIdentifier, code: &str) -> String {
    t(lang, &format!("diag-{}-title", diag_slug(code)))
}

#[must_use]
pub(crate) fn diag_body(lang: &LanguageIdentifier, code: &str, args: &FluentArgs<'_>) -> String {
    tf(lang, &format!("diag-{}-body", diag_slug(code)), args)
}

fn lookup(lang: &LanguageIdentifier, key: &str, args: Option<&FluentArgs<'_>>) -> String {
    if let Some(text) = format_from(catalog_for(lang), key, args) {
        return text;
    }
    if let Some(text) = format_from(en_catalog(), key, args) {
        return text;
    }
    key.to_owned()
}

fn format_from(catalog: &Catalog, key: &str, args: Option<&FluentArgs<'_>>) -> Option<String> {
    let message = catalog.bundle.get_message(key)?;
    let pattern = message.value()?;
    let mut errors: Vec<FluentError> = Vec::new();
    let formatted = catalog.bundle.format_pattern(pattern, args, &mut errors);
    Some(formatted.into_owned())
}

fn catalog_for(lang: &LanguageIdentifier) -> &'static Catalog {
    CATALOGS
        .iter()
        .find(|c| &c.lang == lang)
        .unwrap_or_else(|| en_catalog())
}

fn en_catalog() -> &'static Catalog {
    &CATALOGS[0]
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use aozora::Diagnostic;
    use fluent_syntax::ast;

    use super::*;

    fn lang(tag: &str) -> LanguageIdentifier {
        tag.parse().expect("test locale tag parses")
    }

    #[test]
    fn every_builtin_catalog_parses() {
        assert_eq!(CATALOGS.len(), 3, "en / ja / zh catalogs are all built");
        assert_eq!(AVAILABLE.len(), 3);
    }

    fn message_ids(tag: &str, ftl: &str) -> BTreeSet<String> {
        let resource = FluentResource::try_new(ftl.to_owned())
            .unwrap_or_else(|(_, errors)| panic!("`{tag}.ftl` has parse errors: {errors:?}"));
        resource
            .entries()
            .filter_map(|entry| match entry {
                ast::Entry::Message(message) => Some(message.id.name.to_owned()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn every_locale_defines_the_same_message_ids() {
        let catalogs: Vec<(&str, BTreeSet<String>)> = SOURCES
            .iter()
            .map(|&(tag, ftl)| (tag, message_ids(tag, ftl)))
            .collect();
        let (canonical_tag, canonical) = &catalogs[0];

        assert!(
            !canonical.is_empty(),
            "{canonical_tag}.ftl parsed to zero messages"
        );

        for (tag, ids) in &catalogs[1..] {
            let missing: Vec<_> = canonical.difference(ids).collect();
            assert!(
                missing.is_empty(),
                "{tag}.ftl is missing {missing:?} — `lookup` would fall through \
                 to {canonical_tag}, so a {tag} reader silently gets \
                 {canonical_tag} prose with nothing to signal the gap"
            );
            let dead: Vec<_> = ids.difference(canonical).collect();
            assert!(
                dead.is_empty(),
                "{tag}.ftl defines {dead:?}, absent from the canonical \
                 {canonical_tag}.ftl — nothing can ever look them up"
            );
        }

        for (tag, ids) in &catalogs {
            let catalog = catalog_for(&lang(tag));
            for id in ids {
                assert!(
                    catalog
                        .bundle
                        .get_message(id)
                        .is_some_and(|message| message.value().is_some()),
                    "{tag}.ftl: `{id}` resolves to no value"
                );
            }
        }
    }

    #[test]
    fn resolve_prefers_explicit_over_all_lower_sources() {
        let got = resolve(Some("ja"), Some("zh"), Some("en"), Some("zh_CN.UTF-8"));
        assert_eq!(got, lang("ja"), "--lang wins over env / config / LANG");
    }

    #[test]
    fn resolve_env_beats_config_and_lang() {
        let got = resolve(None, Some("zh"), Some("en"), Some("ja_JP.UTF-8"));
        assert_eq!(got, lang("zh"), "AOZORA_LANG wins over config.lang / LANG");
    }

    #[test]
    fn resolve_config_beats_lang() {
        let got = resolve(None, None, Some("ja"), Some("zh_CN.UTF-8"));
        assert_eq!(got, lang("ja"), "config.lang wins over LANG");
    }

    #[test]
    fn resolve_uses_lang_as_lowest_priority() {
        let got = resolve(None, None, None, Some("zh_CN.UTF-8"));
        assert_eq!(got, lang("zh"), "LANG is the last message-language source");
    }

    #[test]
    fn resolve_defaults_to_english_when_nothing_set() {
        assert_eq!(resolve(None, None, None, None), lang("en"));
    }

    #[test]
    fn resolve_treats_blank_sources_as_absent() {
        let got = resolve(Some("   "), Some(""), None, Some("ja_JP.UTF-8"));
        assert_eq!(got, lang("ja"), "blank sources fall through to LANG");
    }

    #[test]
    fn resolve_unknown_locale_falls_back_to_english_without_falling_through() {
        let got = resolve(Some("xx"), None, Some("ja"), None);
        assert_eq!(got, lang("en"), "unknown --lang → en, not the config's ja");
    }

    #[test]
    fn resolve_negotiates_region_and_script_to_the_base_language() {
        assert_eq!(resolve(Some("zh-Hans-CN"), None, None, None), lang("zh"));
        assert_eq!(resolve(Some("en-GB"), None, None, None), lang("en"));
        assert_eq!(resolve(Some("ja-JP"), None, None, None), lang("ja"));
    }

    #[test]
    fn resolve_posix_c_and_posix_locales_are_english() {
        assert_eq!(resolve(None, None, None, Some("C")), lang("en"));
        assert_eq!(resolve(None, None, None, Some("POSIX")), lang("en"));
    }

    #[test]
    fn stdin_empty_english_bytes_are_exact() {
        let mut args = FluentArgs::new();
        args.set("cmd", "check");
        let text = tf(&lang("en"), "stdin-empty", &args);
        assert_eq!(
            text,
            concat!(
                "error: standard input is empty (reading from a terminal)\n",
                "  hint: read a file →  aozora check <FILE>\n",
                "        or a pipe   →  cat f.txt | aozora check\n",
                "  all commands:  aozora --help",
            ),
        );
    }

    #[test]
    fn stdin_empty_japanese_bytes_are_exact() {
        let mut args = FluentArgs::new();
        args.set("cmd", "check");
        let text = tf(&lang("ja"), "stdin-empty", &args);
        assert_eq!(
            text,
            concat!(
                "error: 標準入力が空です (端末から実行中)\n",
                "  ヒント: ファイルを →  aozora check <FILE>\n",
                "          パイプで   →  cat f.txt | aozora check\n",
                "  全機能:  aozora --help",
            ),
        );
    }

    #[test]
    fn watch_banner_interpolates_the_path_without_bidi_isolates() {
        let text = tf_path("en", "doc.txt");
        assert_eq!(text, "── watching doc.txt (Ctrl-C to stop) ──");
        assert!(!text.contains('\u{2068}') && !text.contains('\u{2069}'));
    }

    fn tf_path(tag: &str, path: &str) -> String {
        let mut args = FluentArgs::new();
        args.set("path", path.to_owned());
        tf(&lang(tag), "watch-banner", &args)
    }

    #[test]
    fn missing_key_falls_back_to_english_then_to_the_key() {
        assert_eq!(t(&lang("ja"), "no-such-key"), "no-such-key");
    }

    #[test]
    fn explain_labels_differ_by_locale() {
        assert_eq!(t(&lang("en"), "explain-repro-label"), "Reproduction:");
        assert_eq!(t(&lang("ja"), "explain-repro-label"), "再現例:");
        assert_eq!(t(&lang("zh"), "explain-repro-label"), "复现示例:");
    }

    #[test]
    fn every_diagnostic_code_has_prose_in_the_canonical_catalog() {
        let (canonical_tag, _) = SOURCES[0];
        let l = lang(canonical_tag);
        for code in Diagnostic::ALL_CODES {
            let slug = diag_slug(code);
            let title = diag_title(&l, code);
            assert_ne!(title, format!("diag-{slug}-title"), "{code} title");
            assert!(!title.trim().is_empty(), "{code} empty title");
            let body = diag_body(&l, code, &FluentArgs::new());
            assert_ne!(body, format!("diag-{slug}-body"), "{code} body");
            assert!(!body.trim().is_empty(), "{code} empty body");
        }
    }

    #[test]
    fn diag_slug_takes_the_trailing_segment_and_kebabs_underscores() {
        assert_eq!(
            diag_slug("aozora::lex::unclosed_bracket"),
            "unclosed-bracket"
        );
        assert_eq!(
            diag_slug("aozora::lint::non_canonical_directive"),
            "non-canonical-directive"
        );
        assert_eq!(diag_slug("nested_ruby"), "nested-ruby");
    }

    #[test]
    fn diag_title_localizes_by_locale() {
        let code = "aozora::lex::unclosed_bracket";
        assert_eq!(diag_title(&lang("en"), code), "Unclosed opening bracket");
        assert_eq!(diag_title(&lang("ja"), code), "閉じられていない開き括弧");
        assert_eq!(diag_title(&lang("zh"), code), "未闭合的开括号");
    }

    #[test]
    fn diag_body_interpolates_instance_args_without_bidi_isolates() {
        let mut args = FluentArgs::new();
        args.set("open", "［");
        args.set("close", "］");
        args.set("example", "［＃改ページ］");
        let body = diag_body(&lang("en"), "aozora::lex::unclosed_bracket", &args);
        assert!(body.contains('［') && body.contains('］'), "glyphs: {body}");
        assert!(body.contains("［＃改ページ］"), "example woven in: {body}");
        assert!(
            !body.contains('\u{2068}') && !body.contains('\u{2069}'),
            "no bidi isolates: {body:?}"
        );
        assert!(body.contains("\n\n"), "paragraphs preserved: {body:?}");
    }

    #[test]
    fn diag_body_missing_locale_key_falls_back_to_english() {
        let code = "aozora::lex::does_not_exist";
        assert_eq!(
            diag_body(&lang("zh"), code, &FluentArgs::new()),
            "diag-does-not-exist-body"
        );
    }

    #[test]
    fn is_english_matches_only_the_english_base() {
        assert!(is_english(&lang("en")));
        assert!(!is_english(&lang("ja")));
        assert!(!is_english(&lang("zh")));
        assert!(is_english(&resolve(Some("en-US"), None, None, None)));
    }
}
