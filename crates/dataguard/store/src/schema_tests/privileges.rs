//! Rules 35-39: the grants, catalog-driven so a new table needs a decision.

use super::*;

const PRIVILEGES: [&str; 7] = [
    "SELECT",
    "INSERT",
    "UPDATE",
    "DELETE",
    "TRUNCATE",
    "REFERENCES",
    "TRIGGER",
];

/// What a table grants: table-level privileges per role, and the columns the
/// application role may UPDATE.
struct Grants {
    app: &'static [&'static str],
    readonly: &'static [&'static str],
    app_update_columns: &'static [&'static str],
}

/// The decision for each table. A table missing here fails the test.
fn decided(table: &str) -> Option<Grants> {
    let grants = |app, app_update_columns| Grants {
        app,
        readonly: &["SELECT"],
        app_update_columns,
    };
    match table {
        "data_queue" => Some(grants(
            &["SELECT", "INSERT"],
            &[
                "state",
                "based_on",
                "projection",
                "your_value",
                "confirmation",
                "parked",
                "requeued_from",
                "data_version",
                "violations",
            ],
        )),
        "data_version_counter" => Some(grants(&["SELECT"], &["value"])),
        "audit_log" => Some(grants(&["SELECT", "INSERT"], &[])),
        // UPDATE (ref) satisfies the foreign-key check of the queue; see the
        // migration.
        "data_capability_registry" => Some(grants(&["SELECT", "INSERT"], &["ref"])),
        "schema_migrations" => Some(grants(&["SELECT", "INSERT"], &[])),
        _ => None,
    }
}

async fn has_table_privilege(store: &Store, role: &str, table: &str, privilege: &str) -> bool {
    store
        .client()
        .query_one(
            "SELECT has_table_privilege($1, $2, $3)",
            &[&role, &table, &privilege],
        )
        .await
        .expect("has_table_privilege")
        .get(0)
}

async fn columns_of(store: &Store, schema: &str, table: &str) -> Vec<String> {
    store
        .client()
        .query(
            "SELECT a.attname::text FROM pg_attribute a \
             JOIN pg_class c ON c.oid = a.attrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = $1 AND c.relname = $2 AND a.attnum > 0 AND NOT a.attisdropped \
             ORDER BY a.attnum",
            &[&schema, &table],
        )
        .await
        .expect("columns")
        .iter()
        .map(|row| row.get(0))
        .collect()
}

#[tokio::test]
async fn the_privilege_matrix_is_exact() {
    let Some((schema, store)) = migrated("privilege_matrix_is_exact").await else {
        return;
    };
    let schema_name = schema.name();
    for table in tables_of(&store, schema_name).await {
        let grants = decided(&table)
            .unwrap_or_else(|| panic!("table {table} has no privilege decision in this test"));
        for privilege in PRIVILEGES {
            for (role, granted) in [(APP_ROLE, grants.app), (READONLY_ROLE, grants.readonly)] {
                assert_eq!(
                    has_table_privilege(&store, role, &table, privilege).await,
                    granted.contains(&privilege),
                    "{role} {privilege} on {table}"
                );
            }
        }
        for column in columns_of(&store, schema_name, &table).await {
            let can_update: bool = store
                .client()
                .query_one(
                    "SELECT has_column_privilege($1, $2, $3, 'UPDATE')",
                    &[&APP_ROLE, &table, &column],
                )
                .await
                .expect("has_column_privilege")
                .get(0);
            assert_eq!(
                can_update,
                grants.app_update_columns.contains(&column.as_str()),
                "{APP_ROLE} UPDATE on {table}.{column}"
            );
            let readonly_can_update: bool = store
                .client()
                .query_one(
                    "SELECT has_column_privilege($1, $2, $3, 'UPDATE')",
                    &[&READONLY_ROLE, &table, &column],
                )
                .await
                .expect("has_column_privilege")
                .get(0);
            assert!(
                !readonly_can_update,
                "{READONLY_ROLE} UPDATE on {table}.{column}"
            );
        }
    }

    // Rule 39: PUBLIC holds nothing, at table or column level.
    let public: Vec<String> = store
        .client()
        .query(
            "SELECT c.relname::text FROM pg_class c \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             CROSS JOIN LATERAL aclexplode(c.relacl) a \
             WHERE n.nspname = $1 AND a.grantee = 0 \
             UNION ALL \
             SELECT c.relname::text || '.' || t.attname::text FROM pg_class c \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             JOIN pg_attribute t ON t.attrelid = c.oid \
             CROSS JOIN LATERAL aclexplode(t.attacl) a \
             WHERE n.nspname = $1 AND a.grantee = 0",
            &[&schema_name],
        )
        .await
        .expect("public grants")
        .iter()
        .map(|row| row.get(0))
        .collect();
    assert!(public.is_empty(), "PUBLIC holds privileges on {public:?}");

    // Both roles may use the schema, neither may create in it: the read-only
    // role cannot (dnd_app owns what it migrates, see the README).
    for role in [APP_ROLE, READONLY_ROLE] {
        let usage: bool = store
            .client()
            .query_one(
                "SELECT has_schema_privilege($1, $2, 'USAGE')",
                &[&role, &schema_name],
            )
            .await
            .expect("usage")
            .get(0);
        assert!(usage, "{role} USAGE");
    }
    let create: bool = store
        .client()
        .query_one(
            "SELECT has_schema_privilege($1, $2, 'CREATE')",
            &[&READONLY_ROLE, &schema_name],
        )
        .await
        .expect("create")
        .get(0);
    assert!(!create);
}

#[tokio::test]
async fn readonly_cannot_write_anything() {
    const TEST: &str = "readonly_cannot_write_anything";
    let Some((schema, store)) = migrated(TEST).await else {
        return;
    };
    insert_fixture_capability(&store).await;
    exec(&store, &queue_insert(&[])).await;
    exec(&store, &audit_insert(&[])).await;
    exec(&store, "UPDATE data_version_counter SET value = value + 1").await;
    let Some(readonly) = readonly(TEST, &schema).await else {
        return;
    };
    let before = snapshot(&store, schema.name()).await;
    let tables = tables_of(&readonly, schema.name()).await;
    assert_eq!(tables, TABLES);
    for table in &tables {
        let first = columns_of(&readonly, schema.name(), table)
            .await
            .into_iter()
            .next()
            .expect("a column");
        let rows = count(&readonly, table).await;
        assert_eq!(count(&store, table).await, rows, "SELECT on {table}");
        for sql in [
            format!("INSERT INTO {table} DEFAULT VALUES"),
            format!("UPDATE {table} SET {first} = {first}"),
            format!("DELETE FROM {table}"),
            format!("TRUNCATE {table}"),
        ] {
            expect_sqlstate(&readonly, &sql, "42501").await;
        }
    }
    assert_eq!(snapshot(&store, schema.name()).await, before);
}

#[tokio::test]
async fn roles_are_bounded() {
    let Some((_schema, store)) = migrated("roles_are_bounded").await else {
        return;
    };
    for role in [APP_ROLE, READONLY_ROLE] {
        let row = store
            .client()
            .query_one(
                "SELECT rolsuper, rolcreaterole, rolbypassrls FROM pg_roles WHERE rolname = $1",
                &[&role],
            )
            .await
            .expect("role");
        for (index, attribute) in ["SUPERUSER", "CREATEROLE", "BYPASSRLS"].iter().enumerate() {
            assert!(!row.get::<_, bool>(index), "{role} has {attribute}");
        }
    }
    let links: i64 = store
        .client()
        .query_one(
            "SELECT count(*) FROM pg_auth_members m \
             JOIN pg_roles member ON member.oid = m.member \
             JOIN pg_roles granted ON granted.oid = m.roleid \
             WHERE member.rolname IN ($1, $2) AND granted.rolname IN ($1, $2)",
            &[&APP_ROLE, &READONLY_ROLE],
        )
        .await
        .expect("memberships")
        .get(0);
    assert_eq!(links, 0, "the two roles are members of each other");
}
