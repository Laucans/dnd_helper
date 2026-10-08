//! Tests of the initial schema (migration 1), against a real PostgreSQL.
//!
//! In-crate because the read-only connection needs `StoreConfig::from_value`
//! and `with_search_path`, which are crate-private. Every test works in its own
//! schema, migrated through the runner. With `DATABASE_URL` unset a test writes
//! its `SKIPPED` line and returns; with `DATABASE_URL_READONLY` unset the
//! tests that need the read-only role do the same for that part.

mod audit;
mod counter;
mod privileges;
mod queue;
mod registry;
mod replay;

use crate::config::StoreConfig;
use crate::connect::{Store, connect};
use crate::migrate::{MigrateReport, migrate};
use crate::migrations::MIGRATIONS;
use crate::testing::{TestSchema, emit};

/// The application role of #73.
const APP_ROLE: &str = "dnd_app";
/// The read-only role of #73.
const READONLY_ROLE: &str = "dnd_readonly";
/// The variable holding the read-only address, read by test code only.
const READONLY_VAR: &str = "DATABASE_URL_READONLY";

/// Every table the migrations and the runner leave in the schema.
const TABLES: [&str; 5] = [
    "audit_log",
    "data_capability_registry",
    "data_queue",
    "data_version_counter",
    "schema_migrations",
];

/// A fresh schema brought to the full schema through the runner, or `None`
/// after the `SKIPPED` line.
async fn migrated(test: &str) -> Option<(TestSchema, Store)> {
    let schema = TestSchema::acquire(test).await.expect("harness")?;
    let mut store = schema.connect().await.expect("connect");
    let report = migrate(&mut store, MIGRATIONS).await.expect("migrate");
    let all = u32::try_from(MIGRATIONS.len()).expect("a short list");
    assert_eq!(
        report,
        MigrateReport {
            applied: all,
            current: all
        }
    );
    Some((schema, store))
}

/// A connection as the read-only role on `schema`, or `None` after the
/// `SKIPPED` line. A variable that is set but unusable fails the test.
async fn readonly(test: &str, schema: &TestSchema) -> Option<Store> {
    let Some(value) = std::env::var_os(READONLY_VAR) else {
        emit(&format!(
            "SKIPPED {test}: {READONLY_VAR} is not set; this test needs the read-only role"
        ));
        return None;
    };
    let config = StoreConfig::from_value(Some(value))
        .unwrap_or_else(|_| panic!("{READONLY_VAR} is set but unusable"))
        .with_search_path(schema.name());
    let store = connect(&config)
        .await
        .unwrap_or_else(|_| panic!("{READONLY_VAR} is set but the database refuses it"));
    Some(store)
}

fn sqlstate(error: &tokio_postgres::Error) -> Option<&str> {
    error.code().map(tokio_postgres::error::SqlState::code)
}

/// Runs `sql` and returns the SQLSTATE it failed with.
async fn failure_of(store: &Store, sql: &str) -> String {
    match store.client().batch_execute(sql).await {
        Ok(()) => panic!("expected a failure, the statement succeeded: {sql}"),
        Err(error) => sqlstate(&error)
            .unwrap_or_else(|| panic!("failure without SQLSTATE: {error:?}"))
            .to_owned(),
    }
}

async fn expect_sqlstate(store: &Store, sql: &str, code: &str) {
    assert_eq!(failure_of(store, sql).await, code, "{sql}");
}

async fn exec(store: &Store, sql: &str) {
    if let Err(error) = store.client().batch_execute(sql).await {
        panic!("statement failed: {sql}\n{error:?}");
    }
}

async fn count(store: &Store, table: &str) -> i64 {
    store
        .client()
        .query_one(&format!("SELECT count(*) FROM {table}"), &[])
        .await
        .expect("count")
        .get(0)
}

async fn counter_value(store: &Store) -> i64 {
    store
        .client()
        .query_one("SELECT value FROM data_version_counter", &[])
        .await
        .expect("counter")
        .get(0)
}

/// `INSERT INTO table (...) VALUES (...)`: `base` columns with the SQL literal
/// of each, `overrides` replacing a base value or adding a column. `DEFAULT`
/// is a valid value.
fn insert_sql(table: &str, base: &[(&str, &str)], overrides: &[(&str, &str)]) -> String {
    let mut columns: Vec<(&str, &str)> = base.to_vec();
    for &(name, value) in overrides {
        match columns.iter_mut().find(|(existing, _)| *existing == name) {
            Some(slot) => slot.1 = value,
            None => columns.push((name, value)),
        }
    }
    let names: Vec<&str> = columns.iter().map(|(name, _)| *name).collect();
    let values: Vec<&str> = columns.iter().map(|(_, value)| *value).collect();
    format!(
        "INSERT INTO {table} ({}) VALUES ({})",
        names.join(", "),
        values.join(", ")
    )
}

/// A valid queue row for the fixture capability, with `overrides` applied.
fn queue_insert(overrides: &[(&str, &str)]) -> String {
    insert_sql(
        "data_queue",
        &[
            ("command", "'c1'"),
            ("data_capability", "'fixture.createProbe@1'"),
            ("by", "'gm'"),
            ("partition", "'Probe/1'"),
            ("position", "0"),
            ("based_on", r#"'{"version":0}'::jsonb"#),
            ("payload", "'{}'::jsonb"),
        ],
        overrides,
    )
}

/// A valid registry row (the fake `fixture.createProbe@1`), with `overrides`
/// applied. Test-only, clearly fake, never shipped.
fn registry_insert(overrides: &[(&str, &str)]) -> String {
    insert_sql(
        "data_capability_registry",
        &[
            ("name", "'fixture.createProbe'"),
            ("version", "1"),
            ("owner", "'dataguard'"),
            ("effect", "'insert'"),
            ("target", r#"'{"aggregate":"Probe"}'::jsonb"#),
            ("touches", "'[]'::jsonb"),
            ("mode", "'overwrite'"),
            ("payload", "'{}'::jsonb"),
            ("invariants", "'[]'::jsonb"),
            ("permissions", "'[]'::jsonb"),
            ("callable_by", r#"'["*"]'::jsonb"#),
            ("idempotency_key", "'optional'"),
            ("manifest", "'{}'::jsonb"),
        ],
        overrides,
    )
}

async fn insert_fixture_capability(store: &Store) {
    exec(store, &registry_insert(&[])).await;
}

/// An audit row, with `overrides` applied.
fn audit_insert(overrides: &[(&str, &str)]) -> String {
    insert_sql(
        "audit_log",
        &[
            ("event", "'enqueued'"),
            ("command", "'c1'"),
            ("actor", "'gm'"),
            ("data_capability", "'fixture.createProbe@1'"),
        ],
        overrides,
    )
}

/// Everything observable about the schema and its rows, sorted: columns,
/// constraints, indexes, table and column privileges, schema privileges, and
/// the text of every row of every table.
async fn snapshot(store: &Store, schema: &str) -> Vec<String> {
    let catalog = [
        "SELECT 'col ' || table_name || '.' || column_name || ' ' || data_type || ' ' \
         || is_nullable || ' ' || coalesce(column_default, '') || ' ' \
         || coalesce(generation_expression, '') \
         FROM information_schema.columns WHERE table_schema = $1",
        "SELECT 'con ' || cl.relname || ' ' || c.conname || ' ' || pg_get_constraintdef(c.oid) \
         FROM pg_constraint c JOIN pg_class cl ON cl.oid = c.conrelid \
         JOIN pg_namespace n ON n.oid = c.connamespace WHERE n.nspname = $1",
        "SELECT 'idx ' || indexdef FROM pg_indexes WHERE schemaname = $1",
        "SELECT 'acl ' || c.relname || ' ' || coalesce(c.relacl::text, '') \
         FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = $1 AND c.relkind IN ('r', 'p', 'S')",
        "SELECT 'colacl ' || c.relname || '.' || a.attname || ' ' || a.attacl::text \
         FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = $1 AND a.attacl IS NOT NULL",
        "SELECT 'nspacl ' || coalesce(nspacl::text, '') FROM pg_namespace WHERE nspname = $1",
    ];
    let mut lines = Vec::new();
    for sql in catalog {
        let rows = store
            .client()
            .query(sql, &[&schema])
            .await
            .expect("catalog");
        lines.extend(rows.iter().map(|row| row.get::<_, String>(0)));
    }
    for table in TABLES {
        let rows = store
            .client()
            .query(
                &format!("SELECT 'row {table} ' || t::text FROM {table} t"),
                &[],
            )
            .await
            .expect("rows");
        lines.extend(rows.iter().map(|row| row.get::<_, String>(0)));
    }
    lines.sort();
    lines
}

/// The tables of `schema` (ordinary and partitioned), sorted.
async fn tables_of(store: &Store, schema: &str) -> Vec<String> {
    store
        .client()
        .query(
            "SELECT c.relname::text FROM pg_class c \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = $1 AND c.relkind IN ('r', 'p') ORDER BY 1",
            &[&schema],
        )
        .await
        .expect("tables")
        .iter()
        .map(|row| row.get(0))
        .collect()
}
