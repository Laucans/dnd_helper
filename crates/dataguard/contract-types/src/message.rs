//! Contract L: what the DataGuard sends to authors and infrastructure.
//!
//! Only the four messages this milestone emits are representable. The schema
//! is an open object, so unknown fields are accepted on read, dropped, and
//! never re-emitted.
//!
//! Stricter than the schema, on purpose: the eight other message names are
//! errors, `command` is required and non-empty, `CommandApplied` needs a
//! `dataVersion` of at least 1, and the rejection messages need violations.

use std::num::NonZeroU64;

use serde::{Deserialize, Serialize};

use crate::text::some;
use crate::{CommandId, CommandResult, ContractError, Recipients, Violations};

/// A message, discriminated by the `message` field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "message")]
pub enum Message {
    /// A command entered the queue.
    CommandQueued(CommandQueued),
    /// A command was applied.
    CommandApplied(CommandApplied),
    /// A command was rejected.
    CommandRejected(CommandRejected),
    /// An invariant is threatened by a queued command.
    InvariantAtRisk(InvariantAtRisk),
}

/// Payload of [`Message::CommandQueued`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandQueued {
    /// The recipients, at least one.
    pub to: Recipients,
    /// The command.
    pub command: CommandId,
}

/// Payload of [`Message::CommandApplied`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandApplied {
    /// The recipients, at least one.
    pub to: Recipients,
    /// The command.
    pub command: CommandId,
    /// The version the application took, at least 1.
    pub data_version: NonZeroU64,
}

/// Payload of [`Message::CommandRejected`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandRejected {
    /// The recipients, at least one.
    pub to: Recipients,
    /// The command.
    pub command: CommandId,
    /// The violations, at least one.
    pub violations: Violations,
    /// A readable reason; omitted when absent, never `null`.
    #[serde(
        default,
        deserialize_with = "some",
        skip_serializing_if = "Option::is_none"
    )]
    pub reason: Option<String>,
}

/// Payload of [`Message::InvariantAtRisk`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvariantAtRisk {
    /// The recipients, at least one.
    pub to: Recipients,
    /// The command.
    pub command: CommandId,
    /// The ids of the invariants at risk, at least one.
    pub violations: Violations,
    /// A readable reason; omitted when absent, never `null`.
    #[serde(
        default,
        deserialize_with = "some",
        skip_serializing_if = "Option::is_none"
    )]
    pub reason: Option<String>,
}

impl CommandRejected {
    /// Builds the rejection message of a `rejected` result, carrying the
    /// same violations.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::NotRejected`] when the result is not `rejected`.
    pub fn from_result(
        result: &CommandResult,
        to: Recipients,
        reason: Option<String>,
    ) -> Result<Self, ContractError> {
        match result.outcome() {
            crate::Outcome::Rejected { violations } => Ok(Self {
                to,
                command: result.command_id().clone(),
                violations: violations.clone(),
                reason,
            }),
            _ => Err(ContractError::NotRejected {
                status: result.status(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::Violation;

    fn parse(doc: Value) -> Result<Message, serde_json::Error> {
        serde_json::from_value(doc)
    }

    #[test]
    fn the_eight_other_schema_names_and_a_bogus_one_are_errors() {
        for name in [
            "ValueDeclaredAhead",
            "BlockedByHold",
            "HoldExpiring",
            "ReviewRequired",
            "Parked",
            "ParkedExpiring",
            "Expired",
            "AheadResolved",
            "Bogus",
        ] {
            let doc = json!({"message": name, "to": ["gm"], "command": "c1"});
            assert!(parse(doc).is_err(), "{name}");
        }
    }

    #[test]
    fn empty_to_and_missing_fields_are_refused() {
        assert!(parse(json!({"message": "CommandQueued", "to": [], "command": "c1"})).is_err());
        assert!(parse(json!({"message": "CommandQueued", "command": "c1"})).is_err());
        assert!(parse(json!({"message": "CommandQueued", "to": ["gm"]})).is_err());
        assert!(parse(json!({"to": ["gm"], "command": "c1"})).is_err());
    }

    #[test]
    fn applied_needs_a_data_version_of_at_least_one() {
        let ok =
            json!({"message": "CommandApplied", "to": ["gm"], "command": "c1", "dataVersion": 1});
        assert!(parse(ok).is_ok());
        for bad in [json!(0), json!(-1), Value::Null] {
            let doc = json!({"message": "CommandApplied", "to": ["gm"], "command": "c1", "dataVersion": bad});
            assert!(parse(doc).is_err());
        }
        let missing = json!({"message": "CommandApplied", "to": ["gm"], "command": "c1"});
        assert!(parse(missing).is_err());
    }

    #[test]
    fn rejected_and_at_risk_need_violations() {
        for name in ["CommandRejected", "InvariantAtRisk"] {
            for violations in [json!([]), json!([""]), Value::Null] {
                let doc = json!({"message": name, "to": ["gm"], "command": "c1", "violations": violations});
                assert!(parse(doc).is_err(), "{name}");
            }
            let missing = json!({"message": name, "to": ["gm"], "command": "c1"});
            assert!(parse(missing).is_err(), "{name}");
            let reason_null = json!({"message": name, "to": ["gm"], "command": "c1", "violations": ["v"], "reason": null});
            assert!(parse(reason_null).is_err(), "{name}");
        }
    }

    #[test]
    fn extra_fields_are_accepted_and_never_re_emitted() {
        let doc = json!({
            "message": "CommandQueued", "to": ["gm"], "command": "c1",
            "actions": ["cancel"], "field": "f", "dataVersion": 3
        });
        let out = serde_json::to_value(parse(doc).unwrap()).unwrap();
        assert_eq!(
            out,
            json!({"message": "CommandQueued", "to": ["gm"], "command": "c1"})
        );
    }

    #[test]
    fn from_result_copies_the_violations_of_a_rejection_only() {
        let violations = Violations::new(vec![Violation::new("v1").unwrap()]).unwrap();
        let id = CommandId::new("c1").unwrap();
        let to = Recipients::new(vec!["gm".into()]).unwrap();

        let rejected = CommandResult::rejected(id.clone(), violations.clone());
        let message = CommandRejected::from_result(&rejected, to.clone(), None).unwrap();
        assert_eq!(message.violations, violations);
        assert_eq!(message.command, id);

        let applied = CommandResult::applied(id, NonZeroU64::MIN);
        assert_eq!(
            CommandRejected::from_result(&applied, to, None),
            Err(ContractError::NotRejected {
                status: crate::CommandStatus::Applied
            })
        );
    }
}
