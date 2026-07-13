//! Shared harness for the CLI integration suites that spawn the real
//! `aozora` binary.
//!
//! Every config-loading subcommand (`check` / `lint` / `render` / `inspect`
//! / `fmt`) reads a *global* config layer at
//! `$XDG_CONFIG_HOME/aozora/config.toml` (falling back to
//! `$HOME/.config/aozora/config.toml` when `XDG_CONFIG_HOME` is unset or
//! empty). Left to the ambient environment, a developer's real
//! `~/.config/aozora/config.toml` would bleed into these assertions and flip
//! their results. Routing every spawn through [`hermetic_command`] pins
//! `XDG_CONFIG_HOME` at a process-wide *empty* tempdir, which seals BOTH
//! lookups at once: the tempdir holds no `aozora/` subdir, so the XDG path
//! resolves to a file that does not exist, and a non-empty `XDG_CONFIG_HOME`
//! means the `HOME` fallback is never consulted.
//!
//! The tempdir is created once per test binary (lazily, on the first spawn)
//! and kept for the life of the process — a `static` is never dropped, so it
//! is left for the OS to reap; the only contract is that it stay empty and
//! alive while commands spawn, which costs a single `mkdir` no matter how
//! many commands a suite runs.
//!
//! Message language is pinned too. Since `LANG` is now the lowest-priority
//! source for *message* language (`--lang > AOZORA_LANG > config.lang >
//! LANG`), a developer's real `LANG=ja_JP.UTF-8` would flip English message
//! assertions to Japanese. [`hermetic_command`] therefore pins
//! `AOZORA_LANG=en` and strips `LANG` / `LC_ALL`, so the human shell strings
//! (and the help / version / message snapshots) are deterministic regardless
//! of the host locale. A test that needs another language passes `--lang`,
//! which outranks the pinned `AOZORA_LANG`.

use std::path::Path;
use std::process::Command;
use std::sync::LazyLock;

use tempfile::TempDir;

/// The `aozora` binary under test — Cargo exports its path to integration
/// tests through this env var.
const BIN: &str = env!("CARGO_BIN_EXE_aozora");

/// Process-wide empty directory pinned as `XDG_CONFIG_HOME`.
static EMPTY_XDG_CONFIG_HOME: LazyLock<TempDir> =
    LazyLock::new(|| TempDir::new().expect("create empty XDG_CONFIG_HOME tempdir"));

/// An empty directory to point `XDG_CONFIG_HOME` at. Because it holds no
/// `aozora/` subdir the global-config path resolves to nothing, and because
/// it is a non-empty *path* the `HOME` fallback is never consulted — so both
/// global-config lookups are sealed off from the host environment.
pub(crate) fn empty_xdg_config_home() -> &'static Path {
    EMPTY_XDG_CONFIG_HOME.path()
}

/// A [`Command`] for the `aozora` binary with the global config layer already
/// sealed: `XDG_CONFIG_HOME` is pinned at the shared [`empty_xdg_config_home`]
/// dir. Every suite here spawns through this instead of
/// `Command::new(CARGO_BIN_EXE_aozora)`, so no run can read a developer's real
/// `~/.config/aozora/config.toml`.
pub(crate) fn hermetic_command() -> Command {
    let mut cmd = Command::new(BIN);
    cmd.env("XDG_CONFIG_HOME", empty_xdg_config_home());
    // Pin the message language: `AOZORA_LANG=en` beats the `LANG` fallback, and
    // stripping `LANG` / `LC_ALL` removes the host locale from the chain
    // entirely, so message output stays English (and deterministic) unless a
    // test passes `--lang`.
    cmd.env("AOZORA_LANG", "en")
        .env_remove("LANG")
        .env_remove("LC_ALL");
    cmd
}
