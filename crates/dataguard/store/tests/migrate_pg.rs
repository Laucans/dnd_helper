//! Runner behaviour that depends on PostgreSQL, against a real server.
//!
//! Each test starts with `TestSchema::acquire`: with `DATABASE_URL` unset it
//! writes a `SKIPPED` line and returns; set but broken, it fails.

use std::sync::mpsc;

use dataguard_store::testing::TestSchema;
use dataguard_store::{MigrateError, Migration, Store, migrate};

const fn mig(number: u32, name: &'static str, sql: &'static str) -> Migration {
    Migration { number, name, sql }
}

const TWO: &[Migration] = &[
    mig(1, "first", "CREATE TABLE t1 (id integer PRIMARY KEY)"),
    mig(2, "second", "CREATE TABLE t2 (id integer PRIMARY KEY)"),
];

/// `(relname, relkind)` of everything in the store's schema.
async fn relations(store: &Store, schema: &str) -> Vec<(String, String)> {
    store
        .client()
        .query(
            "SELECT c.relname::text, c.relkind::text FROM pg_class c \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = $1 ORDER BY c.relname",
            &[&schema],
        )
        .await
        .expect("list relations")
        .iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect()
}

/// `(number, name, checksum, applied_at)` of every bookkeeping row.
async fn bookkeeping(store: &Store) -> Vec<(i32, String, String, String)> {
    store
        .client()
        .query(
            "SELECT number, name, checksum, applied_at::text FROM schema_migrations ORDER BY number",
            &[],
        )
        .await
        .expect("read bookkeeping")
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2), row.get(3)))
        .collect()
}

async fn exists(store: &Store, table: &str) -> bool {
    store
        .client()
        .query_one("SELECT to_regclass($1) IS NOT NULL", &[&table])
        .await
        .expect("to_regclass")
        .get(0)
}

fn only_bookkeeping() -> Vec<(String, String)> {
    vec![
        ("schema_migrations".to_owned(), "r".to_owned()),
        ("schema_migrations_pkey".to_owned(), "i".to_owned()),
    ]
}

#[tokio::test]
async fn empty_list_creates_only_bookkeeping() {
    let Some(schema) = TestSchema::acquire("empty_list_creates_only_bookkeeping")
        .await
        .expect("harness")
    else {
        return;
    };
    let mut store = schema.connect().await.expect("connect");
    for _ in 0..2 {
        let report = migrate(&mut store, &[]).await.expect("migrate");
        assert_eq!((report.applied, report.current), (0, 0));
        assert_eq!(relations(&store, schema.name()).await, only_bookkeeping());
        assert!(bookkeeping(&store).await.is_empty());
    }
}

#[tokio::test]
async fn replay_is_a_noop() {
    let Some(schema) = TestSchema::acquire("replay_is_a_noop")
        .await
        .expect("harness")
    else {
        return;
    };
    let mut store = schema.connect().await.expect("connect");
    let first = migrate(&mut store, TWO).await.expect("first run");
    assert_eq!((first.applied, first.current), (2, 2));
    let rows = bookkeeping(&store).await;
    let objects = relations(&store, schema.name()).await;
    assert_eq!(rows.len(), 2);

    let second = migrate(&mut store, TWO).await.expect("replay");
    assert_eq!((second.applied, second.current), (0, 2));
    assert_eq!(bookkeeping(&store).await, rows, "applied_at included");
    assert_eq!(relations(&store, schema.name()).await, objects);
}

#[tokio::test]
async fn a_database_at_k_applies_only_the_rest() {
    let Some(schema) = TestSchema::acquire("a_database_at_k_applies_only_the_rest")
        .await
        .expect("harness")
    else {
        return;
    };
    let mut store = schema.connect().await.expect("connect");
    migrate(&mut store, &TWO[..1]).await.expect("first");
    let three = [
        TWO[0],
        TWO[1],
        mig(3, "third", "CREATE TABLE t3 (id integer)"),
    ];
    let report = migrate(&mut store, &three).await.expect("rest");
    assert_eq!((report.applied, report.current), (2, 3));
}

#[tokio::test]
async fn failing_migration_rolls_back() {
    let Some(schema) = TestSchema::acquire("failing_migration_rolls_back")
        .await
        .expect("harness")
    else {
        return;
    };
    let mut store = schema.connect().await.expect("connect");
    let broken = [
        TWO[0],
        mig(
            2,
            "second",
            "CREATE TABLE t2 (id integer); SELECT * FROM missing_table_zz",
        ),
        mig(3, "third", "CREATE TABLE t3 (id integer)"),
    ];
    let error = migrate(&mut store, &broken).await.unwrap_err();
    assert!(
        matches!(error, MigrateError::MigrationFailed { number: 2, .. }),
        "{error}"
    );
    assert!(exists(&store, "t1").await);
    assert!(!exists(&store, "t2").await, "t2 must be rolled back");
    assert!(!exists(&store, "t3").await, "3 is never attempted");
    let numbers: Vec<i32> = bookkeeping(&store).await.iter().map(|r| r.0).collect();
    assert_eq!(numbers, [1]);

    let fixed = [
        TWO[0],
        TWO[1],
        mig(3, "third", "CREATE TABLE t3 (id integer)"),
    ];
    let report = migrate(&mut store, &fixed).await.expect("retry");
    assert_eq!((report.applied, report.current), (2, 3));
}

#[tokio::test]
async fn checksum_drift_is_refused() {
    let Some(schema) = TestSchema::acquire("checksum_drift_is_refused")
        .await
        .expect("harness")
    else {
        return;
    };
    let mut store = schema.connect().await.expect("connect");
    migrate(&mut store, &TWO[..1]).await.expect("first");
    let drifted = [mig(1, "first", "CREATE TABLE t1 (id bigint)"), TWO[1]];
    let error = migrate(&mut store, &drifted).await.unwrap_err();
    assert_eq!(error, MigrateError::ChecksumMismatch { number: 1 });
    assert!(!exists(&store, "t2").await, "nothing is applied on drift");
}

#[tokio::test]
async fn unknown_applied_is_refused() {
    let Some(schema) = TestSchema::acquire("unknown_applied_is_refused")
        .await
        .expect("harness")
    else {
        return;
    };
    let mut store = schema.connect().await.expect("connect");
    migrate(&mut store, TWO).await.expect("both");
    let error = migrate(&mut store, &TWO[..1]).await.unwrap_err();
    assert_eq!(error, MigrateError::UnknownApplied { number: 2 });
}

#[tokio::test]
async fn out_of_sequence_applied_is_refused() {
    let Some(schema) = TestSchema::acquire("out_of_sequence_applied_is_refused")
        .await
        .expect("harness")
    else {
        return;
    };
    let mut store = schema.connect().await.expect("connect");
    migrate(&mut store, TWO).await.expect("both");
    store
        .client()
        .batch_execute("DELETE FROM schema_migrations WHERE number = 1")
        .await
        .expect("punch a hole");
    let error = migrate(&mut store, TWO).await.unwrap_err();
    assert_eq!(error, MigrateError::AppliedOutOfSequence { number: 2 });
}

#[tokio::test]
async fn concurrent_runners_apply_once() {
    for _ in 0..5 {
        let Some(schema) = TestSchema::acquire("concurrent_runners_apply_once")
            .await
            .expect("harness")
        else {
            return;
        };
        let mut a = schema.connect().await.expect("connect a");
        let mut b = schema.connect().await.expect("connect b");
        let (ra, rb) = tokio::join!(migrate(&mut a, TWO), migrate(&mut b, TWO));
        let (ra, rb) = (ra.expect("runner a"), rb.expect("runner b"));
        assert_eq!(ra.applied + rb.applied, 2);
        assert_eq!(ra.current, 2);
        assert_eq!(rb.current, 2);
        assert_eq!(bookkeeping(&a).await.len(), 2);
    }
}

#[tokio::test]
async fn harness_drops_schema_after_panic() {
    let Some(outer) = TestSchema::acquire("harness_drops_schema_after_panic")
        .await
        .expect("harness")
    else {
        return;
    };
    let (sender, receiver) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async move {
            let inner = TestSchema::acquire("harness_inner_panics")
                .await
                .expect("harness")
                .expect("database configured");
            sender.send(inner.name().to_owned()).expect("send name");
            panic!("deliberate panic: the schema must still be dropped");
        });
    });
    assert!(worker.join().is_err(), "the worker must have panicked");
    let name = receiver.recv().expect("schema name");

    let store = outer.connect().await.expect("connect");
    let found: bool = store
        .client()
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_namespace WHERE nspname = $1)",
            &[&name],
        )
        .await
        .expect("pg_namespace")
        .get(0);
    assert!(!found, "schema {name} leaked after a panic");
}

#[tokio::test]
async fn dropping_the_schema_removes_it() {
    let Some(schema) = TestSchema::acquire("dropping_the_schema_removes_it")
        .await
        .expect("harness")
    else {
        return;
    };
    let name = schema.name().to_owned();
    let observer = schema.connect().await.expect("connect");
    drop(schema);
    let found: bool = observer
        .client()
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_namespace WHERE nspname = $1)",
            &[&name],
        )
        .await
        .expect("pg_namespace")
        .get(0);
    assert!(!found);
}

#[tokio::test]
async fn parallel_schemas_are_isolated() {
    let Some(a) = TestSchema::acquire("parallel_schemas_are_isolated")
        .await
        .expect("harness")
    else {
        return;
    };
    let b = TestSchema::acquire("parallel_schemas_are_isolated")
        .await
        .expect("harness")
        .expect("database configured");
    assert_ne!(a.name(), b.name());
    let store_a = a.connect().await.expect("connect a");
    let store_b = b.connect().await.expect("connect b");
    store_a
        .client()
        .batch_execute("CREATE TABLE only_in_a (id integer)")
        .await
        .expect("create");
    assert!(exists(&store_a, "only_in_a").await);
    assert!(!exists(&store_b, "only_in_a").await);
    assert!(!exists(&store_b, "public.only_in_a").await);
}
