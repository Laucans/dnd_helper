//! Schema loader and proptest strategies shared by the integration tests.
//!
//! Every strategy yields valid values only, built through the public API.

#![allow(dead_code, reason = "each test crate uses a different subset")]

use std::num::NonZeroU64;

use dataguard_contract_types::{
    Actor, BasedOn, CascadeItem, CascadePolicy, Classification, CommandApplied, CommandId,
    CommandQueued, CommandRejected, CommandResult, Confirmation, DataCapabilityRef, Decision,
    Dependencies, Hold, ImpactPlan, ImpactPlanFields, InvariantAtRisk, InvariantsAfter, Message,
    OnExpire, Parked, Partition, PendingAhead, Projection, QueueEntry, QueueState, ReaderRef,
    Recipients, Ttl, Violation, Violations,
};
use proptest::collection::{hash_map, vec};
use proptest::prelude::*;
use serde_json::{Map, Value, json};

/// Builds a draft 2020-12 validator from the real file in `contracts/`.
///
/// A missing, unreadable or invalid schema fails the test: never skipped.
/// Nothing is fetched: the schemas carry no `$ref`.
pub fn validator(file: &str) -> jsonschema::Validator {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../contracts/");
    let path = format!("{path}{file}");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let schema: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {path}: {e}"));
    jsonschema::draft202012::new(&schema).unwrap_or_else(|e| panic!("compile {path}: {e}"))
}

/// Fails with every schema error when `doc` is not valid.
pub fn assert_valid(validator: &jsonschema::Validator, doc: &Value) {
    let errors: Vec<String> = validator.iter_errors(doc).map(|e| e.to_string()).collect();
    assert!(errors.is_empty(), "{doc}\n{}", errors.join("\n"));
}

pub fn g_validator() -> jsonschema::Validator {
    validator("g-queue-entry.schema.json")
}

pub fn h_validator() -> jsonschema::Validator {
    validator("h-command-result.schema.json")
}

pub fn j_validator() -> jsonschema::Validator {
    validator("j-impact-plan.schema.json")
}

pub fn l_validator() -> jsonschema::Validator {
    validator("l-message.schema.json")
}

/// Any JSON value. No non-integral float: `f64` does not round-trip exactly.
pub fn arb_json_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(|n| json!(n)),
        "[a-zA-Z0-9 ]{0,8}".prop_map(Value::String),
    ];
    leaf.prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            vec(inner.clone(), 0..4).prop_map(Value::Array),
            arb_map(inner).prop_map(Value::Object),
        ]
    })
}

fn arb_map(values: impl Strategy<Value = Value>) -> impl Strategy<Value = Map<String, Value>> {
    hash_map("[a-z]{1,5}", values, 0..4).prop_map(|m| m.into_iter().collect())
}

/// Opaque, readable, non-empty text, including non-ASCII.
fn arb_text() -> impl Strategy<Value = String> {
    "\\PC{1,12}"
}

fn arb_command_id() -> impl Strategy<Value = CommandId> {
    arb_text().prop_map(|s| CommandId::new(s).unwrap())
}

fn arb_violations() -> impl Strategy<Value = Violations> {
    vec(arb_text().prop_map(|s| Violation::new(s).unwrap()), 1..4)
        .prop_map(|v| Violations::new(v).unwrap())
}

fn arb_recipients() -> impl Strategy<Value = Recipients> {
    vec(".{0,8}", 1..4).prop_map(|v| Recipients::new(v).unwrap())
}

fn arb_data_capability() -> impl Strategy<Value = DataCapabilityRef> {
    prop_oneof![
        Just("credit.requestLimitChange@2".to_string()),
        "[a-z][a-z0-9-]{0,6}\\.[A-Za-z][A-Za-z0-9]{0,6}@[0-9]{1,4}",
    ]
    .prop_map(|s| DataCapabilityRef::new(s).unwrap())
}

fn arb_reader() -> impl Strategy<Value = ReaderRef> {
    "[a-z][a-z0-9-]{0,6}\\.[A-Za-z][A-Za-z0-9]{0,6}".prop_map(|s| ReaderRef::new(s).unwrap())
}

fn arb_partition() -> impl Strategy<Value = Partition> {
    prop_oneof![
        Just("Account/881".to_string()),
        "[A-Z][A-Za-z0-9]{0,6}/[^\n\r\u{2028}\u{2029}]{1,12}",
    ]
    .prop_map(|s| Partition::new(s).unwrap())
}

fn arb_ttl() -> impl Strategy<Value = Ttl> {
    prop_oneof![Just("0s".to_string()), "[0-9]{1,4}(ms|s|m|h|d)",]
        .prop_map(|s| Ttl::new(s).unwrap())
}

fn arb_state() -> impl Strategy<Value = QueueState> {
    prop::sample::select(vec![
        QueueState::Queued,
        QueueState::AwaitingConfirmation,
        QueueState::Confirmed,
        QueueState::AwaitingReview,
        QueueState::Parked,
        QueueState::Applied,
        QueueState::Rejected,
        QueueState::Cancelled,
        QueueState::Expired,
    ])
}

fn arb_pending_ahead() -> impl Strategy<Value = PendingAhead> {
    (
        arb_text(),
        arb_text(),
        prop::option::of(arb_json_value()),
        arb_text(),
    )
        .prop_map(|(command, by, value, state)| PendingAhead {
            command,
            by,
            value,
            state,
        })
}

fn arb_projection() -> impl Strategy<Value = Projection> {
    (arb_map(arb_json_value()), vec(arb_pending_ahead(), 0..3)).prop_map(
        |(confirmed, pending_ahead)| Projection {
            confirmed,
            pending_ahead,
        },
    )
}

fn arb_based_on() -> impl Strategy<Value = BasedOn> {
    (
        prop_oneof![Just(0u64), any::<u64>()],
        prop::option::of(arb_map(arb_json_value())),
    )
        .prop_map(|(version, values)| BasedOn { version, values })
}

pub fn arb_queue_entry() -> impl Strategy<Value = QueueEntry> {
    let head = (
        arb_command_id(),
        arb_data_capability(),
        arb_text().prop_map(|s| Actor::new(s).unwrap()),
        arb_partition(),
        prop_oneof![Just(0u64), any::<u64>()],
        arb_based_on(),
    );
    let tail = (
        prop::option::of(arb_projection()),
        // `Some(Null)` is generated: a present null must survive.
        prop::option::of(arb_json_value()),
        arb_state(),
        prop::option::of(arb_text().prop_map(|by| Confirmation { by })),
        prop::option::of(arb_ttl().prop_map(|ttl| Parked {
            ttl,
            on_expire: OnExpire::Drop,
        })),
        prop::option::of(arb_text()),
    );
    (head, tail).prop_map(
        |(
            (command, data_capability, by, partition, position, based_on),
            (projection, your_value, state, confirmation, parked, requeued_from),
        )| QueueEntry {
            command,
            data_capability,
            by,
            partition,
            position,
            based_on,
            projection,
            your_value,
            state,
            confirmation,
            parked,
            requeued_from,
        },
    )
}

pub fn arb_command_result() -> impl Strategy<Value = CommandResult> {
    prop_oneof![
        (
            arb_command_id(),
            prop_oneof![Just(1u64), 1..=u64::MAX].prop_map(|n| NonZeroU64::new(n).unwrap())
        )
            .prop_map(|(id, version)| CommandResult::applied(id, version)),
        (arb_command_id(), arb_violations())
            .prop_map(|(id, violations)| CommandResult::rejected(id, violations)),
        arb_command_id().prop_map(CommandResult::cancelled),
        arb_command_id().prop_map(CommandResult::expired),
    ]
}

fn arb_dependencies() -> impl Strategy<Value = Dependencies> {
    (
        vec(arb_reader(), 0..3),
        vec(
            (arb_text(), arb_text(), arb_text()).prop_map(|(id, by, expires)| Hold {
                id,
                by,
                expires,
            }),
            0..3,
        ),
        vec(arb_text(), 0..3),
    )
        .prop_map(|(readers, holds, pending_commands)| Dependencies {
            readers,
            holds,
            pending_commands,
        })
}

fn arb_cascade_item() -> impl Strategy<Value = CascadeItem> {
    (
        arb_text(),
        prop_oneof![Just(0u64), any::<u64>()],
        prop::sample::select(vec![
            CascadePolicy::Restrict,
            CascadePolicy::Cascade,
            CascadePolicy::Nullify,
            CascadePolicy::Archive,
            CascadePolicy::Review,
        ]),
        prop::option::of(any::<bool>()),
        prop::option::of(any::<bool>()),
    )
        .prop_map(|(relation, rows, policy, blocking, semantic)| CascadeItem {
            relation,
            rows,
            policy,
            blocking,
            semantic,
        })
}

pub fn arb_impact_plan() -> impl Strategy<Value = ImpactPlan> {
    let head = (
        arb_text(),
        prop::sample::select(vec![
            Classification::Insert,
            Classification::Update,
            Classification::Delete,
            Classification::Upsert,
        ]),
        arb_dependencies(),
        vec(arb_cascade_item(), 0..3),
        vec(arb_text(), 0..3),
    );
    let tail = (
        prop::sample::select(vec![InvariantsAfter::Ok, InvariantsAfter::Violated]),
        prop::sample::select(vec![
            Decision::Auto,
            Decision::Confirm,
            Decision::HumanReview,
            Decision::Blocked,
            Decision::Rejected,
        ]),
        vec(arb_text(), 0..3),
    );
    (head, tail).prop_map(
        |(
            (command, classification, dependencies, cascade, invalidates),
            (invariants_after, decision, mut reasons),
        )| {
            // A blocked or rejected plan must say why.
            if matches!(decision, Decision::Blocked | Decision::Rejected) && reasons.is_empty() {
                reasons.push("effet non supporté".to_string());
            }
            ImpactPlan::new(ImpactPlanFields {
                command,
                classification,
                dependencies,
                cascade,
                invalidates,
                invariants_after,
                decision,
                reasons,
            })
            .unwrap()
        },
    )
}

pub fn arb_message() -> impl Strategy<Value = Message> {
    prop_oneof![
        (arb_recipients(), arb_command_id())
            .prop_map(|(to, command)| Message::CommandQueued(CommandQueued { to, command })),
        (
            arb_recipients(),
            arb_command_id(),
            prop_oneof![Just(1u64), 1..=u64::MAX].prop_map(|n| NonZeroU64::new(n).unwrap())
        )
            .prop_map(|(to, command, data_version)| Message::CommandApplied(
                CommandApplied {
                    to,
                    command,
                    data_version
                }
            )),
        (
            arb_recipients(),
            arb_command_id(),
            arb_violations(),
            prop::option::of(arb_text())
        )
            .prop_map(
                |(to, command, violations, reason)| Message::CommandRejected(CommandRejected {
                    to,
                    command,
                    violations,
                    reason
                })
            ),
        (
            arb_recipients(),
            arb_command_id(),
            arb_violations(),
            prop::option::of(arb_text())
        )
            .prop_map(
                |(to, command, violations, reason)| Message::InvariantAtRisk(InvariantAtRisk {
                    to,
                    command,
                    violations,
                    reason
                })
            ),
    ]
}
