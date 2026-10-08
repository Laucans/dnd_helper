//! The forward-only migration runner.
//!
//! The runner is generic over a [`MigrationStore`] so its logic (numbering,
//! drift, error mapping) is tested against an in-memory fake everywhere, while
//! what depends on PostgreSQL (transactions, locking) is tested only against a
//! real server (`pg.rs`, `tests/migrate_pg.rs`).

use std::fmt::Write as _;

use sha2::{Digest, Sha256};

use crate::connect::Store;
use crate::error::MigrateError;

/// One numbered migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Migration {
    /// Position in the list: `1..=N`, no gap, no duplicate.
    pub number: u32,
    /// Human-readable name, recorded but not part of the checksum.
    pub name: &'static str,
    /// The SQL. It runs inside the runner's transaction, so it must hold no
    /// transaction control and no `CONCURRENTLY`, and never qualify a schema.
    pub sql: &'static str,
}

/// What a run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrateReport {
    /// Migrations applied by this run.
    pub applied: u32,
    /// Number of the last migration recorded after the run (0 when none).
    pub current: u32,
}

/// A migration as recorded in the database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AppliedMigration {
    pub number: u32,
    pub checksum: String,
}

/// A failure of the store side, before it is mapped to a [`MigrateError`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StoreError {
    /// `current_schema()` is NULL.
    NoTargetSchema,
    /// A statement failed.
    Sql {
        step: &'static str,
        sqlstate: Option<String>,
    },
}

/// SQLSTATE `insufficient_privilege`.
const INSUFFICIENT_PRIVILEGE: &str = "42501";

/// The database side of the runner.
pub(crate) trait MigrationStore {
    /// Guards against a NULL target schema, takes the per-schema lock, then
    /// creates the bookkeeping table if absent. The order is load-bearing.
    async fn lock_and_prepare(&mut self) -> Result<(), StoreError>;
    /// The recorded migrations, ascending by number.
    async fn applied(&mut self) -> Result<Vec<AppliedMigration>, StoreError>;
    /// Runs the migration and records it in one transaction.
    async fn apply(&mut self, migration: &Migration, checksum: &str) -> Result<(), StoreError>;
    /// Releases the lock.
    async fn unlock(&mut self) -> Result<(), StoreError>;
}

/// Checks the list is exactly `1..=N` in order.
///
/// # Errors
///
/// [`MigrateError::Duplicate`] when a number repeats, [`MigrateError::Gap`]
/// when the list skips, starts elsewhere than 1 or is out of order.
pub fn validate(migrations: &[Migration]) -> Result<(), MigrateError> {
    for (index, (expected, migration)) in (1_u32..).zip(migrations).enumerate() {
        if migration.number == expected {
            continue;
        }
        let repeated = migrations
            .iter()
            .take(index)
            .any(|earlier| earlier.number == migration.number);
        return Err(if repeated {
            MigrateError::Duplicate {
                number: migration.number,
            }
        } else {
            MigrateError::Gap {
                expected,
                found: migration.number,
            }
        });
    }
    Ok(())
}

/// SHA-256 of the SQL text, lowercase hex.
#[must_use]
pub fn checksum(sql: &str) -> String {
    Sha256::digest(sql.as_bytes())
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

/// Brings the database the store points at to the last migration of the list.
/// Replaying changes nothing and reports `applied == 0`.
///
/// # Errors
///
/// A [`MigrateError`]: invalid list (before any statement), drift, missing
/// privilege, or a failing migration (rolled back, naming its number).
pub async fn migrate(
    store: &mut Store,
    migrations: &[Migration],
) -> Result<MigrateReport, MigrateError> {
    run(store, migrations).await
}

pub(crate) async fn run<S: MigrationStore>(
    store: &mut S,
    migrations: &[Migration],
) -> Result<MigrateReport, MigrateError> {
    validate(migrations)?;
    let outcome = locked(store, migrations).await;
    // Dropping the connection releases the lock anyway, so the unlock result
    // never changes the outcome: after a successful run everything is already
    // committed, and on the error path it adds nothing.
    let _ = store.unlock().await;
    outcome
}

async fn locked<S: MigrationStore>(
    store: &mut S,
    migrations: &[Migration],
) -> Result<MigrateReport, MigrateError> {
    store
        .lock_and_prepare()
        .await
        .map_err(|error| match error {
            StoreError::Sql {
                sqlstate: Some(code),
                ..
            } if code == INSUFFICIENT_PRIVILEGE => MigrateError::InsufficientPrivilege,
            other => map_store(other),
        })?;
    let applied = store.applied().await.map_err(map_store)?;
    check_drift(&applied, migrations)?;

    let mut count = 0_u32;
    for migration in migrations.iter().skip(applied.len()) {
        let sum = checksum(migration.sql);
        store.apply(migration, &sum).await.map_err(|error| {
            let sqlstate = match error {
                StoreError::Sql { sqlstate, .. } => sqlstate,
                StoreError::NoTargetSchema => None,
            };
            MigrateError::MigrationFailed {
                number: migration.number,
                sqlstate: sqlstate.unwrap_or_else(|| "unknown".to_owned()),
            }
        })?;
        count += 1;
    }
    Ok(MigrateReport {
        applied: count,
        current: u32::try_from(migrations.len()).unwrap_or(u32::MAX),
    })
}

fn map_store(error: StoreError) -> MigrateError {
    match error {
        StoreError::NoTargetSchema => MigrateError::NoTargetSchema,
        StoreError::Sql { step, sqlstate } => MigrateError::Database { step, sqlstate },
    }
}

/// Drift is refused before anything is applied, and wins over "replay is a
/// no-op".
fn check_drift(applied: &[AppliedMigration], migrations: &[Migration]) -> Result<(), MigrateError> {
    for (expected, row) in (1_u32..).zip(applied) {
        if row.number != expected {
            return Err(MigrateError::AppliedOutOfSequence { number: row.number });
        }
    }
    for row in applied {
        let known = usize::try_from(row.number)
            .ok()
            .and_then(|number| number.checked_sub(1))
            .and_then(|index| migrations.get(index));
        let Some(migration) = known else {
            return Err(MigrateError::UnknownApplied { number: row.number });
        };
        if checksum(migration.sql) != row.checksum {
            return Err(MigrateError::ChecksumMismatch { number: row.number });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    const fn m(number: u32, sql: &'static str) -> Migration {
        Migration {
            number,
            name: "fixture",
            sql,
        }
    }

    /// In-memory stand-in for the database side.
    #[derive(Default)]
    struct FakeStore {
        rows: Vec<AppliedMigration>,
        calls: Vec<&'static str>,
        applied_order: Vec<u32>,
        prepare_error: Option<StoreError>,
        fail_apply_at: Option<(u32, Option<String>)>,
    }

    impl FakeStore {
        fn with_rows(rows: &[(u32, &str)]) -> Self {
            Self {
                rows: rows
                    .iter()
                    .map(|(number, sql)| AppliedMigration {
                        number: *number,
                        checksum: checksum(sql),
                    })
                    .collect(),
                ..Self::default()
            }
        }
    }

    impl MigrationStore for FakeStore {
        async fn lock_and_prepare(&mut self) -> Result<(), StoreError> {
            self.calls.push("lock_and_prepare");
            self.prepare_error.clone().map_or(Ok(()), Err)
        }

        async fn applied(&mut self) -> Result<Vec<AppliedMigration>, StoreError> {
            self.calls.push("applied");
            Ok(self.rows.clone())
        }

        async fn apply(&mut self, migration: &Migration, checksum: &str) -> Result<(), StoreError> {
            self.calls.push("apply");
            if let Some((at, sqlstate)) = &self.fail_apply_at {
                if *at == migration.number {
                    return Err(StoreError::Sql {
                        step: "apply",
                        sqlstate: sqlstate.clone(),
                    });
                }
            }
            self.applied_order.push(migration.number);
            self.rows.push(AppliedMigration {
                number: migration.number,
                checksum: checksum.to_owned(),
            });
            Ok(())
        }

        async fn unlock(&mut self) -> Result<(), StoreError> {
            self.calls.push("unlock");
            Ok(())
        }
    }

    #[tokio::test]
    async fn empty_list_on_empty_store_applies_nothing() {
        let mut store = FakeStore::default();
        let report = run(&mut store, &[]).await.unwrap();
        assert_eq!(
            report,
            MigrateReport {
                applied: 0,
                current: 0
            }
        );
        assert_eq!(store.calls, ["lock_and_prepare", "applied", "unlock"]);
    }

    #[tokio::test]
    async fn applies_in_order_then_replay_changes_nothing() {
        let list = [m(1, "a"), m(2, "b")];
        let mut store = FakeStore::default();
        let first = run(&mut store, &list).await.unwrap();
        assert_eq!(
            first,
            MigrateReport {
                applied: 2,
                current: 2
            }
        );
        assert_eq!(store.applied_order, [1, 2]);

        let rows = store.rows.clone();
        let replay = run(&mut store, &list).await.unwrap();
        assert_eq!(
            replay,
            MigrateReport {
                applied: 0,
                current: 2
            }
        );
        assert_eq!(store.rows, rows);
        assert_eq!(store.applied_order, [1, 2]);
    }

    #[tokio::test]
    async fn a_database_at_k_applies_only_the_rest() {
        let list = [m(1, "a"), m(2, "b"), m(3, "c")];
        let mut store = FakeStore::with_rows(&[(1, "a")]);
        let report = run(&mut store, &list).await.unwrap();
        assert_eq!(
            report,
            MigrateReport {
                applied: 2,
                current: 3
            }
        );
        assert_eq!(store.applied_order, [2, 3]);
    }

    #[tokio::test]
    async fn an_invalid_list_fails_before_any_store_call() {
        let lists: [(&[Migration], MigrateError); 3] = [
            (
                &[m(1, "a"), m(1, "b")],
                MigrateError::Duplicate { number: 1 },
            ),
            (
                &[m(1, "a"), m(3, "b")],
                MigrateError::Gap {
                    expected: 2,
                    found: 3,
                },
            ),
            (
                &[m(2, "a")],
                MigrateError::Gap {
                    expected: 1,
                    found: 2,
                },
            ),
        ];
        for (list, expected) in lists {
            let mut store = FakeStore::default();
            assert_eq!(run(&mut store, list).await.unwrap_err(), expected);
            assert!(store.calls.is_empty());
        }
    }

    #[tokio::test]
    async fn changed_sql_is_refused_before_anything_is_applied() {
        let mut store = FakeStore::with_rows(&[(1, "old")]);
        let list = [m(1, "new"), m(2, "b")];
        let error = run(&mut store, &list).await.unwrap_err();
        assert_eq!(error, MigrateError::ChecksumMismatch { number: 1 });
        assert!(store.applied_order.is_empty());
        assert!(!store.calls.contains(&"apply"));
    }

    #[tokio::test]
    async fn a_recorded_number_the_code_lacks_is_refused() {
        let mut store = FakeStore::with_rows(&[(1, "a"), (2, "b"), (3, "c")]);
        let error = run(&mut store, &[m(1, "a"), m(2, "b")]).await.unwrap_err();
        assert_eq!(error, MigrateError::UnknownApplied { number: 3 });
    }

    #[tokio::test]
    async fn recorded_numbers_with_a_hole_are_out_of_sequence() {
        let mut store = FakeStore::with_rows(&[(1, "a"), (3, "c")]);
        let list = [m(1, "a"), m(2, "b"), m(3, "c")];
        let error = run(&mut store, &list).await.unwrap_err();
        assert_eq!(error, MigrateError::AppliedOutOfSequence { number: 3 });
    }

    #[tokio::test]
    async fn a_failing_migration_stops_the_run_and_unlocks() {
        let mut store = FakeStore {
            fail_apply_at: Some((2, Some("42601".to_owned()))),
            ..FakeStore::default()
        };
        let list = [m(1, "a"), m(2, "b"), m(3, "c")];
        let error = run(&mut store, &list).await.unwrap_err();
        assert_eq!(
            error,
            MigrateError::MigrationFailed {
                number: 2,
                sqlstate: "42601".to_owned()
            }
        );
        assert_eq!(store.applied_order, [1]);
        assert_eq!(store.calls.last(), Some(&"unlock"));
    }

    #[tokio::test]
    async fn a_failure_without_sqlstate_still_names_the_number() {
        let mut store = FakeStore {
            fail_apply_at: Some((1, None)),
            ..FakeStore::default()
        };
        let error = run(&mut store, &[m(1, "a")]).await.unwrap_err();
        assert_eq!(
            error,
            MigrateError::MigrationFailed {
                number: 1,
                sqlstate: "unknown".to_owned()
            }
        );
    }

    #[tokio::test]
    async fn missing_privilege_and_missing_schema_are_typed() {
        let denied = StoreError::Sql {
            step: "prepare",
            sqlstate: Some("42501".to_owned()),
        };
        let mut store = FakeStore {
            prepare_error: Some(denied),
            ..FakeStore::default()
        };
        assert_eq!(
            run(&mut store, &[]).await.unwrap_err(),
            MigrateError::InsufficientPrivilege
        );
        assert_eq!(store.calls.last(), Some(&"unlock"));

        let mut store = FakeStore {
            prepare_error: Some(StoreError::NoTargetSchema),
            ..FakeStore::default()
        };
        assert_eq!(
            run(&mut store, &[]).await.unwrap_err(),
            MigrateError::NoTargetSchema
        );
    }

    #[test]
    fn checksum_is_sha256_hex() {
        assert_eq!(
            checksum(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_ne!(checksum("a"), checksum("b"));
    }

    proptest! {
        #[test]
        fn validate_accepts_exactly_one_to_n_in_order(
            numbers in proptest::collection::vec(0_u32..8, 0..8)
        ) {
            let list: Vec<Migration> = numbers.iter().map(|n| m(*n, "")).collect();
            let canonical = (1_u32..).take(numbers.len()).eq(numbers.iter().copied());
            prop_assert_eq!(validate(&list).is_ok(), canonical);
        }
    }
}
