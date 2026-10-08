//! Rules 26-30: one row per event, append-only for every role.

use super::*;

#[tokio::test]
async fn events_have_the_shape_the_rules_give_them() {
    let Some((_schema, store)) = migrated("audit_event_shapes").await else {
        return;
    };
    insert_fixture_capability(&store).await;
    exec(&store, &queue_insert(&[])).await;
    exec(&store, &audit_insert(&[])).await;
    // A pre-enqueue refusal has no queue row and may name an unknown capability.
    exec(
        &store,
        &audit_insert(&[
            ("event", "'refused'"),
            ("command", "NULL"),
            ("data_capability", "'nobody.knows@9'"),
        ]),
    )
    .await;
    exec(
        &store,
        &audit_insert(&[("event", "'applied'"), ("data_version", "1")]),
    )
    .await;
    for event in ["rejected", "cancelled"] {
        exec(&store, &audit_insert(&[("event", &format!("'{event}'"))])).await;
    }
    assert_eq!(count(&store, "audit_log").await, 5);

    for overrides in [
        // Every event but `refused` names a command.
        vec![("command", "NULL")],
        // dataVersion exists if and only if the event is `applied`, and is >= 1.
        vec![("event", "'applied'")],
        vec![("event", "'applied'"), ("data_version", "0")],
        vec![("event", "'rejected'"), ("data_version", "1")],
        vec![("data_version", "1")],
        vec![("event", "'updated'")],
        vec![("detail", "'[]'::jsonb")],
        vec![("actor", "''")],
    ] {
        expect_sqlstate(&store, &audit_insert(&overrides), "23514").await;
    }
    expect_sqlstate(&store, &audit_insert(&[("command", "'unknown'")]), "23503").await;
    assert_eq!(count(&store, "audit_log").await, 5);
}

/// UPDATE, DELETE and TRUNCATE all fail with a permission error and leave the
/// rows as they were.
async fn assert_append_only(store: &Store, observer: &Store, schema: &str) {
    let before = snapshot(observer, schema).await;
    for sql in [
        "UPDATE audit_log SET actor = 'someone else'",
        "UPDATE audit_log SET detail = '{}'::jsonb",
        "DELETE FROM audit_log",
        "TRUNCATE audit_log",
    ] {
        expect_sqlstate(store, sql, "42501").await;
    }
    assert_eq!(snapshot(observer, schema).await, before);
}

#[tokio::test]
async fn the_audit_log_is_append_only_for_the_owner_and_the_read_only_role() {
    const TEST: &str = "audit_append_only";
    let Some((schema, store)) = migrated(TEST).await else {
        return;
    };
    insert_fixture_capability(&store).await;
    exec(&store, &queue_insert(&[])).await;
    exec(&store, &audit_insert(&[])).await;
    exec(
        &store,
        &audit_insert(&[("event", "'refused'"), ("command", "NULL")]),
    )
    .await;

    // dnd_app is the application role and the owner of the table.
    assert_append_only(&store, &store, schema.name()).await;
    let Some(readonly) = readonly(TEST, &schema).await else {
        return;
    };
    assert_append_only(&readonly, &store, schema.name()).await;
}
