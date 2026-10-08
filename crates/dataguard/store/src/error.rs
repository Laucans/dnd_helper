//! Typed errors of the store.
//!
//! None of them carries a driver error, a connection string or any fragment of
//! one: a conflict between a useful message and secrecy is settled for
//! secrecy. Connection failures keep a category, database failures keep the
//! SQLSTATE code only.

use std::fmt;

use thiserror::Error;

/// The connection variable is unusable; no connection was attempted.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ConfigError {
    /// The variable is absent, empty or whitespace-only.
    #[error("{var} is not set or is empty")]
    Missing {
        /// Name of the variable.
        var: &'static str,
    },
    /// The value is not valid UTF-8.
    #[error("{var} does not hold valid UTF-8")]
    NotUnicode {
        /// Name of the variable.
        var: &'static str,
    },
    /// The value cannot be parsed as a connection address.
    #[error("{var} does not hold a valid connection address")]
    Unparseable {
        /// Name of the variable.
        var: &'static str,
    },
}

/// Why a connection to the database failed, as a category only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectFailure {
    /// No server answered: refused connection, DNS failure, unreachable host.
    Unreachable,
    /// The server refused the credentials.
    Authentication,
    /// The connection took longer than the bound.
    Timeout,
    /// The server (or the driver) refused the session for another reason.
    Rejected,
}

impl fmt::Display for ConnectFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unreachable => "database unreachable",
            Self::Authentication => "authentication failed",
            Self::Timeout => "connection timed out",
            Self::Rejected => "connection rejected",
        })
    }
}

/// Opening the connection failed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ConnectError {
    /// The variable is unusable.
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// The variable parsed but the database did not accept the connection.
    #[error("cannot connect with {var}: {failure}")]
    Failed {
        /// Name of the variable.
        var: &'static str,
        /// Category of the failure.
        failure: ConnectFailure,
    },
}

/// The migration runner stopped.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MigrateError {
    /// Two migrations of the list share a number.
    #[error("migration number {number} is duplicated")]
    Duplicate {
        /// The repeated number.
        number: u32,
    },
    /// The list is not numbered `1..=N` in order.
    #[error("migration numbers must run 1..=N without a gap: expected {expected}, found {found}")]
    Gap {
        /// The number the list should hold at this position.
        expected: u32,
        /// The number it holds.
        found: u32,
    },
    /// An applied migration's checksum differs from the one in the code.
    #[error("migration {number} changed after it was applied (checksum mismatch)")]
    ChecksumMismatch {
        /// Number of the drifted migration.
        number: u32,
    },
    /// The database records a migration the code does not know.
    #[error("the database records migration {number}, unknown to this binary")]
    UnknownApplied {
        /// Number of the unknown migration.
        number: u32,
    },
    /// The recorded numbers are not `1..=K`.
    #[error("the recorded migrations are out of sequence at number {number}")]
    AppliedOutOfSequence {
        /// First recorded number that breaks the sequence.
        number: u32,
    },
    /// The connection's `search_path` resolves to no usable schema.
    #[error("the connection resolves to no schema the role can use")]
    NoTargetSchema,
    /// The role may not create objects in the target schema.
    #[error("the role may not create objects in the target schema")]
    InsufficientPrivilege,
    /// A migration's SQL failed; nothing of it persists.
    #[error("migration {number} failed (SQLSTATE {sqlstate}); nothing of it was kept")]
    MigrationFailed {
        /// Number of the failing migration.
        number: u32,
        /// SQLSTATE code, or `unknown` when the failure carried none.
        sqlstate: String,
    },
    /// A bookkeeping or lock statement failed.
    #[error("database error during {step} (SQLSTATE {})", sqlstate.as_deref().unwrap_or("unknown"))]
    Database {
        /// The runner step that failed.
        step: &'static str,
        /// SQLSTATE code, when the failure carried one.
        sqlstate: Option<String>,
    },
}
