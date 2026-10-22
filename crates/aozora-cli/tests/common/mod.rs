use std::path::Path;
use std::process::Command;
use std::sync::LazyLock;

use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_aozora");

static EMPTY_XDG_CONFIG_HOME: LazyLock<TempDir> =
    LazyLock::new(|| TempDir::new().expect("create empty XDG_CONFIG_HOME tempdir"));

pub(crate) fn empty_xdg_config_home() -> &'static Path {
    EMPTY_XDG_CONFIG_HOME.path()
}

pub(crate) fn hermetic_command() -> Command {
    let mut cmd = Command::new(BIN);
    cmd.env("XDG_CONFIG_HOME", empty_xdg_config_home());
    cmd.env("AOZORA_LANG", "en")
        .env_remove("LANG")
        .env_remove("LC_ALL");
    cmd
}
