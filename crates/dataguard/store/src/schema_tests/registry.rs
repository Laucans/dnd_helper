//! Rules 31-34: the registry of DataCapability versions.

use super::*;

#[tokio::test]
async fn the_registry_is_empty_after_migration() {
    let Some((_schema, store)) = migrated("registry_empty_after_migration").await else {
        return;
    };
    assert_eq!(count(&store, "data_capability_registry").await, 0);
}

#[tokio::test]
async fn name_and_version_are_the_key() {
    let Some((_schema, store)) = migrated("registry_key").await else {
        return;
    };
    insert_fixture_capability(&store).await;
    // Same id@version, other content: refused on the key.
    expect_sqlstate(
        &store,
        &registry_insert(&[("effect", "'upsert'"), ("manifest", r#"'{"x":1}'::jsonb"#)]),
        "23505",
    )
    .await;
    // Another version, another name: accepted.
    exec(&store, &registry_insert(&[("version", "2")])).await;
    exec(
        &store,
        &registry_insert(&[("name", "'fixture.otherProbe'")]),
    )
    .await;
    let refs: Vec<String> = store
        .client()
        .query("SELECT ref FROM data_capability_registry ORDER BY ref", &[])
        .await
        .expect("refs")
        .iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(
        refs,
        [
            "fixture.createProbe@1",
            "fixture.createProbe@2",
            "fixture.otherProbe@1"
        ]
    );
}

#[tokio::test]
async fn values_outside_the_contract_are_refused() {
    let Some((_schema, store)) = migrated("registry_contract_values").await else {
        return;
    };
    for overrides in [
        vec![("effect", "'merge'")],
        vec![("mode", "'last_write'")],
        vec![("idempotency_key", "'never'")],
        vec![("owner", "'monde'")],
        vec![("version", "0")],
        vec![("name", "'NoDot'")],
        vec![("name", "'Upper.case'")],
        vec![("target", "'{}'::jsonb")],
        vec![("target", r#"'{"aggregate":"lower"}'::jsonb"#)],
        vec![("touches", "'{}'::jsonb")],
        vec![("callable_by", "'{}'::jsonb")],
        vec![("manifest", "'[]'::jsonb")],
    ] {
        expect_sqlstate(&store, &registry_insert(&overrides), "23514").await;
    }
    assert_eq!(count(&store, "data_capability_registry").await, 0);
    for effect in ["insert", "update", "delete", "upsert"] {
        exec(
            &store,
            &registry_insert(&[
                ("name", &format!("'fixture.{effect}Probe'")),
                ("effect", &format!("'{effect}'")),
            ]),
        )
        .await;
    }
}

#[tokio::test]
async fn a_registered_row_cannot_change_or_go() {
    let Some((schema, store)) = migrated("registry_immutable").await else {
        return;
    };
    insert_fixture_capability(&store).await;
    let before = snapshot(&store, schema.name()).await;
    for sql in [
        "UPDATE data_capability_registry SET effect = 'update'",
        "UPDATE data_capability_registry SET manifest = '{}'::jsonb",
        "UPDATE data_capability_registry SET name = name",
        "UPDATE data_capability_registry SET version = version",
        "DELETE FROM data_capability_registry",
        "TRUNCATE data_capability_registry",
    ] {
        expect_sqlstate(&store, sql, "42501").await;
    }
    // The one column the role may update is generated: no value is accepted.
    expect_sqlstate(
        &store,
        "UPDATE data_capability_registry SET ref = 'x'",
        "428C9",
    )
    .await;
    assert_eq!(snapshot(&store, schema.name()).await, before);
}
