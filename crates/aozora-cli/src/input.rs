use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::process::ExitCode;

use crate::i18n::{self as i18n, FluentArgs, LanguageIdentifier};

fn is_interactive_decision(path: &Path, is_terminal: bool) -> bool {
    path.as_os_str() == "-" && is_terminal
}

fn stdin_is_interactive(path: &Path) -> bool {
    is_interactive_decision(path, io::stdin().is_terminal())
}

fn write_stdin_hint(w: &mut impl Write, cmd: &str, lang: &LanguageIdentifier) -> io::Result<()> {
    let mut args = FluentArgs::new();
    args.set("cmd", cmd);
    writeln!(w, "{}", i18n::tf(lang, "stdin-empty", &args))
}

pub(crate) fn guard_stdin(path: &Path, cmd: &str, lang: &LanguageIdentifier) -> Option<ExitCode> {
    if !stdin_is_interactive(path) {
        return None;
    }
    let mut stderr = io::stderr().lock();
    let _drop = write_stdin_hint(&mut stderr, cmd, lang);
    Some(ExitCode::from(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lang(tag: &str) -> LanguageIdentifier {
        tag.parse().expect("test locale tag parses")
    }

    fn hint(cmd: &str, tag: &str) -> String {
        let mut buf = Vec::new();
        write_stdin_hint(&mut buf, cmd, &lang(tag)).expect("writing to a Vec cannot fail");
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn stdin_hint_bytes_are_exact_in_english_by_default() {
        assert_eq!(
            hint("check", "en"),
            concat!(
                "error: standard input is empty (reading from a terminal)\n",
                "  hint: read a file →  aozora check <FILE>\n",
                "        or a pipe   →  cat f.txt | aozora check\n",
                "  all commands:  aozora --help\n",
            ),
        );
    }

    #[test]
    fn stdin_hint_bytes_are_exact_in_japanese() {
        assert_eq!(
            hint("check", "ja"),
            concat!(
                "error: 標準入力が空です (端末から実行中)\n",
                "  ヒント: ファイルを →  aozora check <FILE>\n",
                "          パイプで   →  cat f.txt | aozora check\n",
                "  全機能:  aozora --help\n",
            ),
        );
    }

    #[test]
    fn stdin_hint_bytes_are_exact_in_chinese() {
        assert_eq!(
            hint("inspect nodes", "zh"),
            concat!(
                "error: 标准输入为空（正在从终端读取）\n",
                "  提示: 从文件读取 →  aozora inspect nodes <FILE>\n",
                "        用管道传入 →  cat f.txt | aozora inspect nodes\n",
                "  全部命令:  aozora --help\n",
            ),
        );
    }

    #[test]
    fn decision_is_true_only_for_dash_sentinel_on_a_terminal() {
        assert!(
            is_interactive_decision(Path::new("-"), true),
            "`-` on a terminal must be interactive"
        );
    }

    #[test]
    fn decision_is_false_off_a_terminal_even_for_the_sentinel() {
        assert!(
            !is_interactive_decision(Path::new("-"), false),
            "`-` off a terminal must not be interactive"
        );
    }

    #[test]
    fn decision_is_false_for_a_file_path_on_a_terminal() {
        assert!(
            !is_interactive_decision(Path::new("some/file.txt"), true),
            "a file path is never the stdin sentinel"
        );
    }

    #[test]
    fn a_real_path_is_never_interactive() {
        assert!(
            !stdin_is_interactive(Path::new("some/file.txt")),
            "a file path must not be treated as interactive stdin"
        );
        assert!(
            guard_stdin(Path::new("some/file.txt"), "check", &lang("en")).is_none(),
            "a file path must never trip the stdin guard"
        );
    }

    #[test]
    fn piped_stdin_sentinel_is_not_interactive() {
        assert!(
            !stdin_is_interactive(Path::new("-")),
            "non-terminal stdin must not be interactive"
        );
        assert!(
            guard_stdin(Path::new("-"), "check", &lang("en")).is_none(),
            "non-terminal `-` must not trip the guard"
        );
    }
}
