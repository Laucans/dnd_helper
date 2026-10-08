//! Store plumbing of the Data layer: the connection, the migration runner and
//! the test schema harness. Nothing the Data layer stores ships here, only the
//! runner's own bookkeeping table.
//!
//! # Connection
//!
//! The address comes from one environment variable, [`DATABASE_URL_VAR`]
//! (`DATABASE_URL`), and from nothing else: no default, no other variable, no
//! command-line argument. Export it from your local, uncommitted environment
//! file. Its value never appears in an error, a debug output or a message.
//!
//! # Migrate command
//!
//! ```text
//! cargo run --locked -p dataguard-store --bin dataguard-migrate
//! ```
//!
//! It takes no argument. It prints `applied <n> migration(s); schema at number
//! <k>` and exits 0 on success and on replay, 1 on any error, 2 on an
//! unexpected argument. Objects land in the schema the connection resolves to.
//!
//! # Adding a migration
//!
//! Add a file under `migrations/` and an entry in [`MIGRATIONS`] with the next
//! number. The SQL runs in the runner's transaction: no `BEGIN`/`COMMIT`, no
//! `CONCURRENTLY`, no schema qualifier. Never edit an applied migration: its
//! checksum would drift and the runner refuses to go on.
//!
//! # Tests that need the database
//!
//! ```ignore
//! let Some(schema) = TestSchema::acquire("my_test").await.expect("harness") else {
//!     return;
//! };
//! let store = schema.connect().await.expect("connect");
//! ```
//!
//! With the variable unset, `acquire` writes one `SKIPPED <test>: ...` line to
//! stderr and returns `None`; the test itself reports `ok`, so the count of
//! `SKIPPED` lines is the count of skips. With the variable set but empty or
//! unreachable, or when the schema cannot be created, `acquire` returns an
//! error and the test fails.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod config;
mod connect;
mod error;
mod migrate;
mod migrations;
mod pg;
pub mod testing;

pub use config::{DATABASE_URL_VAR, DEFAULT_CONNECT_TIMEOUT, StoreConfig};
pub use connect::{Store, connect};
pub use error::{ConfigError, ConnectError, ConnectFailure, MigrateError};
pub use migrate::{MigrateReport, Migration, checksum, migrate, validate};
pub use migrations::MIGRATIONS;
