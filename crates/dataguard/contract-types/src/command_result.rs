//! Contract H: what the DataGuard answers once a command is settled.
//!
//! Stricter than the schema, by milestone rule 32: an invalid status
//! combination can neither be built nor decoded.

use std::num::NonZeroU64;

use serde::{Deserialize, Serialize};

use crate::text::required;
use crate::{CommandId, ContractError, Violation, Violations};

/// The status of a settled command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandStatus {
    /// The command took effect.
    Applied,
    /// The command was refused.
    Rejected,
    /// The command expired.
    Expired,
    /// The command was cancelled.
    Cancelled,
}

/// How a command ended, with the data each status carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Applied, taking this global version. Version 0 means "nothing applied",
    /// so it is unrepresentable here although schema H allows it.
    Applied {
        /// The `dataVersion` the application took.
        data_version: NonZeroU64,
    },
    /// Rejected for these violations.
    Rejected {
        /// The violations, at least one.
        violations: Violations,
    },
    /// Cancelled before any effect.
    Cancelled,
    /// Expired before any effect.
    Expired,
}

/// The result of a command: five fields on the wire, `reviewId` always null
/// in this milestone.
///
/// Fields are private so the status cannot be set apart from its data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ResultWire", into = "ResultWire")]
pub struct CommandResult {
    command_id: CommandId,
    outcome: Outcome,
}

impl CommandResult {
    /// The command was applied at `data_version`.
    #[must_use]
    pub const fn applied(command_id: CommandId, data_version: NonZeroU64) -> Self {
        Self {
            command_id,
            outcome: Outcome::Applied { data_version },
        }
    }

    /// The command was rejected; this also covers a refusal before enqueue.
    #[must_use]
    pub const fn rejected(command_id: CommandId, violations: Violations) -> Self {
        Self {
            command_id,
            outcome: Outcome::Rejected { violations },
        }
    }

    /// The command was cancelled.
    #[must_use]
    pub const fn cancelled(command_id: CommandId) -> Self {
        Self {
            command_id,
            outcome: Outcome::Cancelled,
        }
    }

    /// The command expired.
    #[must_use]
    pub const fn expired(command_id: CommandId) -> Self {
        Self {
            command_id,
            outcome: Outcome::Expired,
        }
    }

    /// The command id.
    #[must_use]
    pub const fn command_id(&self) -> &CommandId {
        &self.command_id
    }

    /// How the command ended.
    #[must_use]
    pub const fn outcome(&self) -> &Outcome {
        &self.outcome
    }

    /// The status on the wire.
    #[must_use]
    pub const fn status(&self) -> CommandStatus {
        match self.outcome {
            Outcome::Applied { .. } => CommandStatus::Applied,
            Outcome::Rejected { .. } => CommandStatus::Rejected,
            Outcome::Cancelled => CommandStatus::Cancelled,
            Outcome::Expired => CommandStatus::Expired,
        }
    }

    /// The `dataVersion`, only for an applied command.
    #[must_use]
    pub const fn data_version(&self) -> Option<NonZeroU64> {
        match self.outcome {
            Outcome::Applied { data_version } => Some(data_version),
            _ => None,
        }
    }

    /// The violations, only for a rejected command.
    #[must_use]
    pub fn violations(&self) -> &[Violation] {
        match &self.outcome {
            Outcome::Rejected { violations } => violations.as_slice(),
            _ => &[],
        }
    }

    /// The review id: always `None` until a milestone ships review queues.
    #[must_use]
    pub const fn review_id(&self) -> Option<&str> {
        None
    }
}

/// The five fields as they travel. Every one is required, `null` included.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResultWire {
    command_id: String,
    status: CommandStatus,
    #[serde(deserialize_with = "required")]
    data_version: Option<u64>,
    violations: Vec<String>,
    #[serde(deserialize_with = "required")]
    review_id: Option<String>,
}

impl From<CommandResult> for ResultWire {
    fn from(result: CommandResult) -> Self {
        let status = result.status();
        let (data_version, violations) = match result.outcome {
            Outcome::Applied { data_version } => (Some(data_version.get()), Vec::new()),
            Outcome::Rejected { violations } => (
                None,
                violations
                    .into_vec()
                    .into_iter()
                    .map(String::from)
                    .collect(),
            ),
            Outcome::Cancelled | Outcome::Expired => (None, Vec::new()),
        };
        Self {
            command_id: result.command_id.into(),
            status,
            data_version,
            violations,
            review_id: None,
        }
    }
}

impl TryFrom<ResultWire> for CommandResult {
    type Error = ContractError;

    fn try_from(wire: ResultWire) -> Result<Self, ContractError> {
        let status = wire.status;
        let refuse = |rule, detail| ContractError::Status {
            status,
            rule,
            detail,
        };
        let command_id = CommandId::try_from(wire.command_id)?;
        let violations = wire
            .violations
            .into_iter()
            .map(Violation::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        if wire.review_id.is_some() {
            return Err(refuse(22, "reviewId must be null"));
        }
        let outcome = match status {
            CommandStatus::Applied => {
                let data_version = wire
                    .data_version
                    .and_then(NonZeroU64::new)
                    .ok_or_else(|| refuse(19, "dataVersion must be an integer >= 1"))?;
                if !violations.is_empty() {
                    return Err(refuse(19, "violations must be empty"));
                }
                Outcome::Applied { data_version }
            }
            CommandStatus::Rejected => {
                if wire.data_version.is_some() {
                    return Err(refuse(20, "dataVersion must be null"));
                }
                let violations = Violations::new(violations)
                    .map_err(|_| refuse(20, "violations must not be empty"))?;
                Outcome::Rejected { violations }
            }
            CommandStatus::Cancelled | CommandStatus::Expired => {
                if wire.data_version.is_some() {
                    return Err(refuse(21, "dataVersion must be null"));
                }
                if !violations.is_empty() {
                    return Err(refuse(21, "violations must be empty"));
                }
                if status == CommandStatus::Cancelled {
                    Outcome::Cancelled
                } else {
                    Outcome::Expired
                }
            }
        };
        Ok(Self {
            command_id,
            outcome,
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn id() -> CommandId {
        CommandId::new("c1").unwrap()
    }

    fn violations() -> Violations {
        Violations::new(vec![Violation::new("balance-not-negative").unwrap()]).unwrap()
    }

    fn parse(doc: Value) -> Result<CommandResult, serde_json::Error> {
        serde_json::from_value(doc)
    }

    fn doc(status: &str, data_version: Value, violations: Value, review_id: Value) -> Value {
        json!({
            "commandId": "c1",
            "status": status,
            "dataVersion": data_version,
            "violations": violations,
            "reviewId": review_id
        })
    }

    #[test]
    fn the_four_valid_shapes_round_trip_with_five_keys() {
        let cases = [
            (
                CommandResult::applied(id(), NonZeroU64::MIN),
                doc("applied", json!(1), json!([]), Value::Null),
            ),
            (
                CommandResult::rejected(id(), violations()),
                doc(
                    "rejected",
                    Value::Null,
                    json!(["balance-not-negative"]),
                    Value::Null,
                ),
            ),
            (
                CommandResult::cancelled(id()),
                doc("cancelled", Value::Null, json!([]), Value::Null),
            ),
            (
                CommandResult::expired(id()),
                doc("expired", Value::Null, json!([]), Value::Null),
            ),
        ];
        for (result, expected) in cases {
            let wire = serde_json::to_value(&result).unwrap();
            assert_eq!(wire, expected);
            assert_eq!(wire.as_object().unwrap().len(), 5);
            assert_eq!(parse(wire).unwrap(), result);
            assert_eq!(result.review_id(), None);
        }
    }

    #[test]
    fn getters_follow_the_outcome() {
        let applied = CommandResult::applied(id(), NonZeroU64::MIN);
        assert_eq!(applied.status(), CommandStatus::Applied);
        assert_eq!(applied.data_version(), Some(NonZeroU64::MIN));
        assert!(applied.violations().is_empty());
        let rejected = CommandResult::rejected(id(), violations());
        assert_eq!(rejected.status(), CommandStatus::Rejected);
        assert_eq!(rejected.data_version(), None);
        assert_eq!(rejected.violations().len(), 1);
    }

    #[test]
    fn invalid_combinations_are_refused() {
        let v = json!(["x"]);
        let refused = [
            doc("applied", json!(0), json!([]), Value::Null),
            doc("applied", Value::Null, json!([]), Value::Null),
            doc("applied", json!(1), v.clone(), Value::Null),
            doc("rejected", Value::Null, json!([]), Value::Null),
            doc("rejected", json!(1), v.clone(), Value::Null),
            doc("cancelled", Value::Null, v.clone(), Value::Null),
            doc("cancelled", json!(1), json!([]), Value::Null),
            doc("expired", Value::Null, v.clone(), Value::Null),
            doc("expired", json!(1), json!([]), Value::Null),
            doc("applied", json!(1), json!([]), json!("r1")),
            doc("rejected", Value::Null, v, json!("r1")),
            doc("cancelled", Value::Null, json!([]), json!("r1")),
            doc("rejected", Value::Null, json!([""]), Value::Null),
            doc("applied", json!(-1), json!([]), Value::Null),
            doc("applied", json!(1.5), json!([]), Value::Null),
            doc("applied", json!("1"), json!([]), Value::Null),
        ];
        for bad in refused {
            assert!(parse(bad.clone()).is_err(), "{bad}");
        }
    }

    #[test]
    fn refusal_is_a_typed_error_naming_the_rule() {
        let err = parse(doc("applied", json!(0), json!([]), Value::Null)).unwrap_err();
        assert!(err.to_string().contains("rule 19"), "{err}");
    }

    #[test]
    fn empty_command_id_missing_and_extra_fields_are_refused() {
        let mut empty = doc("cancelled", Value::Null, json!([]), Value::Null);
        empty["commandId"] = json!("");
        assert!(parse(empty).is_err());

        for key in [
            "commandId",
            "status",
            "dataVersion",
            "violations",
            "reviewId",
        ] {
            let mut missing = doc("cancelled", Value::Null, json!([]), Value::Null);
            missing.as_object_mut().unwrap().remove(key);
            assert!(parse(missing).is_err(), "missing {key}");
        }

        let mut extra = doc("cancelled", Value::Null, json!([]), Value::Null);
        extra["extra"] = json!(1);
        assert!(parse(extra).is_err());
    }
}
