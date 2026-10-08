//! Differential tests: the hand-written checks of the types against an
//! independent oracle.
//!
//! The patterns are checked by hand on ASCII bytes, so the real schema
//! validator is the oracle for them. The H and J cross-field rules are small
//! decision tables, walked exhaustively against a restatement of the rule.
//! Last, a mutated valid document must never be accepted by the type when
//! the schema refuses it (rule 7: never looser).

mod common;

use common::{
    arb_command_result, arb_impact_plan, arb_message, arb_queue_entry, g_validator, h_validator,
    j_validator, l_validator,
};
use dataguard_contract_types::{CommandResult, ImpactPlan, Message, QueueEntry};
use proptest::prelude::*;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

/// Strings that are valid, near-valid or hostile for the four patterns:
/// line breaks, non-ASCII letters and digits, case-fold look-alikes.
fn arb_pattern_candidate() -> impl Strategy<Value = String> {
    let hostile = prop::sample::select(vec![
        'a', 'z', 'A', 'Z', '0', '9', '-', '_', '.', '@', '/', ' ', 'm', 's', 'h', 'd', '\n', '\r',
        '\u{2028}', '\u{2029}', '\u{0}', 'é', 'İ', '\u{212A}', '٣', '０',
    ]);
    prop_oneof![
        prop::collection::vec(hostile, 0..10).prop_map(|chars| chars.into_iter().collect()),
        // Valid shapes, then a hostile tail or head.
        "[a-z][a-zA-Z0-9_-]{0,4}\\.[A-Za-z][A-Za-z0-9_-]{0,4}(@[0-9]{1,3})?[\\n\\r a-z@é٣]{0,2}",
        "[A-Z][A-Za-z0-9]{0,4}/[ -~\\n\\r\u{2028}é]{0,4}",
        "[0-9٣]{0,3}(ms|s|m|h|d|S|M|x)?[\\n ]{0,1}",
    ]
}

/// The type and the schema give the same verdict on one document.
fn assert_same_verdict<T: DeserializeOwned>(
    validator: &jsonschema::Validator,
    doc: &Value,
    field: &str,
) -> Result<(), TestCaseError> {
    let schema_ok = validator.is_valid(doc);
    let type_ok = serde_json::from_value::<T>(doc.clone()).is_ok();
    prop_assert_eq!(type_ok, schema_ok, "`{}` disagrees on {}", field, doc);
    Ok(())
}

fn g_base() -> Value {
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

fn j_base() -> Value {
    json!({
        "command": "c1",
        "classification": "update",
        "dependencies": {"readers": [], "holds": [], "pendingCommands": []},
        "cascade": [],
        "invalidates": [],
        "invariantsAfter": "ok",
        "decision": "auto",
        "reasons": []
    })
}

proptest! {
    #[test]
    fn data_capability_check_agrees_with_the_schema(s in arb_pattern_candidate()) {
        let mut doc = g_base();
        doc["dataCapability"] = json!(s);
        assert_same_verdict::<QueueEntry>(&g_validator(), &doc, "dataCapability")?;
    }

    #[test]
    fn partition_check_agrees_with_the_schema(s in arb_pattern_candidate()) {
        let mut doc = g_base();
        doc["partition"] = json!(s);
        let schema_ok = g_validator().is_valid(&doc);
        let type_ok = serde_json::from_value::<QueueEntry>(doc).is_ok();
        // The type follows the `.` of ECMA-262, which also excludes `\r`,
        // U+2028 and U+2029; the Rust validator's `.` only excludes `\n`.
        let ecma_only = s.contains(['\r', '\u{2028}', '\u{2029}']);
        prop_assert!(!type_ok || schema_ok, "type looser than the schema on {:?}", s);
        prop_assert!(type_ok || !schema_ok || ecma_only, "type refuses {:?} for no ECMA reason", s);
    }

    #[test]
    fn ttl_check_agrees_with_the_schema(s in arb_pattern_candidate()) {
        let mut doc = g_base();
        doc["parked"] = json!({"ttl": s, "onExpire": "drop"});
        assert_same_verdict::<QueueEntry>(&g_validator(), &doc, "parked.ttl")?;
    }

    #[test]
    fn reader_check_agrees_with_the_schema(s in arb_pattern_candidate()) {
        let mut doc = j_base();
        doc["dependencies"]["readers"] = json!([s]);
        assert_same_verdict::<ImpactPlan>(&j_validator(), &doc, "readers[]")?;
    }
}

/// Rule 32 of the milestone, restated apart from the implementation.
fn h_rule_accepts(
    command_id: &str,
    status: &str,
    data_version: &Value,
    violations: &[&str],
    review_id: &Value,
) -> bool {
    let version_at_least_one = data_version.as_u64().is_some_and(|v| v >= 1);
    !command_id.is_empty()
        && review_id.is_null()
        && violations.iter().all(|v| !v.is_empty())
        && match status {
            "applied" => version_at_least_one && violations.is_empty(),
            "rejected" => data_version.is_null() && !violations.is_empty(),
            "cancelled" | "expired" => data_version.is_null() && violations.is_empty(),
            _ => false,
        }
}

#[test]
fn h_accepts_exactly_the_combinations_of_rule_32_and_never_more_than_the_schema() {
    let schema = h_validator();
    let violation_sets: [&[&str]; 4] = [&[], &[""], &["v"], &["v", "v"]];
    let mut accepted = 0_u32;
    for command_id in ["", "c1"] {
        for status in ["applied", "rejected", "cancelled", "expired", "pending"] {
            for data_version in [json!(null), json!(0), json!(1), json!(7), json!(-1)] {
                for violations in violation_sets {
                    for review_id in [json!(null), json!("r1")] {
                        let doc = json!({
                            "commandId": command_id,
                            "status": status,
                            "dataVersion": data_version,
                            "violations": violations,
                            "reviewId": review_id,
                        });
                        let type_ok = serde_json::from_value::<CommandResult>(doc.clone()).is_ok();
                        let expected = h_rule_accepts(
                            command_id,
                            status,
                            &data_version,
                            violations,
                            &review_id,
                        );
                        assert_eq!(type_ok, expected, "rule 32 disagrees on {doc}");
                        assert!(
                            !type_ok || schema.is_valid(&doc),
                            "type looser than the schema on {doc}"
                        );
                        accepted += u32::from(type_ok);
                    }
                }
            }
        }
    }
    // applied: 2 versions; rejected: 2 sets; cancelled and expired: 1 set each.
    assert_eq!(accepted, 6, "the walk must cover every valid shape");
}

#[test]
fn j_refuses_blocked_and_rejected_without_a_reason_and_nothing_else() {
    let schema = j_validator();
    for decision in ["auto", "confirm", "human_review", "blocked", "rejected"] {
        for invariants_after in ["ok", "violated"] {
            for reasons in [json!([]), json!(["effet non supporté"])] {
                let mut doc = j_base();
                doc["decision"] = json!(decision);
                doc["invariantsAfter"] = json!(invariants_after);
                doc["reasons"] = reasons.clone();
                let type_ok = serde_json::from_value::<ImpactPlan>(doc.clone()).is_ok();
                let needs_reason = matches!(decision, "blocked" | "rejected");
                let expected = !needs_reason || reasons != json!([]);
                assert_eq!(type_ok, expected, "rule 46 disagrees on {doc}");
                assert!(schema.is_valid(&doc), "schema refuses {doc}");
            }
        }
    }
}

/// One top-level key of `doc` removed, or replaced by a value of another type.
fn arb_mutation(doc: Value) -> impl Strategy<Value = Value> {
    let keys: Vec<String> = doc
        .as_object()
        .map(|object| object.keys().cloned().collect())
        .unwrap_or_default();
    let replacement = prop_oneof![
        Just(None),
        Just(Some(json!(null))),
        Just(Some(json!(true))),
        Just(Some(json!(-1))),
        Just(Some(json!(1.5))),
        Just(Some(json!("x"))),
        Just(Some(json!([]))),
        Just(Some(json!({}))),
    ];
    (prop::sample::select(keys), replacement).prop_map(move |(key, replacement)| {
        let mut mutated = doc.clone();
        if let Some(object) = mutated.as_object_mut() {
            match replacement {
                None => {
                    object.remove(&key);
                }
                Some(value) => {
                    object.insert(key, value);
                }
            }
        }
        mutated
    })
}

/// The type never accepts a document the schema refuses.
fn never_looser<T: DeserializeOwned>(
    validator: &jsonschema::Validator,
    doc: &Value,
) -> Result<(), TestCaseError> {
    if serde_json::from_value::<T>(doc.clone()).is_ok() {
        prop_assert!(
            validator.is_valid(doc),
            "type accepts what the schema refuses: {}",
            doc
        );
    }
    Ok(())
}

proptest! {
    #[test]
    fn g_is_never_looser_than_its_schema(
        doc in arb_queue_entry().prop_flat_map(|e| arb_mutation(serde_json::to_value(e).unwrap_or_default()))
    ) {
        never_looser::<QueueEntry>(&g_validator(), &doc)?;
    }

    #[test]
    fn h_is_never_looser_than_its_schema(
        doc in arb_command_result().prop_flat_map(|r| arb_mutation(serde_json::to_value(r).unwrap_or_default()))
    ) {
        never_looser::<CommandResult>(&h_validator(), &doc)?;
    }

    #[test]
    fn j_is_never_looser_than_its_schema(
        doc in arb_impact_plan().prop_flat_map(|p| arb_mutation(serde_json::to_value(p).unwrap_or_default()))
    ) {
        never_looser::<ImpactPlan>(&j_validator(), &doc)?;
    }

    #[test]
    fn l_is_never_looser_than_its_schema(
        doc in arb_message().prop_flat_map(|m| arb_mutation(serde_json::to_value(m).unwrap_or_default()))
    ) {
        never_looser::<Message>(&l_validator(), &doc)?;
    }
}
