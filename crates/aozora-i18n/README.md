# aozora-i18n

Localization layer shared by the [aozora](https://github.com/P4suta/aozora)
CLI and LSP. A thin, function-based wrapper over the
[Project Fluent](https://projectfluent.org/) runtime: the human-facing
shell strings live in external `locales/<locale>.ftl` catalogs
(English canonical, Japanese, Chinese) embedded at compile time.

English is the default; `--lang` / `AOZORA_LANG` (and, last, `LANG`)
opt in to `ja` / `zh`. Adding a language is one `.ftl` file plus one
entry in the source locale table — there is no enum to extend.

> **Shell strings only.** The machine output axis — diagnostic codes,
> JSON envelopes, `short` lines, exit codes, schema / timing JSON — is a
> byte-stable English contract and never passes through this layer. See
> [ADR-0033](https://github.com/P4suta/aozora/blob/main/docs/adr/0033-cli-output-language-policy.md).

## Library

The public surface is deliberately tiny:

- `resolve` turns the CLI's four language sources (`--lang`,
  `AOZORA_LANG`, `config.lang`, `LANG`) into a concrete
  `LanguageIdentifier`, negotiating the request against the available
  catalogs and falling back to English for anything unknown or
  unparsable. POSIX shapes (`ja_JP.UTF-8`) are accepted alongside
  BCP-47 (`zh-Hans`).
- `t` looks a key up in a resolved language's bundle; `tf` binds
  `{$arg}` placeables. Both fall back to English for any key a
  translation happens to be missing, and return the key itself as a
  loud, greppable signal when no catalog defines it.

```rust
use aozora_i18n::{resolve, t};

// --lang beats AOZORA_LANG beats config.lang beats LANG; blanks fall through.
let lang = resolve(Some("ja"), None, None, Some("en_US.UTF-8"));
assert_eq!(lang.to_string(), "ja");

// Look up a shell string in the resolved locale.
let _banner = t(&lang, "watch-banner");

// Anything unknown negotiates down to English.
assert_eq!(resolve(Some("tlh"), None, None, None).to_string(), "en");
```

## Repository

Part of the [aozora](https://github.com/P4suta/aozora) workspace.
Dual-licensed Apache-2.0 OR MIT. See the
[workspace README](https://github.com/P4suta/aozora#readme) for the
full picture.
