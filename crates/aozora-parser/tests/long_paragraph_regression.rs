//! Reproduce the Tier A leak on an isolated long paragraph so we don't need
//! 2 MB of context to debug each iteration.

const FIXTURE: &str = include_str!("../../../spec/aozora/fixtures/56656/input.utf8.txt");

#[test]
fn long_paragraph_consumes_all_bracket_annotations() {
    // Line 3713 of 罪と罰 is a single ~30KB paragraph containing 11 of the
    // Tier A leak sites. Isolating it keeps diagnostic output manageable.
    let line = FIXTURE
        .lines()
        .find(|l| l.contains("可哀想［＃「可哀想」に傍点］"))
        .expect("target paragraph present in fixture");
    assert!(
        line.len() > 10_000,
        "expected long paragraph, got {}",
        line.len()
    );

    let html = afm_parser::html::render_to_string(line);
    let stripped = strip_afm_annotations(&html);
    let leaks = stripped.matches("［＃").count();

    // Dump the first ~500 chars of stripped output on failure so the diagnostic
    // is immediately actionable without re-running under --nocapture.
    assert_eq!(
        leaks,
        0,
        "{} bare ［＃ markers leaked (expected 0).\n\
         First leak near: {}",
        leaks,
        first_occurrence(&stripped, "［＃", 80),
    );
}

fn strip_afm_annotations(html: &str) -> String {
    let opener = r#"<span class="afm-annotation" hidden>"#;
    let closer = "</span>";
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(at) = rest.find(opener) {
        out.push_str(&rest[..at]);
        let after_open = &rest[at + opener.len()..];
        if let Some(close_at) = after_open.find(closer) {
            rest = &after_open[close_at + closer.len()..];
        } else {
            out.push_str(rest);
            return out;
        }
    }
    out.push_str(rest);
    out
}

fn first_occurrence(haystack: &str, needle: &str, window: usize) -> String {
    let Some(at) = haystack.find(needle) else {
        return "<missing>".to_owned();
    };
    let lo = snap_left(haystack, at.saturating_sub(window));
    let hi = snap_right(haystack, (at + needle.len() + window).min(haystack.len()));
    format!("...{}...", &haystack[lo..hi])
}

const fn snap_left(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

const fn snap_right(s: &str, mut i: usize) -> usize {
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}
