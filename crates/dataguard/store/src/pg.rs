//! The PostgreSQL side of the runner.
//!
//! No statement here qualifies a schema name: objects land in the schema the
//! connection's `search_path` resolves to.

use crate::connect::Store;
use crate::migrate::{AppliedMigration, Migration, MigrationStore, StoreError};

/// First key of the advisory lock; the second is the hash of the target schema,
/// so parallel schemas never serialise each other.
const MIGRATION_LOCK_CLASS: i32 = 0x4447_4d31;

const CREATE_BOOKKEEPING: &str = "CREATE TABLE IF NOT EXISTS schema_migrations (\
    number integer PRIMARY KEY CHECK (number >= 1), \
    name text NOT NULL, \
    checksum text NOT NULL, \
    applied_at timestamptz NOT NULL DEFAULT now())";

fn sql_error(step: &'static str, error: &tokio_postgres::Error) -> StoreError {
    StoreError::Sql {
        step,
        sqlstate: error.code().map(|code| code.code().to_owned()),
    }
}

impl MigrationStore for Store {
    async fn lock_and_prepare(&mut self) -> Result<(), StoreError> {
        let client = self.client();
        let schema: Option<String> = client
            .query_one("SELECT current_schema()", &[])
            .await
            .map_err(|e| sql_error("resolve schema", &e))?
            .get(0);
        // pg_advisory_lock is strict: a NULL key would silently take no lock.
        if schema.is_none() {
            return Err(StoreError::NoTargetSchema);
        }
        // The lock comes before CREATE TABLE IF NOT EXISTS: two concurrent
        // creations would otherwise race on pg_type.
        client
            .execute(
                "SELECT pg_advisory_lock($1, hashtext(current_schema()))",
                &[&MIGRATION_LOCK_CLASS],
            )
            .await
            .map_err(|e| sql_error("lock", &e))?;
        client
            .batch_execute(CREATE_BOOKKEEPING)
            .await
            .map_err(|e| sql_error("create bookkeeping table", &e))
    }

    async fn applied(&mut self) -> Result<Vec<AppliedMigration>, StoreError> {
        let rows = self
            .client()
            .query(
                "SELECT number, checksum FROM schema_migrations ORDER BY number",
                &[],
            )
            .await
            .map_err(|e| sql_error("read bookkeeping", &e))?;
        rows.iter()
            .map(|row| {
                let number: i32 = row.get(0);
                Ok(AppliedMigration {
                    number: u32::try_from(number).map_err(|_| StoreError::Sql {
                        step: "read bookkeeping",
                        sqlstate: None,
                    })?,
                    checksum: row.get(1),
                })
            })
            .collect()
    }

    async fn apply(&mut self, migration: &Migration, checksum: &str) -> Result<(), StoreError> {
        let number = i32::try_from(migration.number).map_err(|_| StoreError::Sql {
            step: "apply",
            sqlstate: None,
        })?;
        let tx = self
            .client_mut()
            .transaction()
            .await
            .map_err(|e| sql_error("begin", &e))?;
        let outcome = async {
            tx.batch_execute(migration.sql)
                .await
                .map_err(|e| sql_error("apply", &e))?;
            tx.execute(
                "INSERT INTO schema_migrations (number, name, checksum) VALUES ($1, $2, $3)",
                &[&number, &migration.name, &checksum],
            )
            .await
            .map_err(|e| sql_error("record", &e))
        }
        .await;
        match outcome {
            Ok(_) => tx.commit().await.map_err(|e| sql_error("commit", &e)),
            Err(error) => {
                // The failure to report is the first one; if the rollback also
                // fails the session is gone and the server rolls back itself.
                let _ = tx.rollback().await;
                Err(error)
            }
        }
    }

    async fn unlock(&mut self) -> Result<(), StoreError> {
        self.client()
            .execute(
                "SELECT pg_advisory_unlock($1, hashtext(current_schema()))",
                &[&MIGRATION_LOCK_CLASS],
            )
            .await
            .map(|_| ())
            .map_err(|e| sql_error("unlock", &e))
    }
}
