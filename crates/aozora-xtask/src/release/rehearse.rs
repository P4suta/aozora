//! `xtask release rehearse` — dry-run the tag-driven publishers before the
//! real tag.
//!
//! STUB — implementation is authored by the DEV-64 follow-up. The signature is
//! fixed so `mod.rs` compiles and the offline core can ship ahead of the
//! dry-run dispatch machinery.

pub(super) fn run(commit: Option<&str>) -> Result<(), String> {
    Err(format!(
        "xtask release rehearse is not implemented yet (DEV-64 follow-up); \
         would rehearse {}",
        commit.unwrap_or("HEAD"),
    ))
}
