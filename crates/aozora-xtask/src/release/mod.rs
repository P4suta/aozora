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
