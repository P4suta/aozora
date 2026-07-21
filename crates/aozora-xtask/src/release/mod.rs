//! Release rearm guard tooling (DEV-64).
//!
//! The rearm runbook, made executable. CI-green never proves the paths that
//! run only at tag push, only on a first publish, or only against deployed
//! registry/environment state — so the maintainer used to verify them by hand
//! against `docs/contrib/releasing-secrets.md`. This module gives that
//! on-demand tier the same executable, fail-closed, fail-on-zero treatment the
//! offline `drift-gate` checks already have, and leaves only the irreducibly
//! external residue as prose.
//!
//! * [`decision`] — the pure version-standstill accept/reject logic, lifted
//!   out of `release-plz.yml` (its inline branch was never exercised).
//! * [`ruleset`] — offline source-integrity, folded into `drift-gate`.
//! * [`preflight`] — deployed-state verification at rearm (`gh` / registries).
//! * [`rehearse`] — dry-run the tag-driven publishers before the real tag.

use crate::ReleaseArgs;
use crate::ReleaseOp;

mod decision;
mod preflight;
mod rehearse;
mod ruleset;

pub(crate) fn dispatch(args: &ReleaseArgs) -> Result<(), String> {
    match &args.op {
        ReleaseOp::RearmDecision {
            event,
            version_changed,
            commit,
        } => decision::run(event, version_changed, commit),
        ReleaseOp::Check => ruleset::check(),
        ReleaseOp::Preflight {
            offline,
            commit,
            first_publish,
        } => preflight::run(&preflight::Request {
            offline: *offline,
            first_publish: *first_publish,
            commit: commit.as_deref(),
        }),
        ReleaseOp::Rehearse { commit } => rehearse::run(commit.as_deref()),
    }
}
