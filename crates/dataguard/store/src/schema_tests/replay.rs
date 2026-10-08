//! Rules 1, 3, 4, 5-7, 24, 30 and the foreign-key rule of the architecture.

use super::*;
use crate::error::MigrateError;
use crate::migrate::{Migration, checksum};
use crate::migrations::INITIAL_SCHEMA;

/// A migration list whose single entry is `sql`. Leaks the text: tests only.
fn migration_with(sql: String) -> [Migration; 1] {
    [Migration {
        number: 1,
        name: "initial_schema",
        sql: Box::leak(sql.into_boxed_str()),
    }]
}

#[tokio::test]
async fn full_schema_on_empty_schema() {
    let Some((schema, store)) = migrated("full_schema_on_empty_schema").await else {
        return;
    };
    // Rule 1: exactly these tables, nothing for an aggregate, hold or review.
    assert_eq!(tables_of(&store, schema.name()).await, TABLES);
    // Rule 24: no sequence anywhere.
    let sequences: i64 = store
        .client()
        .query_one(
            "SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = $1 AND c.relkind = 'S'",
            &[&schema.name()],
        )
        .await
        .expect("sequences")
        .get(0);
    assert_eq!(sequences, 0);
    for table in ["data_queue", "audit_log", "data_capability_registry"] {
        assert_eq!(count(&store, table).await, 0, "{table} starts empty");
    }
    let counter = store
        .client()
        .query("SELECT id, value FROM data_version_counter", &[])
        .await
        .expect("counter");
    assert_eq!(counter.len(), 1);
    assert!(counter[0].get::<_, bool>(0));
    assert_eq!(counter[0].get::<_, i64>(1), 0);
    let recorded = store
        .client()
        .query("SELECT number, name, checksum FROM schema_migrations", &[])
        .await
        .expect("bookkeeping");
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].get::<_, i32>(0), 1);
    assert_eq!(recorded[0].get::<_, String>(1), "initial_schema");
    assert_eq!(recorded[0].get::<_, String>(2), checksum(INITIAL_SCHEMA));
}

#[tokio::test]
async fn replay_changes_nothing() {
    let Some((schema, mut store)) = migrated("replay_changes_nothing").await else {
        return;
    };
    insert_fixture_capability(&store).await;
    exec(&store, &queue_insert(&[])).await;
    exec(&store, &audit_insert(&[])).await;
    for _ in 0..5 {
        exec(&store, "UPDATE data_version_counter SET value = value + 1").await;
    }
    let before = snapshot(&store, schema.name()).await;

    let report = migrate(&mut store, MIGRATIONS).await.expect("replay");
    assert_eq!(
        report,
        MigrateReport {
            applied: 0,
            current: 1
        }
    );

    // Rules 5, 6 and 30: schema, rows, counter and audit are identical.
    assert_eq!(snapshot(&store, schema.name()).await, before);
    assert_eq!(counter_value(&store).await, 5);
    assert_eq!(count(&store, "audit_log").await, 1);
}

#[tokio::test]
async fn failing_migration_leaves_nothing() {
    let Some(schema) = TestSchema::acquire("failing_migration_leaves_nothing")
        .await
        .expect("harness")
    else {
        return;
    };
    let mut store = schema.connect().await.expect("connect");
    let broken = migration_with(format!("{INITIAL_SCHEMA}\nSELECT 1/0;"));
    let error = migrate(&mut store, &broken).await.unwrap_err();
    assert!(
        matches!(
            &error,
            MigrateError::MigrationFailed { number: 1, sqlstate } if sqlstate == "22012"
        ),
        "{error}"
    );
    // Only the runner's table survives, empty; the USAGE grant was rolled back.
    assert_eq!(
        tables_of(&store, schema.name()).await,
        ["schema_migrations"]
    );
    assert_eq!(count(&store, "schema_migrations").await, 0);
    let usage: bool = store
        .client()
        .query_one(
            "SELECT has_schema_privilege('dnd_readonly', $1, 'USAGE')",
            &[&schema.name()],
        )
        .await
        .expect("privilege")
        .get(0);
    assert!(!usage);
    // The next run starts from the beginning and succeeds.
    let report = migrate(&mut store, MIGRATIONS).await.expect("retry");
    assert_eq!(
        report,
        MigrateReport {
            applied: 1,
            current: 1
        }
    );
}

#[tokio::test]
async fn missing_role_fails_and_names_it() {
    let Some(schema) = TestSchema::acquire("missing_role_fails_and_names_it")
        .await
        .expect("harness")
    else {
        return;
    };
    let mut store = schema.connect().await.expect("connect");
    let absent = "dg_test_no_such_role";
    let broken = migration_with(INITIAL_SCHEMA.replace(READONLY_ROLE, absent));
    let error = migrate(&mut store, &broken).await.unwrap_err();
    assert!(
        matches!(
            &error,
            MigrateError::MigrationFailed { number: 1, sqlstate } if sqlstate == "42704"
        ),
        "{error}"
    );
    assert_eq!(
        tables_of(&store, schema.name()).await,
        ["schema_migrations"]
    );

    // The runner reports the SQLSTATE only; the server message names the role.
    let start = INITIAL_SCHEMA.find("DO $$").expect("guard block");
    let end = start + INITIAL_SCHEMA[start..].find("$$;").expect("guard end") + "$$;".len();
    let guard = INITIAL_SCHEMA[start..end].replace(READONLY_ROLE, absent);
    let failure = store
        .client()
        .batch_execute(&guard)
        .await
        .expect_err("the guard must refuse");
    let message = failure.as_db_error().expect("a server error").message();
    assert!(message.contains(absent), "{message}");
}

#[tokio::test]
async fn parallel_schemas_do_not_interfere() {
    let (first, second) = tokio::join!(
        migrated("parallel_schemas_first"),
        migrated("parallel_schemas_second")
    );
    let (Some((first_schema, first_store)), Some((second_schema, second_store))) = (first, second)
    else {
        return;
    };
    assert_ne!(first_schema.name(), second_schema.name());
    assert_eq!(tables_of(&first_store, first_schema.name()).await, TABLES);
    assert_eq!(tables_of(&second_store, second_schema.name()).await, TABLES);
    exec(
        &first_store,
        "UPDATE data_version_counter SET value = value + 1",
    )
    .await;
    assert_eq!(counter_value(&first_store).await, 1);
    assert_eq!(counter_value(&second_store).await, 0);
}

#[tokio::test]
async fn every_fk_declares_restrict() {
    let Some((schema, store)) = migrated("every_fk_declares_restrict").await else {
        return;
    };
    let rows = store
        .client()
        .query(
            "SELECT c.conname::text, c.confdeltype::text, c.confupdtype::text \
             FROM pg_constraint c JOIN pg_namespace n ON n.oid = c.connamespace \
             WHERE n.nspname = $1 AND c.contype = 'f'",
            &[&schema.name()],
        )
        .await
        .expect("foreign keys");
    // data_queue -> registry, data_queue -> data_queue, audit_log -> data_queue.
    assert_eq!(rows.len(), 3);
    for row in &rows {
        let name: String = row.get(0);
        assert_eq!(row.get::<_, String>(1), "r", "{name} ON DELETE");
        assert_eq!(row.get::<_, String>(2), "r", "{name} ON UPDATE");
    }
}
