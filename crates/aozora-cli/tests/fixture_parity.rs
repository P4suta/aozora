//! Cross-surface parity gate — CLI channel.
//!
//! One golden authority (`crates/aozora-conformance/fixtures/render`),
//! N thin walkers. This walker drives the real `aozora` binary over
//! every render fixture and asserts each subcommand's stdout is
//! byte-identical to the committed golden the in-process `render_gate`
//! pins. A binding that reframes, re-orders, or drops a byte lights up
//! here without the golden itself having to be duplicated per channel.
//!
//! Surface → invocation:
//!
//! | surface           | `aozora …`                       | framing |
//! |-------------------|----------------------------------|---------|
//! | `html`            | `render FILE`                    | verbatim |
//! | `serialize`       | `fmt FILE`                       | verbatim |
//! | `diagnostics`     | `inspect diagnostics FILE`       | + `\n` |
//! | `nodes`           | `inspect nodes FILE`             | + `\n` |
//! | `pairs`           | `inspect pairs FILE`             | + `\n` |
//! | `container_pairs` | `inspect container-pairs FILE`   | + `\n` |
//!
//! Framing: `render` / `fmt` `write_all` their payload verbatim, so the
//! bytes equal the golden exactly. `inspect` is line-oriented — it frames
//! its JSON with `writeln!`, appending exactly one `\n` after the
//! conformance loader removes an optional storage line ending — so the four
//! `inspect` surfaces are compared against `golden + "\n"`. The trailing
//! newline is asserted, not trimmed: a
//! *second* stray newline (or a missing one) still fails the gate.

use aozora_conformance::{RenderFixture, fixtures_root};

mod common;

/// Whether a subcommand appends a trailing newline to its payload.
#[derive(Clone, Copy)]
enum Framing {
    /// `write_all(payload)` — bytes reach stdout unframed (`render` / `fmt`).
    Verbatim,
    /// `writeln!(payload)` — exactly one `\n` is appended (`inspect …`).
    TrailingNewline,
}

fn load() -> Vec<RenderFixture> {
    let fixtures = RenderFixture::load_group(&fixtures_root(), "render");
    assert!(!fixtures.is_empty(), "no render fixtures found");
    fixtures
}

/// Run `aozora <argv> <fixture-source>` and return its stdout as a
/// `String`, asserting a clean exit.
fn run(argv: &[&str], source_path: &str) -> String {
    let mut cmdline: Vec<&str> = argv.to_vec();
    cmdline.push(source_path);
    let out = common::hermetic_command()
        .args(&cmdline)
        .output()
        .expect("spawn aozora binary");
    assert!(
        out.status.success(),
        "`aozora {}` exited {:?}\nstderr:\n{}",
        cmdline.join(" "),
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
    String::from_utf8(out.stdout).expect("aozora stdout is valid UTF-8")
}

/// Walk every fixture, comparing `aozora <argv> source.txt` against the
/// golden selected by `golden`, under the given `framing`.
fn walk(
    surface: &str,
    argv: &[&str],
    golden: impl Fn(&RenderFixture) -> Option<String>,
    framing: Framing,
) {
    for fx in &load() {
        let want =
            golden(fx).unwrap_or_else(|| panic!("fixture {}: {surface} golden missing", fx.name));
        let source_path = fx.dir.join("source.txt");
        let actual = run(argv, &source_path.to_string_lossy());
        let expected = match framing {
            Framing::Verbatim => want,
            Framing::TrailingNewline => format!("{want}\n"),
        };
        assert_eq!(
            actual, expected,
            "CLI {surface} drift for fixture {}",
            fx.name
        );
    }
}

#[test]
fn fixture_parity_cli_render_matches_golden() {
    walk(
        "html",
        &["render"],
        |fx| fx.expected_html.clone(),
        Framing::Verbatim,
    );
}

#[test]
fn fixture_parity_cli_fmt_matches_golden() {
    walk(
        "serialize",
        &["fmt"],
        |fx| fx.expected_serialize.clone(),
        Framing::Verbatim,
    );
}

#[test]
fn fixture_parity_cli_inspect_diagnostics_matches_golden() {
    walk(
        "diagnostics",
        &["inspect", "diagnostics"],
        |fx| fx.expected_diagnostics.clone(),
        Framing::TrailingNewline,
    );
}

#[test]
fn fixture_parity_cli_inspect_nodes_matches_golden() {
    walk(
        "nodes",
        &["inspect", "nodes"],
        |fx| fx.expected_nodes.clone(),
        Framing::TrailingNewline,
    );
}

#[test]
fn fixture_parity_cli_inspect_pairs_matches_golden() {
    walk(
        "pairs",
        &["inspect", "pairs"],
        |fx| fx.expected_pairs.clone(),
        Framing::TrailingNewline,
    );
}

#[test]
fn fixture_parity_cli_inspect_container_pairs_matches_golden() {
    walk(
        "container_pairs",
        &["inspect", "container-pairs"],
        |fx| fx.expected_container_pairs.clone(),
        Framing::TrailingNewline,
    );
}
