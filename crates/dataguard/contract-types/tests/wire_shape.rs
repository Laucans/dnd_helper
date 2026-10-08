//! Exact wire shapes: keys, `null` against omission, names.

mod common;

use std::num::NonZeroU64;

use common::{h_validator, l_validator};
use dataguard_contract_types::{
    CommandId, CommandResult, Message, QueueState, Recipients, Violation, Violations,
};
use serde_json::{Value, json};

fn keys(value: &Value) -> Vec<&str> {
    let mut keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    keys
}

fn id() -> CommandId {
    CommandId::new("c1").unwrap()
}

#[test]
fn h_always_writes_exactly_five_keys_with_explicit_nulls() {
    let rejected = Violations::new(vec![Violation::new("v1").unwrap()]).unwrap();
    let results = [
        CommandResult::applied(id(), NonZeroU64::MIN),
        CommandResult::rejected(id(), rejected),
        CommandResult::cancelled(id()),
        CommandResult::expired(id()),
    ];
    for result in results {
        let doc = serde_json::to_value(&result).unwrap();
        assert_eq!(
            keys(&doc),
            [
                "commandId",
                "dataVersion",
                "reviewId",
                "status",
                "violations"
            ]
        );
        assert!(doc["reviewId"].is_null());
        assert!(doc["violations"].is_array());
        assert!(h_validator().is_valid(&doc));
    }
    let doc = serde_json::to_value(CommandResult::cancelled(id())).unwrap();
    assert!(doc["dataVersion"].is_null());
}

#[test]
fn state_and_decision_names_are_snake_case_and_message_is_pascal_case() {
    assert_eq!(
        serde_json::to_value(QueueState::AwaitingConfirmation).unwrap(),
        json!("awaiting_confirmation")
    );
    let message = Message::CommandQueued(dataguard_contract_types::CommandQueued {
        to: Recipients::new(vec!["gm".into()]).unwrap(),
        command: id(),
    });
    let doc = serde_json::to_value(&message).unwrap();
    assert_eq!(doc["message"], "CommandQueued");
    assert!(l_validator().is_valid(&doc));
}

#[test]
fn l_never_emits_actions_nor_fields_of_other_variants() {
    let doc = json!({
        "message": "CommandApplied", "to": ["gm"], "command": "c1", "dataVersion": 2,
        "actions": ["cancel"], "violations": ["v"], "field": "f"
    });
    let message: Message = serde_json::from_value(doc).unwrap();
    let out = serde_json::to_value(message).unwrap();
    assert_eq!(keys(&out), ["command", "dataVersion", "message", "to"]);
}
