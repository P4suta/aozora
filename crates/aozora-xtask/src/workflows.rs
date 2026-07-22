//! `xtask lint workflows` — two release-time-only shell hazards, caught at PR time.
//!
//! CI and composite-action `run:` steps invoke `just <recipe>` and pipe
//! producers into early-exiting consumers. Some of those steps only ever run at
//! release time (a producer re-qualify, a tag-driven publish, a recovery
//! dispatch), so the hazard passes every ordinary gate and fails only when the
//! real release runs. This gate reads the committed `run:` blocks so both fail
//! at PR time instead.
//!
//! * **Dangling `just <recipe>`.** A refactor deletes or renames a recipe while
//!   a `run:` block still calls the old name. `readme-gate` (folded into
//!   `publish-check`) survived in `release-ready.yml`'s `quality` job and failed
//!   only in a producer re-qualify — DEV-98. Every `just <recipe>` invoked in a
//!   workflow/action `run:` block must resolve to a Justfile recipe.
//! * **SIGPIPE under `pipefail`.** `tar … | grep -Fxq` makes grep exit on the
//!   first match and close the pipe; the producer takes SIGPIPE and
//!   `set -o pipefail` promotes the 141 to a step failure — DEV-105. Flag a
//!   real producer piped into an early-exit consumer inside a pipefail block.
//!
//! Necessary, not sufficient: it reads `run:` block text, not the shell's
//! runtime. Comment lines are dropped (the false positives — `# … just this …`
//! — live there), and only `pipefail` blocks are checked for SIGPIPE (GitHub's
//! default `bash -e {0}` has no pipefail, so a producer's 141 does not
//! propagate there).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::{fs, io};

use regex::Regex;

use crate::scan::workspace_root;

/// Trees whose `run:` blocks invoke `just` and run shell pipelines.
const WORKFLOW_DIRS: &[&str] = &[".github/workflows", ".github/actions"];

/// A shell command line lifted out of a `run:` block, with its 1-based line
/// number and the index of the block it belongs to.
struct RunLine {
    line_no: usize,
    text: String,
    block: usize,
}

/// Justfile recipe names. A recipe definition is `name params?: deps?` at
/// column 0 (optionally `@`-quiet), never a `:=` assignment or a `set …` line.
fn recipe_names(justfile: &str) -> BTreeSet<String> {
    let re = Regex::new(r"^@?(?P<name>[a-z_][A-Za-z0-9_-]*)(?:\s[^:]*)?:(?:\s|$)")
        .expect("static recipe regex");
    justfile
        .lines()
        .filter(|line| !line.contains(":="))
        .filter_map(|line| re.captures(line).map(|c| c["name"].to_owned()))
        .collect()
}

/// Extract every `run:` block's command lines. A `run:` step is either
/// `run: <cmd>` (single line) or `run: |` / `run: >` followed by a body
/// indented past the **`run:` key's own column** (`- run:` shifts it by two, so
/// a sibling `env:` / `with:` under the same step is not mistaken for body).
/// Comment lines (first non-space char `#`) are dropped, and shell
/// line-continuations (a line ending in `|` or `\`) are joined into one
/// logical command so a multi-line pipe is seen whole.
fn run_lines(yaml: &str) -> Vec<RunLine> {
    let lines: Vec<&str> = yaml.lines().collect();
    let mut out = Vec::new();
    let mut block = 0usize;
    let mut i = 0usize;
    while i < lines.len() {
        let line = lines[i];
        let leading = line.len() - line.trim_start().len();
        let trimmed = line.trim_start();
        let (had_dash, after) = trimmed
            .strip_prefix("- ")
            .map_or((false, trimmed), |rest| (true, rest));
        let Some(rest) = after.strip_prefix("run:") else {
            i += 1;
            continue;
        };
        block += 1;
        let run_col = leading + if had_dash { 2 } else { 0 };
        let rest = rest.trim_start();
        let is_scalar = rest.starts_with('|') || rest.starts_with('>');
        if !is_scalar {
            // `run: <cmd>` on one line.
            push_cmd(&mut out, i + 1, rest, block);
            i += 1;
            continue;
        }
        // Block scalar: body is the deeper-indented lines that follow. Join
        // shell line-continuations — a line ending in `|` or `\` — into one
        // logical command, so a producer piped to a next-line consumer is seen.
        i += 1;
        let mut pending: Option<(usize, String)> = None;
        while i < lines.len() {
            let body = lines[i];
            if body.trim().is_empty() {
                i += 1;
                continue;
            }
            let body_indent = body.len() - body.trim_start().len();
            if body_indent <= run_col {
                break;
            }
            let t = body.trim();
            if !t.starts_with('#') {
                let piece = t.trim_end_matches('\\').trim_end();
                match &mut pending {
                    Some((_, acc)) => {
                        acc.push(' ');
                        acc.push_str(piece);
                    }
                    None => pending = Some((i + 1, piece.to_owned())),
                }
                if !(t.ends_with('|') || t.ends_with('\\')) {
                    let (no, acc) = pending.take().expect("pending set just above");
                    push_cmd(&mut out, no, &acc, block);
                }
            }
            i += 1;
        }
        if let Some((no, acc)) = pending.take() {
            push_cmd(&mut out, no, &acc, block);
        }
    }
    out
}

fn push_cmd(out: &mut Vec<RunLine>, line_no: usize, text: &str, block: usize) {
    let text = text.trim();
    if text.is_empty() || text.starts_with('#') {
        return;
    }
    out.push(RunLine {
        line_no,
        text: text.to_owned(),
        block,
    });
}

/// The `just <recipe>` calls on a command line: `just` at a command boundary
/// (line start, or after `;` / `&` / `|` / `(`), followed by a recipe token.
/// A flag (`just --summary`) or a bare `just` matches nothing.
fn just_calls(text: &str, re: &Regex) -> Vec<String> {
    re.captures_iter(text)
        .map(|c| c["recipe"].to_owned())
        .collect()
}

/// True when `line` pipes a real producer into an early-exit consumer
/// (`grep -q` / `grep -m` / `head`, or an `awk … exit`). Instant builtins
/// (`echo` / `printf`) fill the pipe before the consumer can close it, and a
/// self-terminating `find … -quit` stops on its own — both benign. An `awk`
/// with no `exit` reads to EOF and is likewise safe.
fn sigpipe_hazard(line: &str, re: &Regex) -> bool {
    let Some(m) = re.find(line) else {
        return false;
    };
    if line.contains("-quit") {
        return false;
    }
    // The command feeding the flagged pipe: the last segment before `m`.
    let producer = line[..m.start()]
        .rsplit(['|', ';', '&'])
        .next()
        .unwrap_or("")
        .trim();
    let head = producer.split_whitespace().next().unwrap_or("");
    !matches!(head, "echo" | "printf" | ":")
}

fn read_workflows(root: &Path) -> Result<Vec<(PathBuf, String)>, String> {
    let mut out = Vec::new();
    for dir in WORKFLOW_DIRS {
        let base = root.join(dir);
        if !base.exists() {
            return Err(format!(
                "{dir}: not found — WORKFLOW_DIRS is stale, so this gate checks nothing"
            ));
        }
        collect_yaml(root, &base, &mut out)?;
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    if out.is_empty() {
        return Err("no workflow/action YAML found — this gate would pass vacuously".to_owned());
    }
    Ok(out)
}

fn collect_yaml(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, String)>) -> Result<(), String> {
    let entries = fs::read_dir(dir)
        .map_err(|e| format!("read_dir {}: {e}", dir.display()))?
        .collect::<io::Result<Vec<_>>>()
        .map_err(|e| format!("walk {}: {e}", dir.display()))?;
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_yaml(root, &path, out)?;
        } else if path.extension().is_some_and(|x| x == "yml" || x == "yaml") {
            let text =
                fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
            let rel = path
                .strip_prefix(root)
                .map_err(|e| format!("strip_prefix {}: {e}", path.display()))?;
            out.push((rel.to_path_buf(), text));
        }
    }
    Ok(())
}

/// `xtask lint workflows` — every `just <recipe>` in a `run:` block resolves,
/// and no `pipefail` block pipes a producer into an early-exit consumer.
pub(crate) fn check() -> Result<(), String> {
    let root = workspace_root()?;
    let justfile =
        fs::read_to_string(root.join("Justfile")).map_err(|e| format!("read Justfile: {e}"))?;
    let recipes = recipe_names(&justfile);
    if recipes.is_empty() {
        return Err(
            "Justfile: parsed no recipe names — the recipe scanner drifted, \
                    so this gate would resolve nothing"
                .to_owned(),
        );
    }

    let just_re = Regex::new(r"(?:^|[;&|(])\s*just\s+(?P<recipe>[a-z_][A-Za-z0-9_-]*)")
        .map_err(|e| format!("compile just pattern: {e}"))?;
    let pipe_re = Regex::new(r"\|\s*(?:grep\s+-[A-Za-z]*[qm]|head)\b|\|\s*awk\b[^|]*\bexit\b")
        .map_err(|e| format!("compile pipe pattern: {e}"))?;

    let mut problems = Vec::new();
    let mut just_seen = 0usize;
    for (rel, text) in read_workflows(&root)? {
        let run = run_lines(&text);
        let pipefail: BTreeSet<usize> = run
            .iter()
            .filter(|l| l.text.contains("pipefail"))
            .map(|l| l.block)
            .collect();
        for rl in &run {
            for recipe in just_calls(&rl.text, &just_re) {
                just_seen += 1;
                if !recipes.contains(&recipe) {
                    problems.push(format!(
                        "{}:{}: `just {recipe}` — no such Justfile recipe",
                        rel.display(),
                        rl.line_no
                    ));
                }
            }
            if pipefail.contains(&rl.block) && sigpipe_hazard(&rl.text, &pipe_re) {
                problems.push(format!(
                    "{}:{}: `{}` — producer takes SIGPIPE when the consumer exits early; \
                     `set -o pipefail` fails the step (list into a variable / here-string first)",
                    rel.display(),
                    rl.line_no,
                    rl.text
                ));
            }
        }
    }

    // Fail-on-zero: the extractor must find `just` calls, or it has drifted and
    // this gate resolves nothing.
    if just_seen == 0 {
        return Err(
            "scanned run: blocks but found no `just <recipe>` call — the run-block \
                    extractor drifted, so this gate is inert"
                .to_owned(),
        );
    }
    if !problems.is_empty() {
        for p in &problems {
            eprintln!("    {p}");
        }
        return Err(format!(
            "{} release-time shell hazard(s) in workflow run: blocks",
            problems.len()
        ));
    }
    eprintln!(
        "xtask lint workflows: {just_seen} `just` call(s) resolve; no SIGPIPE-under-pipefail hazards"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn just_re() -> Regex {
        Regex::new(r"(?:^|[;&|(])\s*just\s+(?P<recipe>[a-z_][A-Za-z0-9_-]*)").unwrap()
    }
    fn pipe_re() -> Regex {
        Regex::new(r"\|\s*(?:grep\s+-[A-Za-z]*[qm]|head)\b|\|\s*awk\b[^|]*\bexit\b").unwrap()
    }

    #[test]
    fn recipe_names_parses_defs_and_skips_assignments() {
        let just = "\
_dev := \"docker compose\"
set shell := [\"bash\"]
drift-gate:
    echo hi
example NAME *ARGS:
    echo {{NAME}}
@quiet:
    echo q
";
        let names = recipe_names(just);
        assert!(names.contains("drift-gate"));
        assert!(names.contains("example"));
        assert!(names.contains("quiet"));
        assert!(
            !names.contains("_dev"),
            "variable assignment is not a recipe"
        );
        assert!(!names.contains("set"));
    }

    #[test]
    fn run_lines_extracts_block_body_and_skips_comments_and_siblings() {
        let yaml = "\
jobs:
  x:
    steps:
      - name: Foo
        run: |
          set -euo pipefail
          # just this comment must be ignored
          just drift-gate
        env:
          BAR: baz
      - run: just publish-check
";
        let run = run_lines(yaml);
        let texts: Vec<&str> = run.iter().map(|r| r.text.as_str()).collect();
        assert!(texts.contains(&"just drift-gate"));
        assert!(texts.contains(&"just publish-check"));
        assert!(texts.contains(&"set -euo pipefail"));
        assert!(
            !texts.iter().any(|t| t.contains("BAR")),
            "sibling env: leaked into the run block: {texts:?}"
        );
        assert!(
            !texts.iter().any(|t| t.contains("must be ignored")),
            "a comment line leaked in: {texts:?}"
        );
    }

    #[test]
    fn run_lines_joins_a_multiline_pipe() {
        let yaml = "\
jobs:
  x:
    steps:
      - run: |
          v=\"$(unzip -p w META |
            awk '/V/ { print; exit }')\"
";
        let run = run_lines(yaml);
        let joined: Vec<&str> = run.iter().map(|r| r.text.as_str()).collect();
        assert!(
            joined.iter().any(|t| t.contains("unzip -p w META | awk")),
            "multi-line pipe was not joined: {joined:?}"
        );
    }

    #[test]
    fn just_calls_finds_commands_not_prose_or_flags() {
        let re = just_re();
        assert_eq!(just_calls("just drift-gate", &re), vec!["drift-gate"]);
        assert_eq!(
            just_calls("cmd && just publish-check", &re),
            vec!["publish-check"]
        );
        assert!(
            just_calls("just --summary", &re).is_empty(),
            "a flag is not a recipe"
        );
        assert!(
            just_calls("re-running just this job", &re).is_empty(),
            "prose must not match"
        );
    }

    #[test]
    fn sigpipe_flags_a_real_producer_but_not_builtins_or_find_quit() {
        let re = pipe_re();
        assert!(sigpipe_hazard(
            "tar -tzf x.tgz | grep -Fxq package/main",
            &re
        ));
        assert!(sigpipe_hazard("cat big | head -1", &re));
        assert!(sigpipe_hazard(
            "unzip -p w '*/METADATA' | awk '/V/ { print; exit }'",
            &re
        ));
        assert!(!sigpipe_hazard("echo \"$labels\" | grep -qx approved", &re));
        assert!(!sigpipe_hazard("find . -print -quit | grep -q .", &re));
        assert!(
            !sigpipe_hazard("sha256sum f | awk '{ print $1 }'", &re),
            "awk without exit reads all input"
        );
        assert!(
            !sigpipe_hazard("tar -tzf x.tgz | wc -l", &re),
            "non-early-exit consumer"
        );
    }

    /// The integration guard: the live tree's `run:` blocks must resolve every
    /// `just` call and carry no SIGPIPE-under-pipefail hazard.
    #[test]
    fn the_live_workflows_are_clean() {
        check().expect(
            "workflow run: blocks must have resolvable `just` calls and no SIGPIPE hazards",
        );
    }
}
