//! Test harness: one freshly created schema per test, dropped when the test
//! ends, with an explicit `SKIPPED` line when no database is configured.
//!
//! Always compiled (a cargo feature would let `cargo test --workspace` skip
//! building the tests without saying so), so later crates reuse it as a
//! dev-dependency.

use std::io::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};

use thiserror::Error;

use crate::config::{DATABASE_URL_VAR, StoreConfig};
use crate::connect::{Store, connect};
use crate::error::{ConfigError, ConnectError};

const SCHEMA_PREFIX: &str = "dg_test_";
const MAX_NAME_LEN: usize = 63;
const MAX_TEST_PART_LEN: usize = 24;

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// The harness could not give a test its schema. A test that gets this fails:
/// it never skips.
#[derive(Debug, Error)]
pub enum HarnessError {
    /// The variable is set but unusable (for instance empty).
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// The variable is set but the database cannot be reached.
    #[error(transparent)]
    Connect(#[from] ConnectError),
    /// `CREATE SCHEMA` failed.
    #[error("cannot create the test schema (SQLSTATE {})", sqlstate.as_deref().unwrap_or("unknown"))]
    CreateSchema {
        /// SQLSTATE code, when the failure carried one.
        sqlstate: Option<String>,
    },
}

/// A schema created for one test and dropped (with cascade) on drop, panic
/// included. It only ever drops the schema it created itself.
#[derive(Debug)]
pub struct TestSchema {
    name: String,
    base: StoreConfig,
}

impl TestSchema {
    /// Creates the schema, or writes one `SKIPPED` line to stderr and returns
    /// `Ok(None)` when `DATABASE_URL` is absent.
    ///
    /// The line goes straight to stderr, outside libtest's output capture, so
    /// it shows without `--nocapture`; one line per skipped test makes skips
    /// countable.
    ///
    /// # Errors
    ///
    /// [`HarnessError`] when the variable is set but empty, unparseable or
    /// unreachable, or when the schema cannot be created.
    pub async fn acquire(test_name: &str) -> Result<Option<Self>, HarnessError> {
        let Some(base) = base_config_or_skip(test_name)? else {
            return Ok(None);
        };
        let name = schema_name(test_name);
        let store = connect(&base).await?;
        // No IF NOT EXISTS: an existing name fails, so the harness only ever
        // drops what it created.
        store
            .client()
            .batch_execute(&format!("CREATE SCHEMA \"{name}\""))
            .await
            .map_err(|error| HarnessError::CreateSchema {
                sqlstate: error.code().map(|code| code.code().to_owned()),
            })?;
        Ok(Some(Self { name, base }))
    }

    /// The schema's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// A config whose `search_path` is this schema.
    #[must_use]
    pub fn config(&self) -> StoreConfig {
        self.base.with_search_path(&self.name)
    }

    /// Opens a connection whose `search_path` is this schema.
    ///
    /// # Errors
    ///
    /// [`ConnectError`] as for [`connect`].
    pub async fn connect(&self) -> Result<Store, ConnectError> {
        connect(&self.config()).await
    }
}

impl Drop for TestSchema {
    fn drop(&mut self) {
        // Async drop is impossible and a nested runtime on the test thread
        // panics, so the drop runs on its own thread with its own runtime.
        let name = self.name.clone();
        let base = self.base.clone();
        let dropped = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .ok()?;
            runtime.block_on(async {
                let store = connect(&base).await.ok()?;
                store
                    .client()
                    .batch_execute(&format!(
                        "SET lock_timeout = '10s'; DROP SCHEMA IF EXISTS \"{name}\" CASCADE"
                    ))
                    .await
                    .ok()
            })
        })
        .join();
        if !matches!(dropped, Ok(Some(()))) {
            emit(&format!("LEAKED SCHEMA {}", self.name));
        }
    }
}

/// Writes `line` to stderr in one call, so concurrent tests never tear it.
fn emit(line: &str) {
    let _ = std::io::stderr().write_all(format!("{line}\n").as_bytes());
}

/// The base config, or `Ok(None)` after writing the `SKIPPED` line.
fn base_config_or_skip(test_name: &str) -> Result<Option<StoreConfig>, ConfigError> {
    let value = std::env::var_os(DATABASE_URL_VAR);
    if value.is_none() {
        emit(&format!(
            "SKIPPED {test_name}: {DATABASE_URL_VAR} is not set; this test needs a real PostgreSQL"
        ));
        return Ok(None);
    }
    StoreConfig::from_value(value).map(Some)
}

/// In-crate tests that need a database but no schema. A set-but-broken
/// variable fails the test.
#[cfg(test)]
pub(crate) fn base_config_or_skip_for_test(test_name: &str) -> Option<StoreConfig> {
    base_config_or_skip(test_name).expect("DATABASE_URL is set but unusable")
}

/// `dg_test_<sanitized test name>_<pid>_<counter>`, `[a-z0-9_]`, at most 63
/// bytes: safe to double-quote and unique per process and per call.
fn schema_name(test_name: &str) -> String {
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let suffix = format!("_{}_{counter}", std::process::id());
    let budget = (MAX_NAME_LEN - SCHEMA_PREFIX.len() - suffix.len()).min(MAX_TEST_PART_LEN);
    let sanitized: String = test_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .take(budget)
        .collect();
    format!("{SCHEMA_PREFIX}{sanitized}{suffix}")
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::ffi::OsString;

    use super::*;
    use crate::migrate::migrate;

    #[test]
    fn names_are_sanitised_short_prefixed_and_unique() {
        let mut seen = HashSet::new();
        let long = "Weird Name/with::chars-é-and-a-very-long-tail-that-never-ends".repeat(3);
        for index in 0..1000 {
            let given = if index % 2 == 0 {
                "plain_test"
            } else {
                long.as_str()
            };
            let name = schema_name(given);
            assert!(name.starts_with(SCHEMA_PREFIX), "{name}");
            assert!(name.len() <= MAX_NAME_LEN, "{name}");
            assert!(
                name.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
                "{name}"
            );
            assert!(seen.insert(name));
        }
    }

    #[test]
    fn an_empty_variable_is_an_error_never_a_skip() {
        // The skip decision is "absent only": empty goes through the parser.
        let empty = StoreConfig::from_value(Some(OsString::new()));
        assert!(matches!(empty, Err(ConfigError::Missing { .. })));
    }

    /// Rule 19: a role that cannot create objects fails typed and leaves no
    /// partial state. Reads the read-only variable here and nowhere else.
    #[tokio::test]
    async fn readonly_role_fails_typed() {
        const TEST: &str = "readonly_role_fails_typed";
        if base_config_or_skip(TEST).expect("harness").is_none() {
            return;
        }
        let Some(value) = std::env::var_os("DATABASE_URL_READONLY") else {
            emit(&format!(
                "SKIPPED {TEST}: DATABASE_URL_READONLY is not set; this test needs the read-only role"
            ));
            return;
        };
        let schema = TestSchema::acquire(TEST)
            .await
            .expect("harness")
            .expect("database configured");
        let config = StoreConfig::from_value(Some(value))
            .expect("DATABASE_URL_READONLY is set but unusable")
            .with_search_path(schema.name());
        let mut readonly = connect(&config).await.expect("read-only connection");
        let error = migrate(&mut readonly, &[]).await.unwrap_err();
        // A schema without USAGE for the role is skipped by current_schema().
        assert!(
            matches!(
                error,
                crate::MigrateError::NoTargetSchema | crate::MigrateError::InsufficientPrivilege
            ),
            "{error}"
        );
        let admin = schema.connect().await.expect("owner connection");
        let tables = admin
            .client()
            .query_one(
                "SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname = $1",
                &[&schema.name()],
            )
            .await
            .expect("count");
        assert_eq!(tables.get::<_, i64>(0), 0);
    }
}
