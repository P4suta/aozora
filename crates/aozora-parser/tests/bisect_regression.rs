//! Binary-search which slice of the fixture triggers the Tier A leak.

use afm_parser::html::render_to_string;
use afm_parser::test_support::strip_annotation_wrappers;

const FIXTURE: &str = include_str!("../../../spec/aozora/fixtures/56656/input.utf8.txt");

fn count_leaks(input: &str) -> usize {
    let html = render_to_string(input);
    strip_annotation_wrappers(&html).matches("［＃").count()
}

#[test]
#[ignore = "diagnostic — prints leak counts at multiple slice prefixes"]
fn bisect_leak_trigger_by_line() {
    let total_lines = FIXTURE.lines().count();
    let checkpoints = [3720, 3750, 3800, 3850, 3900, 3950];
    for cp in checkpoints {
        let prefix = prefix_lines(FIXTURE, cp.min(total_lines));
        let leaks = count_leaks(&prefix);
        println!(
            "prefix {cp}/{total_lines} lines ({} bytes) → {leaks} leaks",
            prefix.len()
        );
    }
    panic!("diagnostic only");
}

fn prefix_lines(src: &str, n: usize) -> String {
    let mut out = String::new();
    for (i, line) in src.lines().enumerate() {
        if i >= n {
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}
