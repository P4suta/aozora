//! End-to-end smoke tests for `aozora spec kinds` / `aozora spec schema` /
//! `aozora explain` (Phase L3).
//!
//! Mirrors `smoke.rs` in spawning the *built* binary via
//! `CARGO_BIN_EXE_aozora` so argv → clap → introspect dispatch is
//! exercised end-to-end. The library tests in `aozora-spec` /
//! `aozora-syntax` already pin every enum variant; these tests pin
//! the *CLI shape* — that the subcommand exists, the columns appear,
//! the schema parses, and unknown explain tags surface a hint.

use std::process::{Command, ExitStatus, Stdio};

use aozora::json::SCHEMA_VERSION;

const BIN: &str = env!("CARGO_BIN_EXE_aozora");

fn run(args: &[&str]) -> (ExitStatus, String, String) {
    // Pin the message language so `explain`'s section labels are English and
    // deterministic regardless of the host locale: `AOZORA_LANG=en` beats the
    // `LANG` fallback and stripping `LANG` / `LC_ALL` removes it from the
    // chain. A test that wants another language passes `--lang`, which outranks
    // `AOZORA_LANG`.
    let output = Command::new(BIN)
        .args(args)
        .env("AOZORA_LANG", "en")
        .env_remove("LANG")
        .env_remove("LC_ALL")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn aozora");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    (output.status, stdout, stderr)
}

// ---------------------------------------------------------------------
// `aozora spec kinds`
// ---------------------------------------------------------------------

#[test]
fn kinds_lists_every_enum_section() {
    // `--format human` forces the tables: the captured stdout is piped, where
    // the `auto` default now resolves to json (see `kinds_default_auto_...`).
    let (status, stdout, stderr) = run(&["spec", "kinds", "--format", "human"]);
    assert!(status.success(), "kinds failed: {stderr:?}");
    for section in [
        "NodeKind",
        "PairKind",
        "Severity",
        "DiagnosticSource",
        "InternalCheckCode",
    ] {
        assert!(
            stdout.contains(section),
            "kinds output missing {section} section: {stdout:?}",
        );
    }
}

#[test]
fn kinds_lists_concrete_node_tags() {
    let (status, stdout, _) = run(&["spec", "kinds", "--format", "human"]);
    assert!(status.success());
    // Spot-check tags that span the camelCase / non-ascii lookup paths.
    for tag in ["ruby", "angleQuote", "containerOpen", "containerClose"] {
        assert!(stdout.contains(tag), "kinds missing tag {tag}: {stdout:?}");
    }
}

#[test]
fn kinds_default_auto_is_json_when_stdout_piped() {
    // No `--format`: `auto` resolves to json because the captured stdout is not
    // a terminal — the unification with `check`'s diagnostics auto rule.
    let (status, stdout, stderr) = run(&["spec", "kinds"]);
    assert!(status.success(), "kinds failed: {stderr:?}");
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("piped kinds must default to json");
    assert_eq!(
        parsed["schemaVersion"], SCHEMA_VERSION,
        "wire envelope: {parsed}"
    );
    assert!(parsed["data"]["nodeKinds"].is_array(), "{parsed}");
    // The human table section header must NOT appear — proof it is not tables.
    assert!(
        !stdout.contains("NodeKind — "),
        "piped default must be json, not tables: {stdout:?}"
    );
}

#[test]
fn kinds_format_json_emits_valid_envelope() {
    let (status, stdout, stderr) = run(&["spec", "kinds", "--format", "json"]);
    assert!(status.success(), "kinds --format json failed: {stderr:?}");
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("kinds --format json must be valid JSON");
    assert_eq!(
        parsed["schemaVersion"], SCHEMA_VERSION,
        "envelope schemaVersion: {parsed}"
    );
    // Every section appears as a camelCase array under `data`.
    for key in [
        "nodeKinds",
        "pairKinds",
        "severities",
        "diagnosticSources",
        "internalCheckCodes",
    ] {
        assert!(
            parsed["data"][key].is_array(),
            "data.{key} must be an array: {parsed}",
        );
    }
    // Rows are `{tag, summary}` objects; spot-check a known node tag.
    let node_kinds = parsed["data"]["nodeKinds"]
        .as_array()
        .expect("nodeKinds array");
    assert!(
        node_kinds
            .iter()
            .any(|r| r["tag"] == "ruby" && r["summary"].is_string()),
        "nodeKinds must carry the ruby tag with a summary: {parsed}",
    );
    // Compact single line (matches the `inspect` envelopes, not pretty `schema`).
    assert_eq!(stdout.lines().count(), 1, "envelope must be one line");
}

// ---------------------------------------------------------------------
// `aozora spec schema`
// ---------------------------------------------------------------------

#[test]
fn schema_diagnostics_emits_valid_json() {
    let (status, stdout, stderr) = run(&["spec", "schema", "diagnostics"]);
    assert!(status.success(), "schema diagnostics failed: {stderr:?}");
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("schema output must be valid JSON");
    assert_eq!(
        parsed["title"].as_str(),
        Some("AozoraDiagnosticsEnvelope"),
        "schema title mismatch: {parsed:?}",
    );
}

#[test]
fn schema_each_envelope_succeeds() {
    for which in ["config", "diagnostics", "nodes", "pairs", "container-pairs"] {
        let (status, stdout, stderr) = run(&["spec", "schema", which]);
        assert!(status.success(), "schema {which} failed: {stderr:?}");
        assert!(
            serde_json::from_str::<serde_json::Value>(&stdout).is_ok(),
            "schema {which} output is not valid JSON",
        );
    }
}

#[test]
fn config_schema_is_closed_and_uses_the_runtime_value_types() {
    let (status, stdout, stderr) = run(&["spec", "schema", "config"]);
    assert!(status.success(), "schema config failed: {stderr:?}");
    let schema: serde_json::Value = serde_json::from_str(&stdout).expect("valid config schema");
    assert_eq!(schema["title"], "AozoraConfig");
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(
        schema["properties"]["encoding"]["anyOf"][0]["$ref"],
        "#/$defs/Encoding"
    );
    assert_eq!(
        schema["properties"]["format"]["anyOf"][0]["$ref"],
        "#/$defs/DiagFormat"
    );
    assert_eq!(
        schema["properties"]["color"]["anyOf"][0]["$ref"],
        "#/$defs/ColorChoice"
    );
    assert_eq!(
        schema["properties"]["strict"]["type"],
        serde_json::json!(["boolean", "null"])
    );
    assert_eq!(
        schema["properties"]["lang"]["type"],
        serde_json::json!(["string", "null"])
    );

    let constants = |definition: &str| {
        schema["$defs"][definition]["oneOf"]
            .as_array()
            .expect("enum definition")
            .iter()
            .map(|variant| variant["const"].as_str().expect("string enum constant"))
            .collect::<Vec<_>>()
    };
    assert_eq!(constants("Encoding"), ["auto", "utf8", "sjis"]);
    assert_eq!(constants("DiagFormat"), ["auto", "human", "json", "short"]);
    assert_eq!(constants("ColorChoice"), ["auto", "always", "never"]);
}

// ---------------------------------------------------------------------
// `aozora spec slugs`
// ---------------------------------------------------------------------

#[test]
fn spec_slugs_needs_no_input_and_emits_the_wire_envelope() {
    // The slug catalogue is static: it must succeed with neither stdin nor a
    // file argument, emitting the shared `aozora::json` envelope on stdout —
    // byte-identical to every binding's `slugs_json()` output.
    let (status, stdout, stderr) = run(&["spec", "slugs"]);
    assert!(status.success(), "spec slugs failed: {stderr:?}");
    assert!(
        stdout.starts_with(&format!(r#"{{"schemaVersion":{SCHEMA_VERSION},"#)),
        "wire envelope: {stdout:?}"
    );
    assert!(stdout.contains(r#""canonical":"#), "slugs: {stdout:?}");
}

// ---------------------------------------------------------------------
// `aozora explain`
// ---------------------------------------------------------------------

#[test]
fn explain_known_kind_succeeds() {
    let (status, stdout, _) = run(&["explain", "ruby"]);
    assert!(status.success(), "explain ruby must succeed");
    assert!(stdout.contains("NodeKind::Ruby"), "missing tag: {stdout:?}");
    // The embedded handbook page (Phase O1) carries the Source
    // examples / Rendered HTML / AST shape sections.
    assert!(
        stdout.contains("## Source examples"),
        "missing handbook section: {stdout:?}"
    );
}

#[test]
fn explain_camelcase_tag_succeeds() {
    let (status, stdout, _) = run(&["explain", "angleQuote"]);
    assert!(status.success(), "explain angleQuote must succeed");
    assert!(stdout.contains("AngleQuote"), "missing tag: {stdout:?}");
}

#[test]
fn explain_unknown_kind_fails_with_hint() {
    let (status, _, stderr) = run(&["explain", "bogus"]);
    assert!(!status.success(), "unknown kind must exit non-zero");
    assert!(
        stderr.contains("aozora spec kinds"),
        "expected hint pointing at `aozora spec kinds`: {stderr:?}",
    );
}

#[test]
fn explain_diagnostic_code_prints_severity_and_url() {
    let (status, stdout, stderr) = run(&["explain", "aozora::lex::unclosed_bracket"]);
    assert!(status.success(), "explain by code must succeed: {stderr:?}");
    assert!(
        stdout.contains("aozora::lex::unclosed_bracket"),
        "code echoed back: {stdout:?}"
    );
    assert!(stdout.contains("error"), "severity axis: {stdout:?}");
    assert!(
        stdout.contains(
            "https://p4suta.github.io/aozora-notation-spec/diagnostics.html#unclosed-bracket"
        ),
        "docs url points at the specification, which defines this diagnostic: {stdout:?}"
    );
}

#[test]
fn explain_short_diagnostic_code_form_succeeds() {
    // The bare trailing token expands to the canonical aozora::lex::… code.
    let (status, stdout, _) = run(&["explain", "unresolved_gaiji"]);
    assert!(status.success(), "short-form code must succeed");
    assert!(
        stdout.contains("aozora::lex::unresolved_gaiji"),
        "code: {stdout:?}"
    );
    assert!(stdout.contains("warning"), "severity: {stdout:?}");
}

#[test]
fn explain_internal_diagnostic_code_succeeds() {
    let (status, stdout, _) = run(&["explain", "unregistered_sentinel"]);
    assert!(status.success(), "internal code must explain");
    assert!(
        stdout.contains("aozora::lex::unregistered_sentinel"),
        "code: {stdout:?}"
    );
    assert!(stdout.contains("internal"), "source axis: {stdout:?}");
}

#[test]
fn explain_section_labels_default_to_english() {
    // The CLI-owned section labels are English by default; the spec-owned
    // diagnostic prose around them is untouched by the language axis.
    let (status, stdout, stderr) = run(&["explain", "aozora::lex::unclosed_bracket"]);
    assert!(status.success(), "explain must succeed: {stderr:?}");
    assert!(stdout.contains("Reproduction:"), "repro label: {stdout:?}");
    assert!(stdout.contains("After fix:"), "fixed label: {stdout:?}");
    assert!(stdout.contains("see: "), "see label: {stdout:?}");
}

#[test]
fn explain_section_labels_localize_with_lang() {
    // `--lang` outranks the pinned `AOZORA_LANG=en` and swaps only the
    // CLI-owned labels; the diagnostic code / URL contract is unchanged.
    let (status, ja, stderr) = run(&["explain", "--lang", "ja", "aozora::lex::unclosed_bracket"]);
    assert!(
        status.success(),
        "explain --lang ja must succeed: {stderr:?}"
    );
    assert!(ja.contains("再現例:"), "ja repro label: {ja:?}");
    assert!(ja.contains("修正後:"), "ja fixed label: {ja:?}");
    assert!(ja.contains("参照: "), "ja see label: {ja:?}");
    // The machine-stable code and URL survive localization.
    assert!(ja.contains("aozora::lex::unclosed_bracket"), "code: {ja:?}");

    let (status, zh, _) = run(&["explain", "--lang", "zh", "aozora::lex::unclosed_bracket"]);
    assert!(status.success(), "explain --lang zh must succeed");
    assert!(zh.contains("复现示例:"), "zh repro label: {zh:?}");
    assert!(zh.contains("修正后:"), "zh fixed label: {zh:?}");
    assert!(zh.contains("参见: "), "zh see label: {zh:?}");
}

#[test]
fn explain_title_and_body_prose_localize_with_lang() {
    // Title / body prose comes from aozora-i18n, keyed by code + lang.
    // The English default title/body:
    let (status, en, stderr) = run(&["explain", "aozora::lex::unclosed_bracket"]);
    assert!(status.success(), "explain en must succeed: {stderr:?}");
    assert!(en.contains("Unclosed opening bracket"), "en title: {en:?}");
    assert!(en.contains("There is an unclosed"), "en body: {en:?}");

    // `--lang ja` swaps the title and body to the Japanese prose.
    let (status, ja, _) = run(&["explain", "--lang", "ja", "aozora::lex::unclosed_bracket"]);
    assert!(status.success(), "explain --lang ja must succeed");
    assert!(ja.contains("閉じられていない開き括弧"), "ja title: {ja:?}");
    assert!(
        ja.contains("閉じられていない `［` があります"),
        "ja body: {ja:?}"
    );

    // `--lang zh` swaps to the Chinese prose.
    let (status, zh, _) = run(&["explain", "--lang", "zh", "aozora::lex::unclosed_bracket"]);
    assert!(status.success(), "explain --lang zh must succeed");
    assert!(zh.contains("未闭合的开括号"), "zh title: {zh:?}");
    assert!(zh.contains("存在未闭合的 `［`"), "zh body: {zh:?}");

    // The machine axis (code / severity / URL) is language-invariant.
    for out in [&en, &ja, &zh] {
        assert!(
            out.contains("aozora::lex::unclosed_bracket"),
            "code: {out:?}"
        );
        assert!(out.contains("error · source"), "axes: {out:?}");
        assert!(
            out.contains(
                "https://p4suta.github.io/aozora-notation-spec/diagnostics.html#unclosed-bracket"
            ),
            "url: {out:?}"
        );
    }
}
