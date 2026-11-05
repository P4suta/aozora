use std::env;
use std::fs;
use std::io::{ErrorKind, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use clap::{Args, Subcommand};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::proofs;

mod rules;

const DEPENDENCY_MARKER: &str = "target/.deps-check.timestamp";
const GLOBAL_MISE_CONFIG: &str = "mise/config.toml";

#[derive(Args)]
pub(crate) struct DeveloperArgs {
    #[command(subcommand)]
    command: DeveloperCommand,
}

#[derive(Args)]
struct Root {
    #[arg(long, default_value = ".")]
    root: PathBuf,
}

#[derive(Subcommand)]
enum DeveloperCommand {
    /// Install locked tools, preserve the configured hooks, and run workspace tests.
    Setup(Root),
    /// Check native prerequisites on Linux and skip Linux-only checks on macOS.
    NativePrerequisites(Root),
    /// Report the last successful dependency check and reject missing or stale evidence.
    DepsStatus {
        #[command(flatten)]
        root: Root,
        /// Use a fixed whole-second UTC instant for reproducible inspection.
        #[arg(long)]
        now: Option<String>,
    },
    /// Create a proposed decision record with a descriptive filename and UTC date.
    NewAdr {
        #[command(flatten)]
        root: Root,
        title: String,
    },
    /// Install local hooks or preserve an existing configured dispatcher.
    Hooks(Root),
    /// Remove local Lefthook stubs while preserving a configured dispatcher.
    HooksUninstall(Root),
    /// Verify the complete developer proof inventory with the pinned Linux verifier.
    Prove(Root),
}

pub(crate) fn dispatch(args: DeveloperArgs) -> Result<(), String> {
    match args.command {
        DeveloperCommand::Setup(root) => setup(&root.root),
        DeveloperCommand::NativePrerequisites(root) => native_prerequisites(&root.root),
        DeveloperCommand::DepsStatus { root, now } => {
            let now = now
                .as_deref()
                .map_or_else(|| Ok(OffsetDateTime::now_utc()), parse_timestamp)?;
            deps_status(&root.root, now)
        }
        DeveloperCommand::NewAdr { root, title } => {
            new_adr(&root.root, &title, OffsetDateTime::now_utc())
        }
        DeveloperCommand::Hooks(root) => hooks(&root.root, "install"),
        DeveloperCommand::HooksUninstall(root) => hooks(&root.root, "uninstall"),
        DeveloperCommand::Prove(root) => proofs::verify(&root.root, proofs::DEVELOPER),
    }
}

fn run(root: &Path, program: &str, arguments: &[&str]) -> Result<(), String> {
    let status = Command::new(program)
        .current_dir(root)
        .args(arguments)
        .status()
        .map_err(|error| format!("start {program}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} {arguments:?} exited with {status}"))
    }
}

fn read_command(root: &Path, program: &str, arguments: &[&str]) -> Result<Output, String> {
    Command::new(program)
        .current_dir(root)
        .args(arguments)
        .output()
        .map_err(|error| format!("start {program}: {error}"))
}

fn setup(root: &Path) -> Result<(), String> {
    native_prerequisites(root)?;
    let config = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok_or("XDG_CONFIG_HOME or HOME is required to isolate the project tool installation")?;
    let status = Command::new("mise")
        .current_dir(root)
        .env("MISE_IGNORED_CONFIG_PATHS", config.join(GLOBAL_MISE_CONFIG))
        .args(["install", "--locked", "--jobs", "2"])
        .status()
        .map_err(|error| format!("start locked mise installation: {error}"))?;
    if !status.success() {
        return Err(format!("locked mise installation exited with {status}"));
    }
    hooks(root, "install")?;
    let status = Command::new("mise")
        .current_dir(root)
        .env("MISE_IGNORED_CONFIG_PATHS", config.join(GLOBAL_MISE_CONFIG))
        .args(["exec", "--locked", "--", "just", "test"])
        .status()
        .map_err(|error| format!("start locked workspace tests: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("workspace tests exited with {status}"))
    }
}

fn native_prerequisites(root: &Path) -> Result<(), String> {
    run(root, "bash", &["scripts/check-native-prerequisites.sh"])
}

fn parse_timestamp(value: &str) -> Result<OffsetDateTime, String> {
    let timestamp = OffsetDateTime::parse(value, &Rfc3339)
        .map_err(|error| format!("invalid dependency-check timestamp: {error}"))?;
    let canonical = timestamp
        .format(&Rfc3339)
        .map_err(|error| format!("format dependency-check timestamp: {error}"))?;
    if !timestamp.offset().is_utc() || timestamp.nanosecond() != 0 || canonical != value {
        return Err("dependency-check timestamps require canonical whole-second UTC form".into());
    }
    Ok(timestamp)
}

#[expect(
    clippy::print_stdout,
    reason = "the command reports dependency-check freshness"
)]
fn deps_status(root: &Path, now: OffsetDateTime) -> Result<(), String> {
    let path = root.join(DEPENDENCY_MARKER);
    let recorded = fs::read_to_string(&path).map_err(|error| {
        if error.kind() == ErrorKind::NotFound {
            "dependency check never run; run `just deps-check`".into()
        } else {
            format!("read {}: {error}", path.display())
        }
    })?;
    let checked = parse_timestamp(recorded.trim_end_matches(['\r', '\n']))?;
    let now =
        u64::try_from(now.unix_timestamp()).map_err(|error| format!("current time: {error}"))?;
    let checked = u64::try_from(checked.unix_timestamp())
        .map_err(|error| format!("dependency-check time: {error}"))?;
    match rules::freshness(now, checked) {
        rules::Freshness::Fresh { elapsed } => {
            let days = elapsed / rules::SECONDS_PER_DAY;
            println!("[deps-status] last check {days} days ago: fresh");
            Ok(())
        }
        rules::Freshness::Stale { elapsed } => {
            let days = elapsed / rules::SECONDS_PER_DAY;
            Err(format!(
                "last dependency check {days} days ago: stale; run `just deps-check`"
            ))
        }
        rules::Freshness::Future => Err("dependency-check timestamp is in the future".into()),
    }
}

#[expect(
    clippy::print_stdout,
    reason = "the command reports the created decision record"
)]
fn new_adr(root: &Path, title: &str, now: OffsetDateTime) -> Result<(), String> {
    if title.chars().any(char::is_control) {
        return Err("decision titles cannot contain control characters".into());
    }
    let mut slug = String::new();
    for character in title.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        return Err("decision titles must contain an ASCII letter or digit".into());
    }
    let directory = root.join("docs/adr");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("create {}: {error}", directory.display()))?;
    let path = directory.join(format!("{slug}.md"));
    let mut file = tempfile::NamedTempFile::new_in(&directory)
        .map_err(|error| format!("create {}: {error}", path.display()))?;
    let body = format!(
        "---\ndate: {}\n---\n\n# {title}\n\nStatus: proposed.\n\n## Context\n\n## Decision\n\n## Consequences\n\n## Alternatives considered\n\n## References\n",
        now.date()
    );
    file.write_all(body.as_bytes())
        .and_then(|()| file.as_file().sync_all())
        .map_err(|error| format!("write {}: {error}", path.display()))?;
    file.persist_noclobber(&path)
        .map_err(|error| format!("publish {}: {error}", path.display()))?;
    println!("Created {}", path.display());
    Ok(())
}

#[expect(
    clippy::print_stdout,
    reason = "the command identifies the preserved hook dispatcher"
)]
fn hooks(root: &Path, operation: &str) -> Result<(), String> {
    let configured = read_command(
        root,
        "git",
        &["config", "--path", "--get", "core.hooksPath"],
    )?;
    if configured.status.success() {
        let path = String::from_utf8(configured.stdout)
            .map_err(|error| format!("configured hook path is not UTF-8: {error}"))?;
        let path = path.trim_end_matches(['\r', '\n']);
        if path.is_empty() || !root.join(path).is_dir() {
            return Err(format!(
                "configured hook dispatcher directory is unavailable: {path}"
            ));
        }
        run(root, "lefthook", &["validate"])?;
        println!("Preserving configured core.hooksPath: {path}");
        Ok(())
    } else if configured.status.code() == Some(1) {
        run(root, "lefthook", &[operation])
    } else {
        Err(format!(
            "read core.hooksPath: {}",
            String::from_utf8_lossy(&configured.stderr)
        ))
    }
}
