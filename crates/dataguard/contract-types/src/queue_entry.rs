//! Contract G: one command in the DataQueue.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::text::some;
use crate::{Actor, CommandId, CommandStatus, ContractError, DataCapabilityRef, Partition, Ttl};

/// A command in the queue, as the queue stores it.
///
/// Closed object: an unknown field is refused. No cross-field coherence is
/// checked (a `parked` block on a `queued` entry is valid, as in the schema).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueueEntry {
    /// The command id.
    pub command: CommandId,
    /// The exact data capability version the command targets.
    pub data_capability: DataCapabilityRef,
    /// The author.
    pub by: Actor,
    /// The partition, `Aggregate/id`.
    pub partition: Partition,
    /// The position in the partition, first is 0.
    pub position: u64,
    /// The base the command was built on.
    pub based_on: BasedOn,
    /// The projected state; omitted when absent, never `null`.
    #[serde(
        default,
        deserialize_with = "some",
        skip_serializing_if = "Option::is_none"
    )]
    pub projection: Option<Projection>,
    /// The value the author proposed. `Some(Null)` (clearing a field) is
    /// different from `None` (no value); `None` is omitted.
    #[serde(
        default,
        deserialize_with = "some",
        skip_serializing_if = "Option::is_none"
    )]
    pub your_value: Option<Value>,
    /// Where the command stands.
    pub state: QueueState,
    /// The confirmation; absent and `null` both read as `None`, which is omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation: Option<Confirmation>,
    /// The parking terms; absent and `null` both read as `None`, which is omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parked: Option<Parked>,
    /// The original command id after a requeue; absent and `null` both read
    /// as `None`, which is omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requeued_from: Option<String>,
}

/// The store version, and optionally the values, a command was built on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BasedOn {
    /// The `dataVersion` read; 0 is an empty store.
    pub version: u64,
    /// The values read; omitted when absent, never `null`.
    #[serde(
        default,
        deserialize_with = "some",
        skip_serializing_if = "Option::is_none"
    )]
    pub values: Option<Map<String, Value>>,
}

/// The state the command will see once the commands ahead of it are applied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Projection {
    /// The confirmed values.
    pub confirmed: Map<String, Value>,
    /// The commands ahead, possibly none.
    pub pending_ahead: Vec<PendingAhead>,
}

/// A command ahead in the partition. Open object: unknown fields are dropped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingAhead {
    /// The command id.
    pub command: String,
    /// Its author.
    pub by: String,
    /// Its proposed value; `Some(Null)` is different from `None`.
    #[serde(
        default,
        deserialize_with = "some",
        skip_serializing_if = "Option::is_none"
    )]
    pub value: Option<Value>,
    /// Its state, free text in the schema.
    pub state: String,
}

/// Who confirmed the command. Open object: unknown fields are dropped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Confirmation {
    /// The confirming actor.
    pub by: String,
}

/// How long a parked command is kept. Open object: unknown fields are dropped.
///
/// No default: a missing `ttl` or `onExpire` is an error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Parked {
    /// The duration, kept as written (`24h` and `1440m` stay distinct).
    pub ttl: Ttl,
    /// What happens at expiry.
    pub on_expire: OnExpire,
}

/// What happens to a parked command at expiry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnExpire {
    /// The command is dropped.
    Drop,
}

/// The nine states of a queue entry; the type does not narrow to the ones
/// this milestone reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueState {
    /// Waiting its turn.
    Queued,
    /// Waiting for its author to confirm an overwrite.
    AwaitingConfirmation,
    /// Confirmed by its author.
    Confirmed,
    /// Waiting for a human review.
    AwaitingReview,
    /// Parked until a hold is lifted.
    Parked,
    /// Applied (terminal).
    Applied,
    /// Rejected (terminal).
    Rejected,
    /// Cancelled (terminal).
    Cancelled,
    /// Expired (terminal).
    Expired,
}

impl QueueState {
    /// Whether the state is final: `applied`, `rejected`, `cancelled` or `expired`.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Applied | Self::Rejected | Self::Cancelled | Self::Expired
        )
    }
}

impl TryFrom<QueueState> for CommandStatus {
    type Error = ContractError;

    /// A terminal state maps to the status of the same spelling.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::NotTerminal`] for the five other states.
    fn try_from(state: QueueState) -> Result<Self, ContractError> {
        match state {
            QueueState::Applied => Ok(Self::Applied),
            QueueState::Rejected => Ok(Self::Rejected),
            QueueState::Cancelled => Ok(Self::Cancelled),
            QueueState::Expired => Ok(Self::Expired),
            QueueState::Queued
            | QueueState::AwaitingConfirmation
            | QueueState::Confirmed
            | QueueState::AwaitingReview
            | QueueState::Parked => Err(ContractError::NotTerminal { state }),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn base() -> Value {
        json!({
            "command": "c1",
            "dataCapability": "credit.requestLimitChange@2",
            "by": "gm",
            "partition": "Account/881",
            "position": 0,
            "basedOn": {"version": 0},
            "state": "queued"
        })
    }

    fn entry(doc: Value) -> Result<QueueEntry, serde_json::Error> {
        serde_json::from_value(doc)
    }

    #[test]
    fn all_states_round_trip_with_snake_case_names() {
        for name in [
            "queued",
            "awaiting_confirmation",
            "confirmed",
            "awaiting_review",
            "parked",
            "applied",
            "rejected",
            "cancelled",
            "expired",
        ] {
            let mut doc = base();
            doc["state"] = json!(name);
            let parsed = entry(doc.clone()).unwrap();
            assert_eq!(serde_json::to_value(&parsed).unwrap(), doc);
        }
        let mut doc = base();
        doc["state"] = json!("bogus");
        assert!(entry(doc).is_err());
    }

    #[test]
    fn terminal_states_map_to_the_status_of_the_same_name() {
        for (state, status) in [
            (QueueState::Applied, CommandStatus::Applied),
            (QueueState::Rejected, CommandStatus::Rejected),
            (QueueState::Cancelled, CommandStatus::Cancelled),
            (QueueState::Expired, CommandStatus::Expired),
        ] {
            assert!(state.is_terminal());
            assert_eq!(CommandStatus::try_from(state), Ok(status));
        }
        for state in [
            QueueState::Queued,
            QueueState::AwaitingConfirmation,
            QueueState::Confirmed,
            QueueState::AwaitingReview,
            QueueState::Parked,
        ] {
            assert!(!state.is_terminal());
            assert_eq!(
                CommandStatus::try_from(state),
                Err(ContractError::NotTerminal { state })
            );
        }
    }

    #[test]
    fn projection_null_is_refused_and_absent_is_omitted() {
        let mut doc = base();
        doc["projection"] = Value::Null;
        assert!(entry(doc).is_err());
        let out = serde_json::to_value(entry(base()).unwrap()).unwrap();
        assert!(out.get("projection").is_none());
    }

    #[test]
    fn your_value_null_survives_as_a_present_null() {
        let mut doc = base();
        doc["yourValue"] = Value::Null;
        let parsed = entry(doc.clone()).unwrap();
        assert_eq!(parsed.your_value, Some(Value::Null));
        assert_eq!(serde_json::to_value(&parsed).unwrap(), doc);
        assert_eq!(entry(base()).unwrap().your_value, None);
    }

    #[test]
    fn pending_ahead_value_null_survives() {
        let mut doc = base();
        doc["projection"] = json!({
            "confirmed": {},
            "pendingAhead": [{"command": "c0", "by": "gm", "value": null, "state": "queued"}]
        });
        let parsed = entry(doc.clone()).unwrap();
        assert_eq!(serde_json::to_value(&parsed).unwrap(), doc);
    }

    #[test]
    fn none_blocks_read_null_and_are_omitted_on_write() {
        let mut doc = base();
        doc["confirmation"] = Value::Null;
        doc["parked"] = Value::Null;
        doc["requeuedFrom"] = Value::Null;
        let parsed = entry(doc).unwrap();
        assert!(parsed.confirmation.is_none());
        assert!(parsed.parked.is_none());
        assert!(parsed.requeued_from.is_none());
        assert_eq!(serde_json::to_value(&parsed).unwrap(), base());
    }

    #[test]
    fn unknown_fields_are_refused_on_closed_objects_only() {
        let mut top = base();
        top["extra"] = json!(1);
        assert!(entry(top).is_err());

        let mut based_on = base();
        based_on["basedOn"]["extra"] = json!(1);
        assert!(entry(based_on).is_err());

        let mut projection = base();
        projection["projection"] = json!({"confirmed": {}, "pendingAhead": [], "extra": 1});
        assert!(entry(projection).is_err());

        let mut open = base();
        open["projection"] = json!({
            "confirmed": {},
            "pendingAhead": [{"command": "c", "by": "g", "state": "s", "extra": 1}]
        });
        open["confirmation"] = json!({"by": "gm", "extra": 1});
        open["parked"] = json!({"ttl": "5m", "onExpire": "drop", "extra": 1});
        let out = serde_json::to_value(entry(open).unwrap()).unwrap();
        assert!(out["projection"]["pendingAhead"][0].get("extra").is_none());
        assert!(out["confirmation"].get("extra").is_none());
        assert!(out["parked"].get("extra").is_none());
    }

    #[test]
    fn parked_needs_ttl_and_a_drop_expiry() {
        for parked in [
            json!({"onExpire": "drop"}),
            json!({"ttl": "5m"}),
            json!({"ttl": "24x", "onExpire": "drop"}),
            json!({"ttl": "5m", "onExpire": "keep"}),
        ] {
            let mut doc = base();
            doc["parked"] = parked;
            assert!(entry(doc).is_err());
        }
    }

    #[test]
    fn values_outside_the_schema_are_errors() {
        for (key, value) in [
            ("partition", json!("Account")),
            ("position", json!(-1)),
            ("dataCapability", json!("credit.x")),
            ("command", json!("")),
            ("by", json!("")),
        ] {
            let mut doc = base();
            doc[key] = value;
            assert!(entry(doc).is_err(), "{key}");
        }
    }
}
