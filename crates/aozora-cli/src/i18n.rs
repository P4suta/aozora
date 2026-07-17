//! Localization layer for the `aozora` binary (CLI shell + in-process LSP).
//!
//! A thin, function-based wrapper over the [Project Fluent] runtime. The
//! human-facing shell strings live in external `locales/<locale>.ftl`
//! catalogs (English canonical, Japanese, Chinese) embedded at compile time;
//! adding a language is a matter of dropping in one more `.ftl` and one entry
//! in this crate's source locale table — no enum to extend.
//!
//! The public surface is deliberately tiny:
//!
//! - [`resolve`] turns the CLI's four language sources (`--lang`,
//!   `AOZORA_LANG`, `config.lang`, `LANG`) into a concrete
//!   [`LanguageIdentifier`], negotiating the request against the available
//!   catalogs and falling back to English for anything unknown.
//! - [`t`] and [`tf`] look a key up in a resolved language's bundle, with an
//!   English fallback for any key a translation happens to be missing.
//!
//! What this layer is **not** for: the machine output axis. Diagnostic codes,
//! JSON envelopes, `short` lines, exit codes, and the schema / timing JSON
//! are byte-stable English contracts and never pass through here. See
//! `docs/adr/0033-cli-output-language-policy.md`.
//!
//! [Project Fluent]: https://projectfluent.org/

use std::slice::from_ref;
use std::sync::LazyLock;

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentError, FluentResource};
use fluent_langneg::{NegotiationStrategy, negotiate_languages};

pub(crate) use fluent_bundle::FluentArgs;
pub(crate) use unic_langid::LanguageIdentifier;

/// The embedded Fluent resources, one per available locale, as
/// `(BCP-47 tag, catalog bytes)`. The order is the negotiation preference
/// order and the first entry is the canonical fallback locale (English).
/// Adding a locale is exactly one line here plus its `.ftl` file.
const SOURCES: &[(&str, &str)] = &[
    ("en", include_str!("../locales/en.ftl")),
    ("ja", include_str!("../locales/ja.ftl")),
    ("zh", include_str!("../locales/zh.ftl")),
];

/// A concurrent Fluent bundle: `new_concurrent` picks the `Sync` memoizer so a
/// bundle can live in a process-wide `static`, and `set_use_isolating(false)`
/// keeps interpolated arguments free of the U+2068/U+2069 bidi isolates Fluent
/// inserts by default — those would corrupt the byte-exact terminal output the
/// CLI's tests pin.
type Bundle = FluentBundle<FluentResource>;

/// One available locale: its identifier plus its parsed message bundle.
struct Catalog {
    lang: LanguageIdentifier,
    bundle: Bundle,
}

/// The parsed catalogs, built once on first use. Panics if a *built-in* `.ftl`
/// fails to parse — that is an authoring error in this crate, caught by the
/// unit tests, never a runtime condition driven by user input.
static CATALOGS: LazyLock<Vec<Catalog>> = LazyLock::new(|| {
    SOURCES
        .iter()
        .map(|&(tag, ftl)| build_catalog(tag, ftl))
        .collect()
});

/// The available locale identifiers, in preference order — the negotiation
/// pool [`resolve`] matches a request against.
static AVAILABLE: LazyLock<Vec<LanguageIdentifier>> =
    LazyLock::new(|| CATALOGS.iter().map(|c| c.lang.clone()).collect());

/// Parse `en` — the canonical fallback locale. Built by parsing rather than
/// the `langid!` macro because that macro expands to `unsafe` and this crate
/// is `#![forbid(unsafe_code)]`.
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

/// Resolve the message language from the CLI's precedence chain
/// (`explicit` = `--lang` > `AOZORA_LANG` > `config.lang` > `LANG`), falling
/// back to English when nothing decides.
///
/// The first source that is present and non-blank decides the language: its
/// value is parsed (POSIX `ja_JP.UTF-8` shapes included), negotiated against
/// the available catalogs, and — if unknown or unparsable — resolved to
/// English. A blank source is treated as absent and the chain falls through.
/// `LANG` is consulted only for *message* language here; source-byte encoding
/// never reads it (see `docs/adr/0033-cli-output-language-policy.md`).
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

/// Parse a locale string into a [`LanguageIdentifier`], accepting both BCP-47
/// (`zh-Hans`, `en-US`) and POSIX (`ja_JP.UTF-8`, `en_US@euro`) shapes: the
/// `.<charset>` and `@<modifier>` suffixes are dropped and `_` is swapped for
/// `-` before parsing. `None` when the head is not a valid language subtag
/// (`C`, `POSIX`, an empty string) — callers then fall back to English.
fn parse_locale(raw: &str) -> Option<LanguageIdentifier> {
    let head = raw.split(['.', '@']).next().unwrap_or(raw);
    head.replace('_', "-").parse().ok()
}

/// Negotiate `requested` against the available catalogs with English as the
/// default, returning the best match (always English when nothing matches).
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

/// Look up `key` in `lang`'s catalog, with an English fallback.
///
/// Falls back to English for a key the translation is missing, and returns
/// the key itself if no catalog defines it — a loud, greppable signal of a
/// catalog gap rather than a silent empty string. For messages with `{$arg}`
/// placeables use [`tf`].
#[must_use]
pub(crate) fn t(lang: &LanguageIdentifier, key: &str) -> String {
    lookup(lang, key, None)
}

/// Look up `key` in `lang`'s catalog with `args` bound to the message's
/// placeables, English-fallback as in [`t`].
#[must_use]
pub(crate) fn tf(lang: &LanguageIdentifier, key: &str, args: &FluentArgs<'_>) -> String {
    lookup(lang, key, Some(args))
}

/// True when `lang` is the canonical English locale.
///
/// The one language for which a host keeps the byte-stable `#[error]` Display
/// as the human diagnostic headline instead of substituting a localized title
/// — so the English human report never moves. Every resolved language from
/// [`resolve`] is one of the available base tags, so this is an exact match.
#[must_use]
pub(crate) fn is_english(lang: &LanguageIdentifier) -> bool {
    *lang == english()
}

/// The Fluent message-key stem for a diagnostic `code`.
///
/// Its trailing `::` segment with `_` turned into `-`:
/// `aozora::lex::unclosed_bracket` → `unclosed-bracket`, so the catalog keys
/// are `diag-unclosed-bracket-title` and `diag-unclosed-bracket-body`. The
/// diagnostic code string thus doubles as the localization key, keeping
/// the `aozora` diagnostic codes and the `.ftl` catalogs in lock-step with no
/// separate mapping table.
fn diag_slug(code: &str) -> String {
    code.rsplit_once("::")
        .map_or(code, |(_, tail)| tail)
        .replace('_', "-")
}

/// The localized one-line title for a diagnostic `code` in `lang`.
///
/// Looks up `diag-<slug>-title`, where `slug` is `code`'s trailing `::`
/// segment with `_` turned into `-`; missing keys surface as the key itself, a
/// loud, greppable signal of a catalog gap.
#[must_use]
pub(crate) fn diag_title(lang: &LanguageIdentifier, code: &str) -> String {
    t(lang, &format!("diag-{}-title", diag_slug(code)))
}

/// The localized long-form body for a diagnostic `code` in `lang`.
///
/// `args` binds the body message's instance placeables (`$open`, `$close`,
/// `$example`, `$codepoint`, `$char`, `$open_kind`, `$close_kind`,
/// `$container`, `$open_family`, `$close_family`, `$canonical`). The consumer
/// builds `args` from the live diagnostic — `aozora_spec::Diagnostic::
/// body_args` yields exactly the `(name, value)` pairs a variant's body
/// interpolates (empty for static-body variants). Looks up `diag-<slug>-body`,
/// the slug derived from `code` as in [`diag_title`].
#[must_use]
pub(crate) fn diag_body(lang: &LanguageIdentifier, code: &str, args: &FluentArgs<'_>) -> String {
    tf(lang, &format!("diag-{}-body", diag_slug(code)), args)
}

fn lookup(lang: &LanguageIdentifier, key: &str, args: Option<&FluentArgs<'_>>) -> String {
    if let Some(text) = format_from(catalog_for(lang), key, args) {
        return text;
    }
    // The requested locale lacks the key — fall back to the English canonical
    // catalog before giving up.
    if let Some(text) = format_from(en_catalog(), key, args) {
        return text;
    }
    key.to_owned()
}

/// Format `key` from one catalog, or `None` when that catalog does not define
/// it (so the caller can fall through to English).
fn format_from(catalog: &Catalog, key: &str, args: Option<&FluentArgs<'_>>) -> Option<String> {
    let message = catalog.bundle.get_message(key)?;
    let pattern = message.value()?;
    let mut errors: Vec<FluentError> = Vec::new();
    // A formatting error (e.g. a missing argument) is a bug in our own
    // catalog, not a user condition; Fluent still returns a best-effort
    // string with the offending placeable left as `{$name}`, which is more
    // useful to surface than a panic.
    let formatted = catalog.bundle.format_pattern(pattern, args, &mut errors);
    Some(formatted.into_owned())
}

/// The catalog for `lang`, or the English catalog when `lang` is not one of
/// the available locales. [`resolve`] already negotiates to an available
/// locale, so the fallback is defensive.
fn catalog_for(lang: &LanguageIdentifier) -> &'static Catalog {
    CATALOGS
        .iter()
        .find(|c| &c.lang == lang)
        .unwrap_or_else(|| en_catalog())
}

/// The English canonical catalog — the first (and fallback) entry.
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

    // --- catalog integrity: every built-in `.ftl` parses and every key
    //     resolves in every locale (kills silent catalog gaps) ---

    #[test]
    fn every_builtin_catalog_parses() {
        // Forcing the lazy build here turns any `.ftl` authoring error into a
        // test failure rather than a first-use panic in production.
        assert_eq!(CATALOGS.len(), 3, "en / ja / zh catalogs are all built");
        assert_eq!(AVAILABLE.len(), 3);
    }

    /// Every message id `tag`'s catalog defines, read back out of the AST
    /// Fluent itself parsed, so the set is whatever the `.ftl` really says.
    ///
    /// Messages only. A term (`-name`) is catalog-internal — reachable only
    /// from patterns inside its own bundle, never a [`t`] / [`tf`] key — so a
    /// translation may factor out terms the canonical catalog has no use for.
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
        // Parity, never a key list: whatever the canonical catalog defines the
        // others must define, and vice versa. Both sides come from the
        // catalogs, so this covers every namespace — including ones nobody has
        // invented yet — where an enumeration only ever covers what someone
        // remembered to type.
        let catalogs: Vec<(&str, BTreeSet<String>)> = SOURCES
            .iter()
            .map(|&(tag, ftl)| (tag, message_ids(tag, ftl)))
            .collect();
        let (canonical_tag, canonical) = &catalogs[0];

        // "Nothing to check" must not read as "nothing wrong": were the walk to
        // stop seeing messages, every catalog would agree on the empty set and
        // the parity assertions below would pass while proving nothing.
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

        // Id parity alone would still admit a message carrying only attributes:
        // `lookup` formats `message.value()`, so a valueless message counts as a
        // miss however present its id is — it degrades to the canonical
        // catalog, or to the bare key when it is valueless there too.
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

    // --- resolve: the precedence chain and negotiation ---

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
        // A present-but-blank higher source is skipped, not taken as English.
        let got = resolve(Some("   "), Some(""), None, Some("ja_JP.UTF-8"));
        assert_eq!(got, lang("ja"), "blank sources fall through to LANG");
    }

    #[test]
    fn resolve_unknown_locale_falls_back_to_english_without_falling_through() {
        // A present-but-unknown higher source resolves to English; it does NOT
        // fall through to a lower, known source.
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
        // `C` / `POSIX` mean "no localization" — they must not localize.
        assert_eq!(resolve(None, None, None, Some("C")), lang("en"));
        assert_eq!(resolve(None, None, None, Some("POSIX")), lang("en"));
    }

    // --- t / tf: the byte-exact catalog values that back the CLI shell ---

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
        // `set_use_isolating(false)` must hold: no U+2068/U+2069 around $path.
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
        // A key absent from every catalog surfaces as itself.
        assert_eq!(t(&lang("ja"), "no-such-key"), "no-such-key");
    }

    #[test]
    fn explain_labels_differ_by_locale() {
        assert_eq!(t(&lang("en"), "explain-repro-label"), "Reproduction:");
        assert_eq!(t(&lang("ja"), "explain-repro-label"), "再現例:");
        assert_eq!(t(&lang("zh"), "explain-repro-label"), "复现示例:");
    }

    // --- diag_title / diag_body: the localized prose behind a code ---

    #[test]
    fn every_diagnostic_code_has_prose_in_the_canonical_catalog() {
        // Walk the live catalogue — every code `Diagnostic::code` can return —
        // and require prose for each. A code added to `ALL_CODES` without its
        // `.ftl` entries fails here instead of reaching `aozora explain` and
        // the Problems pane as a raw `diag-<slug>-title`.
        //
        // The canonical locale only, deliberately. `lookup` falls back to it,
        // so asking the same question of ja / zh through `diag_title` cannot
        // fail while the canonical catalog answers: the fallback supplies prose
        // and the assertion passes for a locale that defines nothing. Coverage
        // of the other locales is `every_locale_defines_the_same_message_ids`,
        // which reads the catalogs directly and can therefore see the gap.
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
        // A bare, unqualified token is passed through (still kebab-cased).
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
        // The unclosed-bracket body weaves in the delimiter glyphs and the
        // canonical example; `set_use_isolating(false)` must hold so the
        // interpolated values carry no U+2068/U+2069 around them.
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
        // The multiline body keeps paragraph breaks (blank lines → `\n\n`).
        assert!(body.contains("\n\n"), "paragraphs preserved: {body:?}");
    }

    #[test]
    fn diag_body_missing_locale_key_falls_back_to_english() {
        // zh omits nothing today, so force the fallback path with a fabricated
        // code: it resolves to the bare key (loud signal), same in every locale.
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
        // A resolved en-region still negotiates to the `en` base tag.
        assert!(is_english(&resolve(Some("en-US"), None, None, None)));
    }
}
