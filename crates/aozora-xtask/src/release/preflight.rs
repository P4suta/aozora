//! `xtask release preflight` — verify the rearm preconditions.
//!
//! STUB — implementation is authored by the DEV-64 follow-up. The signature
//! is fixed so `mod.rs` compiles and the offline core (`decision` / `ruleset`)
//! can ship and gate ahead of the deployed-state probes.

/// What to verify. Grouped into a struct rather than passed as loose flags so
/// the online/offline split and the first-publish acknowledgment stay named.
pub(super) struct Request<'a> {
    /// Skip every network probe — run only the repo-local checks.
    pub offline: bool,
    /// Acknowledge a known first-publish (a new crate / project the registry
    /// cannot auto-create), so preflight does not hard-stop on it.
    pub first_publish: bool,
    /// The commit being rearmed (defaults to HEAD when `None`).
    pub commit: Option<&'a str>,
}

pub(super) fn run(request: &Request<'_>) -> Result<(), String> {
    Err(format!(
        "xtask release preflight is not implemented yet (DEV-64 follow-up); \
         would verify commit {} (offline={}, first_publish={})",
        request.commit.unwrap_or("HEAD"),
        request.offline,
        request.first_publish,
    ))
}
