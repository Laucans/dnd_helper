//! The one error type of the crate.

use crate::{CommandStatus, Decision, QueueState};

/// A value that breaks a contract rule.
///
/// Messages name the field and the rule, and never echo the offending value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ContractError {
    /// A string or list that must not be empty is empty.
    #[error("`{field}` must not be empty")]
    Empty {
        /// The field refused.
        field: &'static str,
    },
    /// A string does not match the pattern of its schema.
    #[error("`{field}` does not match `{pattern}`")]
    Pattern {
        /// The field refused.
        field: &'static str,
        /// The pattern it has to match.
        pattern: &'static str,
    },
    /// A command result combines a status with fields the status forbids.
    #[error("command result `{status:?}` breaks milestone rule {rule}: {detail}")]
    Status {
        /// The status being decoded.
        status: CommandStatus,
        /// The milestone rule (19 to 22) it breaks.
        rule: u8,
        /// What is wrong.
        detail: &'static str,
    },
    /// A `blocked` or `rejected` plan carries no reason.
    #[error("an impact plan with decision `{decision:?}` needs at least one reason")]
    ReasonsRequired {
        /// The decision that needs a reason.
        decision: Decision,
    },
    /// A non-terminal queue state has no command result status.
    #[error("queue state `{state:?}` is not terminal, it has no command result status")]
    NotTerminal {
        /// The state that was converted.
        state: QueueState,
    },
    /// A command result that is not `rejected` was used as a refusal.
    #[error("command result `{status:?}` is not a rejection")]
    NotRejected {
        /// The status of the result.
        status: CommandStatus,
    },
}
