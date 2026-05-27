//! Shared outcome types and helpers used to normalize the result of
//! running a shell command across the various execution paths
//! (foreground, parallel, interactive).
//!
//! The parallel execution path delegates aggregation, exit-code derivation,
//! and body-visibility decisions to this module; the single-command and
//! interactive paths will migrate in later refactor steps.

use std::time::Duration;

use crate::{CommandOutput, RunStatus};

/// The terminal outcome of a single command execution, capturing both
/// the status classification and the raw output (or the fact that
/// output was inherited to the parent terminal).
pub(crate) struct Outcome {
    pub(crate) status: RunStatus,
    pub(crate) output: OutputCapture,
    pub(crate) elapsed: Duration,
    pub(crate) label: String,
    pub(crate) signal_num: Option<i32>,
}

/// Whether a command's output was captured into memory or inherited
/// directly to the parent's stdio.
pub(crate) enum OutputCapture {
    Captured(CommandOutput),
    Inherited,
}

/// Aggregate a slice of completed [`Outcome`]s into a single [`RunStatus`].
///
/// Semantics mirror `parallel::compute_parent_status` for completed runs:
/// - any `Interrupted` => `Interrupted`
/// - else all `Success` => `Success`
/// - else => `Failure` (Timeout folds in here)
/// - empty slice => `Success`
pub(crate) fn aggregate(outcomes: &[Outcome]) -> RunStatus {
    let mut all_success = true;
    for o in outcomes {
        if matches!(o.status, RunStatus::Interrupted) {
            return RunStatus::Interrupted;
        }
        if !matches!(o.status, RunStatus::Success) {
            all_success = false;
        }
    }
    if all_success {
        RunStatus::Success
    } else {
        RunStatus::Failure
    }
}

/// Derive a process exit code from an [`Outcome`].
///
/// - `Timeout` => `124`
/// - `Interrupted` => `128 + signal_num` (fallback `1`)
/// - `Success`/`Failure` with captured output => the command's real exit code
/// - `Success`/`Failure` with inherited output => `0` on success, `1` otherwise
pub(crate) fn exit_code(outcome: &Outcome) -> i32 {
    match outcome.status {
        RunStatus::Timeout => 124,
        RunStatus::Interrupted => outcome.signal_num.map_or(1, |n| 128 + n),
        RunStatus::Success | RunStatus::Failure => match &outcome.output {
            OutputCapture::Captured(cmd_output) => cmd_output.exit_code,
            OutputCapture::Inherited => i32::from(!matches!(outcome.status, RunStatus::Success)),
        },
    }
}

/// Whether the captured body (stdout/stderr) should be included in
/// user-facing output for a given status under a given quiet setting.
///
/// Bodies are suppressed only when the run was successful AND quiet
/// mode is enabled.
pub(crate) fn should_include_body(status: RunStatus, quiet: bool) -> bool {
    !matches!(status, RunStatus::Success) || !quiet
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_outcome(status: RunStatus, output: OutputCapture, signal_num: Option<i32>) -> Outcome {
        Outcome {
            status,
            output,
            elapsed: Duration::from_millis(0),
            label: String::new(),
            signal_num,
        }
    }

    fn captured(exit_code: i32) -> OutputCapture {
        OutputCapture::Captured(CommandOutput {
            stdout: String::new(),
            stderr: String::new(),
            exit_code,
        })
    }

    #[test]
    fn aggregate_empty_is_success() {
        assert_eq!(aggregate(&[]), RunStatus::Success);
    }

    #[test]
    fn aggregate_all_success() {
        let outcomes = [
            make_outcome(RunStatus::Success, captured(0), None),
            make_outcome(RunStatus::Success, captured(0), None),
        ];
        assert_eq!(aggregate(&outcomes), RunStatus::Success);
    }

    #[test]
    fn aggregate_mix_success_failure_is_failure() {
        let outcomes = [
            make_outcome(RunStatus::Success, captured(0), None),
            make_outcome(RunStatus::Failure, captured(1), None),
        ];
        assert_eq!(aggregate(&outcomes), RunStatus::Failure);
    }

    #[test]
    fn aggregate_interrupted_in_successes_is_interrupted() {
        let outcomes = [
            make_outcome(RunStatus::Success, captured(0), None),
            make_outcome(RunStatus::Interrupted, captured(130), Some(2)),
            make_outcome(RunStatus::Success, captured(0), None),
        ];
        assert_eq!(aggregate(&outcomes), RunStatus::Interrupted);
    }

    #[test]
    fn aggregate_interrupt_precedes_failure() {
        let outcomes = [
            make_outcome(RunStatus::Failure, captured(1), None),
            make_outcome(RunStatus::Interrupted, captured(130), Some(2)),
        ];
        assert_eq!(aggregate(&outcomes), RunStatus::Interrupted);
    }

    #[test]
    fn aggregate_timeout_folds_to_failure() {
        let outcomes = [
            make_outcome(RunStatus::Timeout, captured(124), None),
            make_outcome(RunStatus::Success, captured(0), None),
        ];
        assert_eq!(aggregate(&outcomes), RunStatus::Failure);
    }

    #[test]
    fn exit_code_success_captured_is_zero() {
        let o = make_outcome(RunStatus::Success, captured(0), None);
        assert_eq!(exit_code(&o), 0);
    }

    #[test]
    fn exit_code_failure_captured_returns_command_exit_code() {
        let o = make_outcome(RunStatus::Failure, captured(42), None);
        assert_eq!(exit_code(&o), 42);
    }

    #[test]
    fn exit_code_timeout_is_124() {
        let o = make_outcome(RunStatus::Timeout, captured(0), None);
        assert_eq!(exit_code(&o), 124);
    }

    #[test]
    fn exit_code_interrupted_with_sigterm_is_143() {
        let o = make_outcome(RunStatus::Interrupted, OutputCapture::Inherited, Some(15));
        assert_eq!(exit_code(&o), 143);
    }

    #[test]
    fn exit_code_interrupted_unknown_signal_is_1() {
        let o = make_outcome(RunStatus::Interrupted, OutputCapture::Inherited, None);
        assert_eq!(exit_code(&o), 1);
    }

    #[test]
    fn exit_code_success_inherited_is_zero() {
        let o = make_outcome(RunStatus::Success, OutputCapture::Inherited, None);
        assert_eq!(exit_code(&o), 0);
    }

    #[test]
    fn exit_code_failure_inherited_is_one() {
        let o = make_outcome(RunStatus::Failure, OutputCapture::Inherited, None);
        assert_eq!(exit_code(&o), 1);
    }

    #[test]
    fn should_include_body_success_quiet_suppressed() {
        assert!(!should_include_body(RunStatus::Success, true));
    }

    #[test]
    fn should_include_body_success_not_quiet_included() {
        assert!(should_include_body(RunStatus::Success, false));
    }

    #[test]
    fn should_include_body_failure_quiet_included() {
        assert!(should_include_body(RunStatus::Failure, true));
    }

    #[test]
    fn should_include_body_interrupted_quiet_included() {
        assert!(should_include_body(RunStatus::Interrupted, true));
    }

    #[test]
    fn should_include_body_timeout_quiet_included() {
        assert!(should_include_body(RunStatus::Timeout, true));
    }
}
