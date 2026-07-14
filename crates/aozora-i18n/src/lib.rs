//! `aozora-i18n` — the localization layer shared by the aozora CLI and LSP.
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

#![forbid(unsafe_code)]

use std::slice::from_ref;
use std::sync::LazyLock;

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentError, FluentResource};
use fluent_langneg::{NegotiationStrategy, negotiate_languages};

pub use fluent_bundle::FluentArgs;
pub use unic_langid::LanguageIdentifier;

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
pub fn resolve(
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
pub fn t(lang: &LanguageIdentifier, key: &str) -> String {
    lookup(lang, key, None)
}

/// Look up `key` in `lang`'s catalog with `args` bound to the message's
/// placeables, English-fallback as in [`t`].
#[must_use]
pub fn tf(lang: &LanguageIdentifier, key: &str, args: &FluentArgs<'_>) -> String {
    lookup(lang, key, Some(args))
}

/// True when `lang` is the canonical English locale.
///
/// The one language for which a host keeps the byte-stable `#[error]` Display
/// as the human diagnostic headline instead of substituting a localized title
/// — so the English human report never moves. Every resolved language from
/// [`resolve`] is one of the available base tags, so this is an exact match.
#[must_use]
pub fn is_english(lang: &LanguageIdentifier) -> bool {
    *lang == english()
}

/// The Fluent message-key stem for a diagnostic `code`.
///
/// Its trailing `::` segment with `_` turned into `-`:
/// `aozora::lex::unclosed_bracket` → `unclosed-bracket`, so the catalog keys
/// are `diag-unclosed-bracket-title` and `diag-unclosed-bracket-body`. The
/// diagnostic code string thus doubles as the localization key, keeping
/// aozora-spec and the `.ftl` catalogs in lock-step with no separate mapping
/// table.
fn diag_slug(code: &str) -> String {
    code.rsplit_once("::")
        .map_or(code, |(_, tail)| tail)
        .replace('_', "-")
}

/// The localized one-line title for a diagnostic `code` in `lang`.
///
/// Looks up `diag-<slug>-title`, where `slug` is `code`'s trailing `::`
/// segment with `_` turned into `-`; missing keys surface as the key itself, a
/// loud, greppable signal of a catalog gap. This is the prose migrated out of
/// `aozora-spec`'s `DOCS` table — the machine `code` / severity / `#[error]`
/// Display in that crate are unchanged.
#[must_use]
pub fn diag_title(lang: &LanguageIdentifier, code: &str) -> String {
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
pub fn diag_body(lang: &LanguageIdentifier, code: &str, args: &FluentArgs<'_>) -> String {
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

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one exhaustive per-locale key enumeration; splitting the single catalog-parity contract across helpers would scatter it and read worse"
    )]
    fn shell_keys_present_in_every_locale() {
        // Each shell key must resolve to something other than the bare key
        // (the missing-key signal) in every locale — no accidental gaps.
        let keys = [
            "watch-banner",
            "explain-hint-header",
            "explain-hint-more",
            "explain-repro-label",
            "explain-fixed-label",
            "explain-see-label",
            "explain-unknown-hint",
            "explain-did-you-mean",
            // Notation-concept prose (`aozora explain <concept>`): one title +
            // body per family, present in every locale.
            "concept-ruby-title",
            "concept-ruby-body",
            "concept-gaiji-title",
            "concept-gaiji-body",
            "concept-kaeriten-title",
            "concept-kaeriten-body",
            "concept-bouten-title",
            "concept-bouten-body",
            "concept-warichu-title",
            "concept-warichu-body",
            "concept-tcy-title",
            "concept-tcy-body",
            // `aozora doctor` chrome: section headings, status words, hints,
            // and the summary (the arg-bearing keys resolve through `tf` below).
            "doctor-title",
            "doctor-config-heading",
            "doctor-settings-heading",
            "doctor-tools-heading",
            "doctor-terminal-heading",
            "doctor-global-none",
            "doctor-parse-ok",
            "doctor-tool-missing",
            "doctor-hint-pandoc",
            "doctor-hint-lsp",
            "doctor-terminal-yes",
            "doctor-terminal-no",
            "doctor-env-unset",
            "doctor-colour-label",
            "doctor-colour-on",
            "doctor-colour-off",
            "doctor-all-passed",
            // `aozora init` chrome: heading, per-file outcome words, the skip
            // hint, and the next-steps footer.
            "init-heading",
            "init-created",
            "init-overwritten",
            "init-skipped",
            "init-skipped-hint",
            "init-next-steps",
            "init-step-render",
            "init-step-check",
            "init-step-doctor",
            // `aozora repl` chrome: banner, help, section labels, and the
            // clean-parse placeholder (the arg-bearing keys resolve via `tf`).
            "repl-banner",
            "repl-help",
            "repl-label-nodes",
            "repl-label-html",
            "repl-label-pandoc",
            "repl-label-diag",
            "repl-diag-none",
            // `aozora tui` chrome: pane titles, the modified marker, the
            // clean-parse placeholder, the footer keybind words, the no-file
            // hint, and the non-tty refusal (the arg-bearing save keys resolve
            // via `tf` below).
            "tui-title-source",
            "tui-title-preview",
            "tui-title-diagnostics",
            "tui-modified",
            "tui-diag-none",
            "tui-key-save",
            "tui-key-lang",
            "tui-key-preview",
            "tui-key-quit",
            "tui-no-file",
            "tui-no-tty",
        ];
        for tag in ["en", "ja", "zh"] {
            let l = lang(tag);
            for key in keys {
                assert_ne!(t(&l, key), key, "{tag}.ftl is missing `{key}`");
            }
            // The arg-bearing doctor keys resolve through `tf` too.
            let mut dir = FluentArgs::new();
            dir.set("dir", "/x");
            assert_ne!(tf(&l, "doctor-project-none", &dir), "doctor-project-none");
            let mut err = FluentArgs::new();
            err.set("error", "boom");
            assert_ne!(tf(&l, "doctor-parse-error", &err), "doctor-parse-error");
            let mut value = FluentArgs::new();
            value.set("value", "1");
            assert_ne!(tf(&l, "doctor-env-set", &value), "doctor-env-set");
            let mut count = FluentArgs::new();
            count.set("count", "1");
            assert_ne!(tf(&l, "doctor-problems", &count), "doctor-problems");
            let mut rejected = FluentArgs::new();
            rejected.set("var", "AOZORA_ENCODING");
            rejected.set("value", "SJIS");
            assert_ne!(
                tf(&l, "doctor-setting-rejected", &rejected),
                "doctor-setting-rejected"
            );
            // The arg-bearing keys resolve through `tf` too.
            let mut args = FluentArgs::new();
            args.set("cmd", "check");
            assert_ne!(tf(&l, "stdin-empty", &args), "stdin-empty");
            let mut target = FluentArgs::new();
            target.set("target", "bogus");
            assert_ne!(tf(&l, "explain-unknown", &target), "explain-unknown");
            // The arg-bearing `aozora repl` keys resolve through `tf` too.
            let mut mode = FluentArgs::new();
            mode.set("mode", "all");
            assert_ne!(tf(&l, "repl-mode-set", &mode), "repl-mode-set");
            let mut msg_lang = FluentArgs::new();
            msg_lang.set("lang", "ja");
            assert_ne!(tf(&l, "repl-lang-set", &msg_lang), "repl-lang-set");
            let mut enc = FluentArgs::new();
            enc.set("encoding", "utf8");
            assert_ne!(tf(&l, "repl-encoding-set", &enc), "repl-encoding-set");
            let mut path = FluentArgs::new();
            path.set("path", "book.txt");
            assert_ne!(tf(&l, "repl-loaded", &path), "repl-loaded");
            let mut load_err = FluentArgs::new();
            load_err.set("path", "book.txt");
            load_err.set("error", "boom");
            assert_ne!(tf(&l, "repl-load-error", &load_err), "repl-load-error");
            let mut unknown = FluentArgs::new();
            unknown.set("cmd", "frob");
            assert_ne!(tf(&l, "repl-unknown-meta", &unknown), "repl-unknown-meta");
            let mut usage = FluentArgs::new();
            usage.set("cmd", "mode");
            usage.set("expected", "nodes, html");
            assert_ne!(tf(&l, "repl-usage", &usage), "repl-usage");
            // The arg-bearing `aozora tui` save keys resolve through `tf` too.
            let mut saved = FluentArgs::new();
            saved.set("path", "book.txt");
            assert_ne!(tf(&l, "tui-saved", &saved), "tui-saved");
            let mut save_err = FluentArgs::new();
            save_err.set("path", "book.txt");
            save_err.set("error", "boom");
            assert_ne!(tf(&l, "tui-save-error", &save_err), "tui-save-error");
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
    fn stdin_empty_japanese_matches_the_migrated_source() {
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

    // --- diagnostic prose migrated out of aozora-spec ---

    /// The 21 diagnostic code slugs, mirroring `aozora_spec::Diagnostic::
    /// ALL_CODES` (kept here as literals so this crate does not depend on the
    /// catalogue crate — the `.ftl` keys are the coupling point, verified by
    /// the CLI's per-code `explain` tests end-to-end).
    const DIAG_SLUGS: [&str; 21] = [
        "source-contains-pua",
        "unclosed-bracket",
        "unmatched-close",
        "accent-decomposition-applied",
        "unresolved-gaiji",
        "mismatched-container-close",
        "empty-ruby-reading",
        "nested-ruby",
        "unrecognised-container-directive",
        "tcy-target-not-found",
        "bouten-target-ambiguous",
        "forward-referent-not-stylable",
        "break-in-single-line-container",
        "bracketed-kaeriten-no-pair",
        "kaeriten-outside-kanbun",
        "mismatched-bouten-container",
        "non-canonical-directive",
        "residual-annotation-marker",
        "unregistered-sentinel",
        "registry-out-of-order",
        "registry-position-mismatch",
    ];

    #[test]
    fn every_diagnostic_has_title_and_body_in_every_locale() {
        // No silent catalog gap: each code's title/body resolves (i.e. does
        // not fall through to the bare key) in en / ja / zh.
        for tag in ["en", "ja", "zh"] {
            let l = lang(tag);
            for slug in DIAG_SLUGS {
                let code = format!("aozora::lex::{}", slug.replace('-', "_"));
                let title = diag_title(&l, &code);
                assert_ne!(title, format!("diag-{slug}-title"), "{tag}: {slug} title");
                assert!(!title.trim().is_empty(), "{tag}: {slug} empty title");
                let body = diag_body(&l, &code, &FluentArgs::new());
                assert_ne!(body, format!("diag-{slug}-body"), "{tag}: {slug} body");
                assert!(!body.trim().is_empty(), "{tag}: {slug} empty body");
            }
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
