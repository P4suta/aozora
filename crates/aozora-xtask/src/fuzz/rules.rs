#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Outcome {
    Clean,
    InputFailure,
    Incomplete,
}

pub(crate) const FAILURE_EXIT: i32 = 77;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Artifacts {
    Missing,
    FailureInput,
    Unclassified,
}

pub(crate) fn artifacts(count: usize, name: &[u8]) -> Artifacts {
    if count == 0 {
        Artifacts::Missing
    } else if count == 1 && (name.starts_with(b"crash-") || name.starts_with(b"leak-")) {
        Artifacts::FailureInput
    } else {
        Artifacts::Unclassified
    }
}

pub(crate) const fn classify(
    exit: Option<i32>,
    artifacts: Artifacts,
    replay_exit: Option<i32>,
) -> Outcome {
    match (exit, artifacts, replay_exit) {
        (Some(0), Artifacts::Missing, None) => Outcome::Clean,
        (Some(FAILURE_EXIT), Artifacts::FailureInput, Some(FAILURE_EXIT)) => Outcome::InputFailure,
        _ => Outcome::Incomplete,
    }
}

#[cfg(kani)]
mod proofs {
    use super::{Artifacts, FAILURE_EXIT, Outcome, artifacts, classify};

    const LINUX_NAME_BYTES: usize = 255;

    #[kani::proof]
    #[kani::unwind(8)]
    fn only_a_completed_run_or_a_reproduced_input_has_a_verdict() {
        let exit: Option<i32> = kani::any();
        let count: usize = kani::any();
        let bytes: [u8; LINUX_NAME_BYTES] = kani::any();
        let length: u8 = kani::any();
        let name = &bytes[..usize::from(length)];
        let inputs = artifacts(count, name);
        let failure_name = (name.len() >= 6 && &name[..6] == b"crash-")
            || (name.len() >= 5 && &name[..5] == b"leak-");
        assert_eq!(inputs == Artifacts::Missing, count == 0);
        assert_eq!(
            inputs == Artifacts::FailureInput,
            count == 1 && failure_name
        );
        let replay: Option<i32> = kani::any();
        let result = classify(exit, inputs, replay);
        assert_eq!(
            result == Outcome::Clean,
            exit == Some(0) && count == 0 && replay.is_none()
        );
        assert_eq!(
            result == Outcome::InputFailure,
            exit == Some(FAILURE_EXIT)
                && count == 1
                && failure_name
                && replay == Some(FAILURE_EXIT)
        );
        assert_eq!(
            result == Outcome::Incomplete,
            result != Outcome::Clean && result != Outcome::InputFailure
        );
        kani::cover!(result == Outcome::Clean);
        kani::cover!(result == Outcome::InputFailure);
        kani::cover!(result == Outcome::Incomplete);
    }
}

#[cfg(kani)]
mod rejecting_controls {
    use super::{Artifacts, FAILURE_EXIT, Outcome, artifacts, classify};

    #[kani::proof]
    fn reject_a_startup_error_as_an_input_failure() {
        assert_eq!(
            classify(Some(1), Artifacts::Missing, None),
            Outcome::InputFailure
        );
    }

    #[kani::proof]
    fn reject_a_failure_without_a_retained_input() {
        assert_eq!(
            classify(Some(FAILURE_EXIT), Artifacts::Missing, Some(FAILURE_EXIT)),
            Outcome::InputFailure
        );
    }

    #[kani::proof]
    fn reject_an_input_that_replays_successfully() {
        assert_eq!(
            classify(Some(FAILURE_EXIT), Artifacts::FailureInput, Some(0)),
            Outcome::InputFailure
        );
    }

    #[kani::proof]
    fn reject_a_replay_startup_error_as_an_input_failure() {
        assert_eq!(
            classify(Some(FAILURE_EXIT), Artifacts::FailureInput, Some(1)),
            Outcome::InputFailure
        );
    }

    #[kani::proof]
    fn reject_an_interrupted_run_as_completed() {
        assert_eq!(classify(None, Artifacts::Missing, None), Outcome::Clean);
    }

    #[kani::proof]
    fn reject_an_oom_artifact_as_a_crash() {
        assert_eq!(
            classify(
                Some(FAILURE_EXIT),
                artifacts(1, b"oom-input"),
                Some(FAILURE_EXIT)
            ),
            Outcome::InputFailure
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{Artifacts, FAILURE_EXIT, Outcome, artifacts, classify};

    #[test]
    fn startup_failure_is_incomplete_without_an_input() {
        assert_eq!(
            classify(Some(1), Artifacts::Missing, None),
            Outcome::Incomplete
        );
    }

    #[test]
    fn only_a_retained_input_that_reproduces_can_support_a_failure_report() {
        for exit in [None, Some(0), Some(1), Some(FAILURE_EXIT), Some(78)] {
            for inputs in [
                Artifacts::Missing,
                Artifacts::FailureInput,
                Artifacts::Unclassified,
            ] {
                for replay in [None, Some(0), Some(1), Some(FAILURE_EXIT)] {
                    let expected =
                        if exit == Some(0) && inputs == Artifacts::Missing && replay.is_none() {
                            Outcome::Clean
                        } else if exit == Some(FAILURE_EXIT)
                            && inputs == Artifacts::FailureInput
                            && replay == Some(FAILURE_EXIT)
                        {
                            Outcome::InputFailure
                        } else {
                            Outcome::Incomplete
                        };
                    assert_eq!(classify(exit, inputs, replay), expected);
                }
            }
        }
    }

    #[test]
    fn only_one_crash_or_leak_input_can_support_a_report() {
        assert_eq!(artifacts(1, b"crash-input"), Artifacts::FailureInput);
        assert_eq!(artifacts(1, b"leak-input"), Artifacts::FailureInput);
        for name in [b"oom-input".as_slice(), b"timeout-input", b"other", b""] {
            assert_eq!(artifacts(1, name), Artifacts::Unclassified);
        }
        assert_eq!(artifacts(0, b"crash-old"), Artifacts::Missing);
        assert_eq!(artifacts(2, b"crash-input"), Artifacts::Unclassified);
    }
}
