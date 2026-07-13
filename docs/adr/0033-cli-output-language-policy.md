# 0033. CLI output language policy

- Status: accepted
- Date: 2026-07-14
- Deciders: @P4suta
- Tags: cli, i18n, contracts, output

## Context

The CLI's human-facing text had accreted a Japanese/English mix within single
surfaces — the stdin guard printed Japanese (`標準入力が空です …`), the `explain`
footer printed English (`help: run \`aozora explain <code>\` …`), and the
`explain` section labels printed Japanese (`再現例:` / `修正後:`) next to an
English `see:`. Worse, some prose was hard-coded in a *machine-contract* crate
(`aozora-spec`), coupling diagnostic wording to the stable code/severity/source
surface that every binding and the drift gate depend on.

The DX overhaul (`.claude/plans/ui-ux-serialized-wave.md`) makes deep i18n a
pillar: externalize prose to Fluent `.ftl` catalogs, seed `en`/`ja`/`zh`, and
share one catalog between the CLI and the LSP. But localizing *everything* would
be a correctness hazard — agents, CI, and language bindings parse the CLI's JSON
/ `short` / exit codes, and those must stay byte-stable. So the boundary between
"human chrome" and "machine contract" has to be drawn explicitly and defended.

This ADR fixes the **language policy**: which language human output speaks, how
that language is chosen, and — the load-bearing half — what is never localized.
(W5 lands the infrastructure and the CLI shell strings; the diagnostic prose
migration out of `aozora-spec` is W6.)

## Decision

**English is the canonical, default, and fallback language of all human
output.** Verbs and concepts are terser and more discoverable in English, and
`en` is the last resort of the resolution chain. `ja` and `zh` are available.

**Localization lives in `aozora-i18n`**, a thin function-based wrapper over the
Project Fluent stack (`fluent-bundle` + `unic-langid` + `fluent-langneg`).
Message text is in external `locales/<locale>.ftl` files embedded at compile
time; a new language is one `.ftl` plus one line — no Rust enum to extend. The
CLI and the LSP share the one catalog.

**Language resolution** is a single, pure, unit-tested precedence chain,
resolved once up front and threaded to every surface:

```
--lang  >  AOZORA_LANG  >  .aozora.toml `lang`  >  LANG  >  en
```

- The first source that is present and non-blank decides the language: its
  value is parsed (POSIX `ja_JP.UTF-8` shapes accepted), negotiated against the
  available catalogs, and — if unknown or unparsable — resolved to `en`. It
  does **not** fall through to a lower source once a source is present.
- Negotiation is real Fluent language negotiation (`negotiate_languages`), so
  `zh-Hans-CN` → `zh`, `en-GB` → `en`, and `C` / `POSIX` → `en`.

**`LANG` is consulted for *message* language only — never for byte encoding.**
Source-byte interpretation stays governed exclusively by `--encoding` /
`AOZORA_ENCODING` / `.aozora.toml encoding` / auto-detection, exactly as before
(ADR-0013). A locale-driven decoder would make the parser non-reproducible
across machines; that prohibition is unchanged. `LANG` graduating to a
lowest-priority *message* source does not touch it.

**The machine axis is never localized.** These stay byte-identical English
contracts under any `--lang`:

- diagnostic codes (`aozora::lex::*`), the JSON envelopes (`aozora::json`),
  `--format short` lines, exit codes, the `schema` output, and the timing JSON;
- the diagnostic `#[error]` Display (the source of `short` / `json` / log text).

Only human chrome routes through the catalog: the stdin guard, the `--watch`
banner, and the `explain` footer / section labels — with the diagnostic prose
following in W6. A `--lang ja/zh` run swaps that chrome and nothing else; a
regression test asserts `check --format json|short` is byte-identical across
`--lang en/ja/zh`.

**clap's structural help / usage stays English.** clap derive has no runtime
locale for help text, and the mainstream precedent (git, cargo) keeps
`--help` / man pages English. This is an explicit carve-out, not an oversight.

**Test hermeticity.** Because `LANG` now influences message language, the CLI
test harness pins `AOZORA_LANG=en` and strips `LANG` / `LC_ALL`, so message
output — and the help / version / message snapshots — is deterministic
regardless of the developer's locale. CI is likewise pinned.

## Consequences

- Human output is coherent (one language per surface) and translator-friendly:
  prose is data in `.ftl`, not string literals in Rust.
- The stable surface every downstream tool parses is provably unaffected by the
  language axis — enforced, not just intended.
- Adding a locale is a `.ftl` file; the enum-free design keeps that a
  data change.
- `LANG` gains a narrow, well-scoped role (message language, lowest priority)
  while its encoding prohibition is untouched — the two axes never cross.

## Alternatives considered

- **Japanese as the default.** Rejected: the notation is Japanese but the tool
  is a developer CLI; English verbs are more discoverable and `en` is the only
  sensible universal fallback.
- **`i18n-embed` + `i18n-embed-fl` (the ergonomic loader).** Rejected for W5:
  it brings a `LanguageLoader` / macro layer heavier than a catalog this small
  needs, and a hand-rolled `resolve` + `t`/`tf` keeps the public API tiny, pure,
  and byte-exact-testable. The raw Fluent stack is still the industry standard.
- **`fluent-langneg` 0.14.** Rejected: 0.14 rebased its identifier type on
  ICU4X's `icu_locid`, which does not unify with the `unic_langid` type
  `fluent-bundle` 0.16 speaks. 0.13 keeps the whole stack on one identifier.
- **Reading `LANG` for encoding too.** Rejected: it would reintroduce exactly
  the cross-machine non-reproducibility ADR-0013 forbids. Message language and
  byte encoding are kept as strictly separate axes.
- **Localizing the machine axis (or number-formatting the args).** Rejected:
  CI / agents / bindings depend on byte stability; Fluent's bidi isolates are
  disabled (`set_use_isolating(false)`) and count args are passed as strings so
  even interpolated chrome is deterministic.

## References

- Plan: `.claude/plans/ui-ux-serialized-wave.md` (§6 i18n; the resolution-order
  row of the 確定した方針 table).
- Project Fluent: <https://projectfluent.org/>. `fluent-bundle`,
  `unic-langid`, `fluent-langneg` (the fluent-rs project).
- ADR-0013 (CLI configuration file) — the encoding-resolution precedence `LANG`
  stays out of.
- Evidence: `crates/aozora-i18n/`, `crates/aozora-cli/src/{main,input,watch,
  diagnostics_render,introspect,config}.rs`, `crates/aozora-cli/tests/common/mod.rs`.
