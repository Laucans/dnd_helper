//! Rules 23-25: one row, a table and not a sequence, gapless.

use super::*;

const INCREMENT: &str = "UPDATE data_version_counter SET value = value + 1 RETURNING value";

async fn increment(store: &Store) -> i64 {
    store
        .client()
        .query_one(INCREMENT, &[])
        .await
        .expect("increment")
        .get(0)
}

#[tokio::test]
async fn a_second_row_cannot_exist() {
    let Some((_schema, store)) = migrated("a_second_row_cannot_exist").await else {
        return;
    };
    // The application role holds no INSERT.
    expect_sqlstate(
        &store,
        "INSERT INTO data_version_counter (id, value) VALUES (false, 0)",
        "42501",
    )
    .await;
    expect_sqlstate(
        &store,
        "INSERT INTO data_version_counter (id, value) VALUES (true, 1)",
        "42501",
    )
    .await;
    // The owner gives itself INSERT inside a transaction that is rolled back,
    // to prove the constraints hold on their own, not only the privilege.
    exec(
        &store,
        "BEGIN; GRANT INSERT ON data_version_counter TO dnd_app",
    )
    .await;
    expect_sqlstate(
        &store,
        "INSERT INTO data_version_counter (id, value) VALUES (false, 0)",
        "23514",
    )
    .await;
    exec(
        &store,
        "ROLLBACK; BEGIN; GRANT INSERT ON data_version_counter TO dnd_app",
    )
    .await;
    expect_sqlstate(
        &store,
        "INSERT INTO data_version_counter (id, value) VALUES (true, 5)",
        "23505",
    )
    .await;
    exec(&store, "ROLLBACK").await;
    expect_sqlstate(
        &store,
        "INSERT INTO data_version_counter (id, value) VALUES (true, 5)",
        "42501",
    )
    .await;
    assert_eq!(count(&store, "data_version_counter").await, 1);
    assert_eq!(counter_value(&store).await, 0);
}

#[tokio::test]
async fn the_counter_row_resists_what_the_role_cannot_do() {
    let Some((_schema, store)) = migrated("the_counter_row_resists").await else {
        return;
    };
    expect_sqlstate(
        &store,
        "UPDATE data_version_counter SET value = -1",
        "23514",
    )
    .await;
    expect_sqlstate(&store, "UPDATE data_version_counter SET id = id", "42501").await;
    expect_sqlstate(&store, "DELETE FROM data_version_counter", "42501").await;
    expect_sqlstate(&store, "TRUNCATE data_version_counter", "42501").await;
    assert_eq!(count(&store, "data_version_counter").await, 1);
    assert_eq!(counter_value(&store).await, 0);
}

#[tokio::test]
async fn a_rolled_back_increment_leaves_no_gap() {
    let Some((_schema, store)) = migrated("a_rolled_back_increment_leaves_no_gap").await else {
        return;
    };
    exec(&store, "BEGIN").await;
    assert_eq!(increment(&store).await, 1);
    exec(&store, "ROLLBACK").await;
    assert_eq!(counter_value(&store).await, 0);
    // The next increments are consecutive: a sequence would have skipped 1.
    assert_eq!(increment(&store).await, 1);
    assert_eq!(increment(&store).await, 2);
}
