//! Rules 9-22: one test per CHECK, per key and per privilege of the queue.

use super::*;

const CHECK_VIOLATION: &str = "23514";
const UNIQUE_VIOLATION: &str = "23505";
const FOREIGN_KEY_VIOLATION: &str = "23503";
const INSUFFICIENT_PRIVILEGE: &str = "42501";

/// A store with the fixture capability registered.
async fn with_fixture(test: &str) -> Option<(TestSchema, Store)> {
    let (schema, store) = migrated(test).await?;
    insert_fixture_capability(&store).await;
    Some((schema, store))
}

/// Each `(overrides)` must fail with `code`.
async fn all_fail(store: &Store, code: &str, cases: &[&[(&str, &str)]]) {
    for overrides in cases {
        expect_sqlstate(store, &queue_insert(overrides), code).await;
    }
}

#[tokio::test]
async fn the_nine_states_are_accepted_and_no_other() {
    let Some((_schema, store)) = with_fixture("the_nine_states").await else {
        return;
    };
    let states = [
        "queued",
        "awaiting_confirmation",
        "confirmed",
        "awaiting_review",
        "parked",
        "applied",
        "rejected",
        "cancelled",
        "expired",
    ];
    for (index, state) in states.iter().enumerate() {
        let command = format!("'c{index}'");
        let position = index.to_string();
        let state_literal = format!("'{state}'");
        let mut overrides = vec![
            ("command", command.as_str()),
            ("position", position.as_str()),
            ("state", state_literal.as_str()),
        ];
        // `applied` and `rejected` carry the result H requires of them.
        match *state {
            "applied" => overrides.push(("data_version", "1")),
            "rejected" => overrides.push(("violations", r#"'["x"]'::jsonb"#)),
            _ => {}
        }
        exec(&store, &queue_insert(&overrides)).await;
    }
    assert_eq!(count(&store, "data_queue").await, 9);
    all_fail(
        &store,
        CHECK_VIOLATION,
        &[&[("command", "'h'"), ("position", "20"), ("state", "'held'")]],
    )
    .await;
}

#[tokio::test]
async fn the_default_state_is_queued() {
    let Some((_schema, store)) = with_fixture("the_default_state_is_queued").await else {
        return;
    };
    exec(&store, &queue_insert(&[])).await;
    let state: String = store
        .client()
        .query_one("SELECT state FROM data_queue", &[])
        .await
        .expect("state")
        .get(0);
    assert_eq!(state, "queued");
}

#[tokio::test]
async fn position_is_non_negative_and_unique_per_partition() {
    let Some((_schema, store)) = with_fixture("position_rules").await else {
        return;
    };
    exec(
        &store,
        &queue_insert(&[("command", "'a'"), ("position", "0")]),
    )
    .await;
    all_fail(
        &store,
        CHECK_VIOLATION,
        &[&[("command", "'b'"), ("position", "-1")]],
    )
    .await;
    all_fail(
        &store,
        UNIQUE_VIOLATION,
        &[&[("command", "'c'"), ("position", "0")]],
    )
    .await;
    // The same position in another partition is a different place.
    exec(
        &store,
        &queue_insert(&[
            ("command", "'d'"),
            ("partition", "'Probe/2'"),
            ("position", "0"),
        ]),
    )
    .await;
    // Gaps are allowed.
    exec(
        &store,
        &queue_insert(&[("command", "'e'"), ("position", "7")]),
    )
    .await;
    assert_eq!(count(&store, "data_queue").await, 3);
}

#[tokio::test]
async fn partition_shape_and_size() {
    let Some((_schema, store)) = with_fixture("partition_shape_and_size").await else {
        return;
    };
    for bad in [
        "'probe/1'",
        "'Probe/'",
        "'Probe'",
        "'1Probe/x'",
        "''",
        "'Pro-be/1'",
        // A line terminator in the id: PostgreSQL's `.` would accept it.
        "E'Probe/a\\nb'",
        "E'Probe/a\\rb'",
    ] {
        expect_sqlstate(
            &store,
            &queue_insert(&[("partition", bad)]),
            CHECK_VIOLATION,
        )
        .await;
    }
    // "Probe/" is 6 bytes: 506 more make exactly 512, 507 make 513.
    let at_limit = format!("'Probe/{}'", "x".repeat(506));
    let over_limit = format!("'Probe/{}'", "x".repeat(507));
    exec(&store, &queue_insert(&[("partition", &at_limit)])).await;
    expect_sqlstate(
        &store,
        &queue_insert(&[("command", "'z'"), ("partition", &over_limit)]),
        CHECK_VIOLATION,
    )
    .await;
}

#[tokio::test]
async fn data_capability_must_be_well_formed_and_registered() {
    let Some((_schema, store)) = with_fixture("data_capability_rules").await else {
        return;
    };
    for unregistered in ["'fixture.createProbe@2'", "'fixture.createProbe@0'"] {
        expect_sqlstate(
            &store,
            &queue_insert(&[("data_capability", unregistered)]),
            FOREIGN_KEY_VIOLATION,
        )
        .await;
    }
    for malformed in ["'Fixture.x@1'", "'fixture.x'", "'fixture@1'", "''"] {
        expect_sqlstate(
            &store,
            &queue_insert(&[("data_capability", malformed)]),
            CHECK_VIOLATION,
        )
        .await;
    }
}

#[tokio::test]
async fn json_columns_keep_their_shape() {
    let Some((_schema, store)) = with_fixture("json_columns_keep_their_shape").await else {
        return;
    };
    let based_on =
        |value: &'static str| -> Vec<(&'static str, &'static str)> { vec![("based_on", value)] };
    let cases: Vec<Vec<(&str, &str)>> = vec![
        // based_on: version required, a non-negative integer, values an object.
        based_on("'{}'::jsonb"),
        based_on(r#"'{"version":-1}'::jsonb"#),
        based_on(r#"'{"version":1.5}'::jsonb"#),
        based_on(r#"'{"version":"1"}'::jsonb"#),
        based_on(r#"'{"version":null}'::jsonb"#),
        based_on(r#"'{"version":0,"values":[]}'::jsonb"#),
        based_on(r#"'{"version":0,"extra":1}'::jsonb"#),
        based_on("'[]'::jsonb"),
        // projection: both members, nothing else.
        vec![("projection", r#"'{"confirmed":{}}'::jsonb"#)],
        vec![("projection", r#"'{"pendingAhead":[]}'::jsonb"#)],
        vec![(
            "projection",
            r#"'{"confirmed":{},"pendingAhead":{}}'::jsonb"#,
        )],
        vec![(
            "projection",
            r#"'{"confirmed":{},"pendingAhead":[],"x":1}'::jsonb"#,
        )],
        // parked: ttl and onExpire = drop.
        vec![("parked", r#"'{"ttl":"5x","onExpire":"drop"}'::jsonb"#)],
        vec![("parked", r#"'{"ttl":"5m","onExpire":"keep"}'::jsonb"#)],
        vec![("parked", r#"'{"ttl":"5m"}'::jsonb"#)],
        vec![("parked", r#"'{"onExpire":"drop"}'::jsonb"#)],
        // confirmation: an object with `by`.
        vec![("confirmation", "'{}'::jsonb")],
        vec![("confirmation", r#"'{"by":1}'::jsonb"#)],
        // by, payload, command, idempotency key.
        vec![("by", "''")],
        vec![("payload", "'[]'::jsonb")],
        vec![("command", "''")],
        vec![("idempotency_key", "''")],
        vec![("violations", "'{}'::jsonb")],
        vec![("data_version", "0")],
        // A requeue never points at itself.
        vec![("requeued_from", "'c1'")],
        // based_on.version must fit a bigint.
        based_on(r#"'{"version":99999999999999999999}'::jsonb"#),
    ];
    for overrides in &cases {
        expect_sqlstate(&store, &queue_insert(overrides), CHECK_VIOLATION).await;
    }
    // The valid counterparts are accepted.
    let valid = [
        (
            "'a'",
            "0",
            vec![("based_on", r#"'{"version":3,"values":{"k":1}}'::jsonb"#)],
        ),
        (
            "'b'",
            "1",
            vec![(
                "projection",
                r#"'{"confirmed":{},"pendingAhead":[]}'::jsonb"#,
            )],
        ),
        (
            "'c'",
            "2",
            vec![("parked", r#"'{"ttl":"24h","onExpire":"drop"}'::jsonb"#)],
        ),
        (
            "'d'",
            "3",
            vec![("confirmation", r#"'{"by":"gm"}'::jsonb"#)],
        ),
        ("'e'", "4", vec![("your_value", "'42'::jsonb")]),
    ];
    for (command, position, mut overrides) in valid {
        overrides.push(("command", command));
        overrides.push(("position", position));
        exec(&store, &queue_insert(&overrides)).await;
    }
}

#[tokio::test]
async fn requeued_from_references_an_existing_command() {
    let Some((_schema, store)) = with_fixture("requeued_from_references").await else {
        return;
    };
    all_fail(
        &store,
        FOREIGN_KEY_VIOLATION,
        &[&[("requeued_from", "'nobody'")]],
    )
    .await;
    exec(&store, &queue_insert(&[("command", "'first'")])).await;
    exec(
        &store,
        &queue_insert(&[
            ("command", "'second'"),
            ("position", "1"),
            ("requeued_from", "'first'"),
        ]),
    )
    .await;
}

#[tokio::test]
async fn the_result_columns_agree_with_the_state() {
    let Some((_schema, store)) = with_fixture("result_columns_agree").await else {
        return;
    };
    all_fail(
        &store,
        CHECK_VIOLATION,
        &[
            &[("state", "'applied'")],
            &[
                ("state", "'applied'"),
                ("data_version", "1"),
                ("violations", r#"'["x"]'::jsonb"#),
            ],
            &[
                ("state", "'rejected'"),
                ("data_version", "1"),
                ("violations", r#"'["x"]'::jsonb"#),
            ],
            &[("state", "'rejected'")],
            &[("state", "'rejected'"), ("violations", "'[]'::jsonb")],
            &[("state", "'cancelled'"), ("data_version", "1")],
            &[("state", "'queued'"), ("data_version", "1")],
            // Only an applied command has a dataVersion.
            &[("state", "'expired'"), ("data_version", "1")],
            &[("state", "'parked'"), ("data_version", "1")],
            &[("state", "'awaiting_review'"), ("data_version", "1")],
        ],
    )
    .await;
    // The accepted forms.
    exec(
        &store,
        &queue_insert(&[
            ("command", "'a'"),
            ("position", "0"),
            ("state", "'applied'"),
            ("data_version", "1"),
        ]),
    )
    .await;
    exec(
        &store,
        &queue_insert(&[
            ("command", "'b'"),
            ("position", "1"),
            ("state", "'applied'"),
            ("data_version", "2"),
            ("violations", "'[]'::jsonb"),
        ]),
    )
    .await;
    exec(
        &store,
        &queue_insert(&[
            ("command", "'c'"),
            ("position", "2"),
            ("state", "'rejected'"),
            ("violations", r#"'["x"]'::jsonb"#),
        ]),
    )
    .await;
    // One applied dataVersion is never given to two commands.
    all_fail(
        &store,
        UNIQUE_VIOLATION,
        &[&[
            ("command", "'d'"),
            ("position", "3"),
            ("state", "'applied'"),
            ("data_version", "1"),
        ]],
    )
    .await;
}

#[tokio::test]
async fn an_idempotency_key_is_unique_per_capability() {
    let Some((_schema, store)) = with_fixture("idempotency_key_unique").await else {
        return;
    };
    exec(
        &store,
        &queue_insert(&[("command", "'a'"), ("idempotency_key", "'k'")]),
    )
    .await;
    all_fail(
        &store,
        UNIQUE_VIOLATION,
        &[&[
            ("command", "'b'"),
            ("position", "1"),
            ("idempotency_key", "'k'"),
        ]],
    )
    .await;
    // Null keys never collide with each other.
    exec(
        &store,
        &queue_insert(&[("command", "'c'"), ("position", "2")]),
    )
    .await;
    exec(
        &store,
        &queue_insert(&[("command", "'d'"), ("position", "3")]),
    )
    .await;
}

#[tokio::test]
async fn a_command_keeps_its_place_and_is_never_deleted() {
    let Some((_schema, store)) = with_fixture("command_keeps_its_place").await else {
        return;
    };
    exec(&store, &queue_insert(&[])).await;
    for column in [
        "command",
        "partition",
        "position",
        "data_capability",
        "by",
        "payload",
        "idempotency_key",
        "created_at",
    ] {
        expect_sqlstate(
            &store,
            &format!("UPDATE data_queue SET {column} = {column}"),
            INSUFFICIENT_PRIVILEGE,
        )
        .await;
    }
    expect_sqlstate(&store, "DELETE FROM data_queue", INSUFFICIENT_PRIVILEGE).await;
    expect_sqlstate(&store, "TRUNCATE data_queue", INSUFFICIENT_PRIVILEGE).await;
    assert_eq!(count(&store, "data_queue").await, 1);
    // What may change does.
    exec(&store, "UPDATE data_queue SET state = 'cancelled'").await;
    let state: String = store
        .client()
        .query_one("SELECT state FROM data_queue", &[])
        .await
        .expect("state")
        .get(0);
    assert_eq!(state, "cancelled");
}
