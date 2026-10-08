//! Fixed corpora of invalid documents.
//!
//! "Both refuse": the schema validator and the type reject the document, so
//! the type is never looser than the schema on a field it models.
//! "Type only": the schema accepts it, the type is deliberately stricter.
//! Plus a no-panic sweep over arbitrary JSON.

mod common;

use common::{arb_json_value, g_validator, h_validator, j_validator, l_validator};
use dataguard_contract_types::{CommandResult, ImpactPlan, Message, QueueEntry};
use proptest::prelude::*;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

/// A copy of `base` after `edit`.
fn with(base: &Value, edit: impl FnOnce(&mut Value)) -> Value {
    let mut doc = base.clone();
    edit(&mut doc);
    doc
}

fn without(base: &Value, pointer: &str) -> Value {
    with(base, |doc| {
        let (parent, key) = pointer.rsplit_once('/').unwrap();
        doc.pointer_mut(parent)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove(key);
    })
}

fn set(base: &Value, pointer: &str, value: Value) -> Value {
    with(base, |doc| *doc.pointer_mut(pointer).unwrap() = value)
}

fn add(base: &Value, pointer: &str, key: &str, value: Value) -> Value {
    with(base, |doc| {
        doc.pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert(key.to_string(), value);
    })
}

fn both_refuse<T: DeserializeOwned>(validator: &jsonschema::Validator, corpus: Vec<(&str, Value)>) {
    for (label, doc) in corpus {
        assert!(!validator.is_valid(&doc), "schema accepts `{label}`: {doc}");
        assert!(
            serde_json::from_value::<T>(doc.clone()).is_err(),
            "type accepts `{label}`: {doc}"
        );
    }
}

fn type_only<T: DeserializeOwned>(validator: &jsonschema::Validator, corpus: Vec<(&str, Value)>) {
    for (label, doc) in corpus {
        assert!(
            validator.is_valid(&doc),
            "`{label}` is not schema-valid, move it to the both-refuse list: {doc}"
        );
        assert!(
            serde_json::from_value::<T>(doc.clone()).is_err(),
            "type accepts `{label}`: {doc}"
        );
    }
}

fn g() -> Value {
    json!({
        "command": "c1",
        "dataCapability": "credit.requestLimitChange@2",
        "by": "gm",
        "partition": "Account/881",
        "position": 0,
        "basedOn": {"version": 0},
        "projection": {
            "confirmed": {},
            "pendingAhead": [{"command": "c0", "by": "gm", "state": "queued"}]
        },
        "state": "queued",
        "confirmation": {"by": "gm"},
        "parked": {"ttl": "24h", "onExpire": "drop"}
    })
}

fn h() -> Value {
    json!({
        "commandId": "c1", "status": "rejected", "dataVersion": null,
        "violations": ["v1"], "reviewId": null
    })
}

fn j() -> Value {
    json!({
        "command": "c1",
        "classification": "update",
        "dependencies": {
            "readers": ["credit.requestLimitChange"],
            "holds": [{"id": "h", "by": "gm", "expires": "soon"}],
            "pendingCommands": []
        },
        "cascade": [{"relation": "r", "rows": 1, "policy": "restrict"}],
        "invalidates": [],
        "invariantsAfter": "ok",
        "decision": "auto",
        "reasons": []
    })
}

fn l(name: &str) -> Value {
    json!({
        "message": name, "to": ["gm"], "command": "c1",
        "dataVersion": 1, "violations": ["v1"]
    })
}

#[test]
fn the_valid_bases_are_valid() {
    assert!(g_validator().is_valid(&g()));
    assert!(h_validator().is_valid(&h()));
    assert!(j_validator().is_valid(&j()));
    for name in [
        "CommandQueued",
        "CommandApplied",
        "CommandRejected",
        "InvariantAtRisk",
    ] {
        assert!(l_validator().is_valid(&l(name)));
        assert!(serde_json::from_value::<Message>(l(name)).is_ok(), "{name}");
    }
    assert!(serde_json::from_value::<QueueEntry>(g()).is_ok());
    assert!(serde_json::from_value::<CommandResult>(h()).is_ok());
    assert!(serde_json::from_value::<ImpactPlan>(j()).is_ok());
}

#[test]
fn g_both_refuse() {
    let g = g();
    let mut corpus = vec![
        (
            "partition without slash",
            set(&g, "/partition", json!("Account")),
        ),
        (
            "partition empty id",
            set(&g, "/partition", json!("Account/")),
        ),
        (
            "partition lowercase aggregate",
            set(&g, "/partition", json!("account/1")),
        ),
        ("partition wrong type", set(&g, "/partition", json!(1))),
        ("negative position", set(&g, "/position", json!(-1))),
        ("fractional position", set(&g, "/position", json!(1.5))),
        ("string position", set(&g, "/position", json!("1"))),
        ("ttl 24x", set(&g, "/parked/ttl", json!("24x"))),
        ("ttl without unit", set(&g, "/parked/ttl", json!("24"))),
        ("onExpire other", set(&g, "/parked/onExpire", json!("keep"))),
        ("parked without ttl", without(&g, "/parked/ttl")),
        ("parked without onExpire", without(&g, "/parked/onExpire")),
        (
            "capability without version",
            set(&g, "/dataCapability", json!("credit.x")),
        ),
        (
            "capability uppercase system",
            set(&g, "/dataCapability", json!("Credit.x@1")),
        ),
        (
            "capability text version",
            set(&g, "/dataCapability", json!("credit.x@v2")),
        ),
        ("unknown state", set(&g, "/state", json!("bogus"))),
        ("unknown top-level field", add(&g, "", "extra", json!(1))),
        (
            "unknown basedOn field",
            add(&g, "/basedOn", "extra", json!(1)),
        ),
        (
            "unknown projection field",
            add(&g, "/projection", "extra", json!(1)),
        ),
        ("projection null", set(&g, "/projection", Value::Null)),
        (
            "projection without pendingAhead",
            without(&g, "/projection/pendingAhead"),
        ),
        (
            "projection without confirmed",
            without(&g, "/projection/confirmed"),
        ),
        (
            "pendingAhead item without state",
            without(&g, "/projection/pendingAhead/0/state"),
        ),
        (
            "pendingAhead item without by",
            without(&g, "/projection/pendingAhead/0/by"),
        ),
        (
            "basedOn.version negative",
            set(&g, "/basedOn/version", json!(-1)),
        ),
        (
            "basedOn.values string",
            add(&g, "/basedOn", "values", json!("x")),
        ),
        (
            "basedOn.values null",
            add(&g, "/basedOn", "values", Value::Null),
        ),
        (
            "confirmation without by",
            set(&g, "/confirmation", json!({})),
        ),
        ("command wrong type", set(&g, "/command", json!(5))),
        (
            "requeuedFrom wrong type",
            add(&g, "", "requeuedFrom", json!(5)),
        ),
    ];
    for field in [
        "command",
        "dataCapability",
        "by",
        "partition",
        "position",
        "basedOn",
        "state",
    ] {
        corpus.push((field, without(&g, &format!("/{field}"))));
    }
    both_refuse::<QueueEntry>(&g_validator(), corpus);
}

#[test]
fn g_type_only() {
    let g = g();
    type_only::<QueueEntry>(
        &g_validator(),
        vec![
            ("empty command", set(&g, "/command", json!(""))),
            ("empty by", set(&g, "/by", json!(""))),
        ],
    );
}

#[test]
fn h_both_refuse() {
    let h = h();
    let mut corpus = vec![
        ("unknown status", set(&h, "/status", json!("bogus"))),
        ("negative dataVersion", set(&h, "/dataVersion", json!(-1))),
        ("string dataVersion", set(&h, "/dataVersion", json!("1"))),
        (
            "fractional dataVersion",
            set(&h, "/dataVersion", json!(1.5)),
        ),
        ("violations not a list", set(&h, "/violations", json!("v1"))),
        ("violation not a string", set(&h, "/violations", json!([1]))),
        ("commandId wrong type", set(&h, "/commandId", json!(5))),
        ("extra field", add(&h, "", "extra", json!(1))),
    ];
    for field in [
        "commandId",
        "status",
        "dataVersion",
        "violations",
        "reviewId",
    ] {
        corpus.push((field, without(&h, &format!("/{field}"))));
    }
    both_refuse::<CommandResult>(&h_validator(), corpus);
}

#[test]
fn h_type_only() {
    let applied = json!({
        "commandId": "c1", "status": "applied", "dataVersion": 1,
        "violations": [], "reviewId": null
    });
    let cancelled = json!({
        "commandId": "c1", "status": "cancelled", "dataVersion": null,
        "violations": [], "reviewId": null
    });
    type_only::<CommandResult>(
        &h_validator(),
        vec![
            (
                "applied, version 0",
                set(&applied, "/dataVersion", json!(0)),
            ),
            (
                "applied, no version",
                set(&applied, "/dataVersion", Value::Null),
            ),
            (
                "applied, violations",
                set(&applied, "/violations", json!(["v1"])),
            ),
            ("applied, reviewId", set(&applied, "/reviewId", json!("r1"))),
            (
                "rejected, no violations",
                set(&h(), "/violations", json!([])),
            ),
            ("rejected, version", set(&h(), "/dataVersion", json!(1))),
            ("rejected, reviewId", set(&h(), "/reviewId", json!("r1"))),
            (
                "rejected, empty violation",
                set(&h(), "/violations", json!([""])),
            ),
            (
                "cancelled, violations",
                set(&cancelled, "/violations", json!(["v1"])),
            ),
            (
                "cancelled, version",
                set(&cancelled, "/dataVersion", json!(1)),
            ),
            (
                "cancelled, reviewId",
                set(&cancelled, "/reviewId", json!("r1")),
            ),
            (
                "expired, violations",
                set(
                    &set(&cancelled, "/status", json!("expired")),
                    "/violations",
                    json!(["v1"]),
                ),
            ),
            (
                "expired, reviewId",
                set(
                    &set(&cancelled, "/status", json!("expired")),
                    "/reviewId",
                    json!("r1"),
                ),
            ),
            ("empty commandId", set(&cancelled, "/commandId", json!(""))),
        ],
    );
}

#[test]
fn j_both_refuse() {
    let j = j();
    let mut corpus = vec![
        (
            "reader with version",
            set(&j, "/dependencies/readers", json!(["a.b@2"])),
        ),
        (
            "reader uppercase system",
            set(&j, "/dependencies/readers", json!(["A.b"])),
        ),
        (
            "unknown classification",
            set(&j, "/classification", json!("merge")),
        ),
        (
            "unknown policy",
            set(&j, "/cascade/0/policy", json!("drop")),
        ),
        ("negative rows", set(&j, "/cascade/0/rows", json!(-1))),
        (
            "blocking null",
            add(&j, "/cascade/0", "blocking", Value::Null),
        ),
        (
            "semantic string",
            add(&j, "/cascade/0", "semantic", json!("yes")),
        ),
        (
            "unknown decision",
            set(&j, "/decision", json!("humanReview")),
        ),
        (
            "unknown invariantsAfter",
            set(&j, "/invariantsAfter", json!("maybe")),
        ),
        ("unknown top-level field", add(&j, "", "extra", json!(1))),
        (
            "unknown dependencies field",
            add(&j, "/dependencies", "extra", json!(1)),
        ),
        (
            "unknown cascade field",
            add(&j, "/cascade/0", "extra", json!(1)),
        ),
        (
            "hold without expires",
            without(&j, "/dependencies/holds/0/expires"),
        ),
        ("cascade without policy", without(&j, "/cascade/0/policy")),
        (
            "dependencies without holds",
            without(&j, "/dependencies/holds"),
        ),
        ("reasons not a list", set(&j, "/reasons", json!("x"))),
    ];
    for field in [
        "command",
        "classification",
        "dependencies",
        "cascade",
        "invalidates",
        "invariantsAfter",
        "decision",
        "reasons",
    ] {
        corpus.push((field, without(&j, &format!("/{field}"))));
    }
    both_refuse::<ImpactPlan>(&j_validator(), corpus);
}

#[test]
fn j_type_only() {
    let j = j();
    type_only::<ImpactPlan>(
        &j_validator(),
        vec![
            ("blocked, no reason", set(&j, "/decision", json!("blocked"))),
            (
                "rejected, no reason",
                set(&j, "/decision", json!("rejected")),
            ),
        ],
    );
}

#[test]
fn j_blocked_with_the_skeleton_reason_is_valid() {
    let doc = with(&j(), |d| {
        d["decision"] = json!("blocked");
        d["reasons"] = json!(["effet non supporté"]);
    });
    assert!(j_validator().is_valid(&doc));
    assert!(serde_json::from_value::<ImpactPlan>(doc).is_ok());
}

#[test]
fn l_both_refuse() {
    let queued = l("CommandQueued");
    let applied = l("CommandApplied");
    let corpus = vec![
        ("unknown name", set(&queued, "/message", json!("Bogus"))),
        ("empty to", set(&queued, "/to", json!([]))),
        ("to wrong type", set(&queued, "/to", json!("gm"))),
        ("to entry wrong type", set(&queued, "/to", json!([1]))),
        ("missing to", without(&queued, "/to")),
        ("missing message", without(&queued, "/message")),
        (
            "negative dataVersion",
            set(&applied, "/dataVersion", json!(-1)),
        ),
        ("command wrong type", set(&queued, "/command", json!(5))),
    ];
    both_refuse::<Message>(&l_validator(), corpus);
}

#[test]
fn l_type_only() {
    let queued = l("CommandQueued");
    let applied = l("CommandApplied");
    let rejected = l("CommandRejected");
    let at_risk = l("InvariantAtRisk");
    let mut corpus = vec![
        (
            "applied, version 0",
            set(&applied, "/dataVersion", json!(0)),
        ),
        ("applied, no version", without(&applied, "/dataVersion")),
        ("queued, no command", without(&queued, "/command")),
        ("applied, no command", without(&applied, "/command")),
        ("rejected, no violations", without(&rejected, "/violations")),
        (
            "rejected, empty violations",
            set(&rejected, "/violations", json!([])),
        ),
        ("at risk, no violations", without(&at_risk, "/violations")),
        (
            "at risk, empty violations",
            set(&at_risk, "/violations", json!([])),
        ),
        ("empty command", set(&queued, "/command", json!(""))),
    ];
    for name in [
        "ValueDeclaredAhead",
        "BlockedByHold",
        "HoldExpiring",
        "ReviewRequired",
        "Parked",
        "ParkedExpiring",
        "Expired",
        "AheadResolved",
    ] {
        corpus.push((name, set(&queued, "/message", json!(name))));
    }
    type_only::<Message>(&l_validator(), corpus);
}

#[test]
fn malformed_json_is_a_typed_error_on_all_four_types() {
    for text in ["", "{", "[]", "null", "42", "\"x\"", "{\"a\":}", "{}"] {
        assert!(serde_json::from_str::<QueueEntry>(text).is_err(), "{text}");
        assert!(
            serde_json::from_str::<CommandResult>(text).is_err(),
            "{text}"
        );
        assert!(serde_json::from_str::<ImpactPlan>(text).is_err(), "{text}");
        assert!(serde_json::from_str::<Message>(text).is_err(), "{text}");
    }
}

proptest! {
    /// Any JSON gives `Ok` or `Err`, never a panic (a panic fails the case).
    #[test]
    fn arbitrary_json_never_panics(value in arb_json_value()) {
        let _ = serde_json::from_value::<QueueEntry>(value.clone());
        let _ = serde_json::from_value::<CommandResult>(value.clone());
        let _ = serde_json::from_value::<ImpactPlan>(value.clone());
        let _ = serde_json::from_value::<Message>(value);
    }
}
