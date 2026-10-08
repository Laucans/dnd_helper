//! Contract J: what the Resolver hands the DataGuard for a command that
//! touches existing data.

use serde::{Deserialize, Serialize};

use crate::text::some;
use crate::{ContractError, ReaderRef};

/// What kind of write the command is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Classification {
    /// Creates a row.
    Insert,
    /// Changes a row.
    Update,
    /// Removes a row.
    Delete,
    /// Creates or changes a row.
    Upsert,
}

/// What happens to the rows a relation points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CascadePolicy {
    /// Refuse while rows depend on it.
    Restrict,
    /// Apply the change to the dependent rows.
    Cascade,
    /// Clear the reference.
    Nullify,
    /// Archive the dependent rows.
    Archive,
    /// Send to a human.
    Review,
}

/// Whether the invariants hold once the command is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvariantsAfter {
    /// They hold.
    Ok,
    /// At least one is violated.
    Violated,
}

/// What the Resolver decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// Apply as is.
    Auto,
    /// Ask the author.
    Confirm,
    /// Ask a human.
    HumanReview,
    /// Cannot proceed.
    Blocked,
    /// Refused.
    Rejected,
}

/// A hold on data the command touches. Open object: unknown fields are
/// dropped. `expires` is opaque.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hold {
    /// The hold id.
    pub id: String,
    /// The hold owner.
    pub by: String,
    /// When the hold ends, in whatever format the holder writes.
    pub expires: String,
}

/// Who and what depends on the data the command touches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Dependencies {
    /// The capabilities reading the data, `system.name`.
    pub readers: Vec<ReaderRef>,
    /// The holds on the data.
    pub holds: Vec<Hold>,
    /// The commands queued on the same data.
    pub pending_commands: Vec<String>,
}

/// One relation hit by the command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CascadeItem {
    /// The relation name.
    pub relation: String,
    /// How many rows it reaches; 0 is valid.
    pub rows: u64,
    /// What happens to them.
    pub policy: CascadePolicy,
    /// Whether it blocks the command; omitted when unset, never `null`.
    #[serde(
        default,
        deserialize_with = "some",
        skip_serializing_if = "Option::is_none"
    )]
    pub blocking: Option<bool>,
    /// Whether the effect is semantic; omitted when unset, never `null`.
    #[serde(
        default,
        deserialize_with = "some",
        skip_serializing_if = "Option::is_none"
    )]
    pub semantic: Option<bool>,
}

/// The eight fields of an impact plan, all required.
///
/// This is the wire shape and the input of [`ImpactPlan::new`]; holding one
/// does not mean the plan is valid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImpactPlanFields {
    /// The command id, opaque.
    pub command: String,
    /// The kind of write.
    pub classification: Classification,
    /// Who and what depends on the data.
    pub dependencies: Dependencies,
    /// The relations hit.
    pub cascade: Vec<CascadeItem>,
    /// Derived values to recompute, opaque (`Risk@3(customer:881)`).
    pub invalidates: Vec<String>,
    /// Whether the invariants hold afterwards.
    pub invariants_after: InvariantsAfter,
    /// The decision.
    pub decision: Decision,
    /// Readable reasons; at least one for `blocked` and `rejected`.
    pub reasons: Vec<String>,
}

/// A valid impact plan.
///
/// The one cross-field rule: `blocked` and `rejected` carry a reason. Any
/// other combination is the Resolver's call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ImpactPlanFields", into = "ImpactPlanFields")]
pub struct ImpactPlan(ImpactPlanFields);

impl ImpactPlan {
    /// Builds a plan from its fields.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::ReasonsRequired`] when the decision is
    /// `blocked` or `rejected` and `reasons` is empty.
    pub fn new(fields: ImpactPlanFields) -> Result<Self, ContractError> {
        Self::try_from(fields)
    }

    /// The fields of the plan.
    #[must_use]
    pub const fn fields(&self) -> &ImpactPlanFields {
        &self.0
    }

    /// Gives the fields back.
    #[must_use]
    pub fn into_fields(self) -> ImpactPlanFields {
        self.0
    }
}

impl TryFrom<ImpactPlanFields> for ImpactPlan {
    type Error = ContractError;

    fn try_from(fields: ImpactPlanFields) -> Result<Self, ContractError> {
        let needs_reason = matches!(fields.decision, Decision::Blocked | Decision::Rejected);
        if needs_reason && fields.reasons.is_empty() {
            return Err(ContractError::ReasonsRequired {
                decision: fields.decision,
            });
        }
        Ok(Self(fields))
    }
}

impl From<ImpactPlan> for ImpactPlanFields {
    fn from(plan: ImpactPlan) -> Self {
        plan.0
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn base() -> Value {
        json!({
            "command": "c1",
            "classification": "update",
            "dependencies": {"readers": ["credit.requestLimitChange"], "holds": [], "pendingCommands": []},
            "cascade": [],
            "invalidates": ["Risk@3(customer:881)"],
            "invariantsAfter": "ok",
            "decision": "auto",
            "reasons": []
        })
    }

    fn plan(doc: Value) -> Result<ImpactPlan, serde_json::Error> {
        serde_json::from_value(doc)
    }

    #[test]
    fn blocked_and_rejected_need_a_reason() {
        for decision in ["blocked", "rejected"] {
            let mut doc = base();
            doc["decision"] = json!(decision);
            assert!(plan(doc.clone()).is_err(), "{decision}");
            doc["reasons"] = json!(["effet non supporté"]);
            let parsed = plan(doc.clone()).unwrap();
            assert_eq!(serde_json::to_value(parsed).unwrap(), doc);
        }
        for decision in ["auto", "confirm", "human_review"] {
            let mut doc = base();
            doc["decision"] = json!(decision);
            assert!(plan(doc).is_ok(), "{decision}");
        }
    }

    #[test]
    fn new_enforces_the_same_rule() {
        let mut fields = plan(base()).unwrap().into_fields();
        fields.decision = Decision::Blocked;
        assert_eq!(
            ImpactPlan::new(fields.clone()),
            Err(ContractError::ReasonsRequired {
                decision: Decision::Blocked
            })
        );
        fields.reasons.push("effet non supporté".into());
        assert!(ImpactPlan::new(fields).is_ok());
    }

    #[test]
    fn readers_carry_no_version() {
        let mut doc = base();
        doc["dependencies"]["readers"] = json!(["a.b@2"]);
        assert!(plan(doc).is_err());
    }

    #[test]
    fn cascade_flags_are_omitted_when_unset_and_null_is_refused() {
        let item = json!({"relation": "r", "rows": 0, "policy": "review"});
        let mut doc = base();
        doc["cascade"] = json!([item]);
        let out = serde_json::to_value(plan(doc.clone()).unwrap()).unwrap();
        assert_eq!(out, doc);

        for flag in ["blocking", "semantic"] {
            let mut bad = doc.clone();
            bad["cascade"][0][flag] = Value::Null;
            assert!(plan(bad).is_err(), "{flag}");
        }
    }

    #[test]
    fn unknown_fields_are_refused_on_closed_objects_only() {
        let mut top = base();
        top["extra"] = json!(1);
        assert!(plan(top).is_err());

        let mut dependencies = base();
        dependencies["dependencies"]["extra"] = json!(1);
        assert!(plan(dependencies).is_err());

        let mut cascade = base();
        cascade["cascade"] =
            json!([{"relation": "r", "rows": 1, "policy": "restrict", "extra": 1}]);
        assert!(plan(cascade).is_err());

        let mut hold = base();
        hold["dependencies"]["holds"] =
            json!([{"id": "h", "by": "gm", "expires": "soon", "extra": 1}]);
        let out = serde_json::to_value(plan(hold).unwrap()).unwrap();
        assert!(out["dependencies"]["holds"][0].get("extra").is_none());
    }

    #[test]
    fn unknown_enum_values_are_errors() {
        for (key, value) in [
            ("classification", "merge"),
            ("invariantsAfter", "maybe"),
            ("decision", "humanReview"),
        ] {
            let mut doc = base();
            doc[key] = json!(value);
            assert!(plan(doc).is_err(), "{key}");
        }
    }
}
