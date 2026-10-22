//! CLI output parity with the shared conformance fixtures.

use aozora_conformance::{RenderFixture, fixtures_root};

mod common;

#[derive(Clone, Copy)]
enum Framing {
    Verbatim,
    TrailingNewline,
}

fn load() -> Vec<RenderFixture> {
    let fixtures = RenderFixture::load_group(&fixtures_root(), "render");
    assert!(!fixtures.is_empty(), "no render fixtures found");
    fixtures
}

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
