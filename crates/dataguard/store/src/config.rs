//! The connection configuration, read from the one environment variable.

use std::ffi::OsString;
use std::fmt;
use std::time::Duration;

use crate::error::ConfigError;

/// Name of the only environment variable the shipped code reads.
pub const DATABASE_URL_VAR: &str = "DATABASE_URL";

/// Default bound on opening a connection.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// A parsed connection address. Its `Debug` never shows the value.
#[derive(Clone)]
pub struct StoreConfig {
    pub(crate) inner: tokio_postgres::Config,
    pub(crate) timeout: Duration,
}

impl StoreConfig {
    /// Reads and parses `DATABASE_URL`. No other variable is read and there is
    /// no default.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Missing`] when the variable is absent, empty or
    /// whitespace-only, [`ConfigError::NotUnicode`] when it is not UTF-8,
    /// [`ConfigError::Unparseable`] when it is not a connection address.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_value(std::env::var_os(DATABASE_URL_VAR))
    }

    /// Parses an already-read value. Tests go through here: setting the
    /// process environment is `unsafe` in edition 2024.
    pub(crate) fn from_value(value: Option<OsString>) -> Result<Self, ConfigError> {
        let var = DATABASE_URL_VAR;
        let text = value
            .ok_or(ConfigError::Missing { var })?
            .into_string()
            .map_err(|_| ConfigError::NotUnicode { var })?;
        if text.trim().is_empty() {
            return Err(ConfigError::Missing { var });
        }
        // The driver's parse error is dropped: it can quote the address.
        let mut inner = text
            .parse::<tokio_postgres::Config>()
            .map_err(|_| ConfigError::Unparseable { var })?;
        inner.connect_timeout(DEFAULT_CONNECT_TIMEOUT);
        Ok(Self {
            inner,
            timeout: DEFAULT_CONNECT_TIMEOUT,
        })
    }

    /// Points the session at `schema` through the connection options, after
    /// any options the address already carries. The
    /// runner never sets `search_path` itself; only the test harness does.
    /// `schema` must be a plain `[a-z0-9_]` identifier.
    pub(crate) fn with_search_path(&self, schema: &str) -> Self {
        let mut next = self.clone();
        let options = match self.inner.get_options() {
            Some(existing) => format!("{existing} -c search_path={schema}"),
            None => format!("-c search_path={schema}"),
        };
        next.inner.options(options);
        next
    }

    /// Replaces the connection bound. Only the unit tests shorten it.
    #[cfg(test)]
    pub(crate) fn with_connect_timeout(&self, timeout: Duration) -> Self {
        let mut next = self.clone();
        next.inner.connect_timeout(timeout);
        next.timeout = timeout;
        next
    }
}

impl fmt::Debug for StoreConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoreConfig")
            .field(DATABASE_URL_VAR, &format_args!("<redacted>"))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANARY: &str = "CANARY-not-a-dsn-7f3a";

    fn var() -> &'static str {
        DATABASE_URL_VAR
    }

    #[test]
    fn absent_empty_and_blank_are_missing_and_name_the_variable() {
        for value in [None, Some(OsString::new()), Some(OsString::from(" \t\n"))] {
            let err = StoreConfig::from_value(value).unwrap_err();
            assert_eq!(err, ConfigError::Missing { var: var() });
            assert!(err.to_string().contains(var()));
        }
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_is_not_unicode() {
        use std::os::unix::ffi::OsStringExt;
        let value = OsString::from_vec(vec![0x66, 0xff, 0xfe]);
        let err = StoreConfig::from_value(Some(value)).unwrap_err();
        assert_eq!(err, ConfigError::NotUnicode { var: var() });
        assert!(err.to_string().contains(var()));
    }

    #[test]
    fn garbage_is_unparseable_and_never_echoed() {
        let err = StoreConfig::from_value(Some(OsString::from(CANARY))).unwrap_err();
        assert_eq!(err, ConfigError::Unparseable { var: var() });
        for rendering in [err.to_string(), format!("{err:?}"), format!("{err:#?}")] {
            assert!(!rendering.contains("CANARY"), "{rendering}");
            assert!(!rendering.contains("7f3a"), "{rendering}");
        }
        assert!(err.to_string().contains(var()));
    }

    #[test]
    fn debug_of_a_valid_config_is_redacted() {
        let mut inner = tokio_postgres::Config::new();
        inner
            .host("CANARY-host")
            .user("CANARY-user")
            .password("CANARY-password")
            .dbname("CANARY-db");
        let config = StoreConfig {
            inner,
            timeout: DEFAULT_CONNECT_TIMEOUT,
        };
        for rendering in [format!("{config:?}"), format!("{config:#?}")] {
            assert!(!rendering.contains("CANARY"), "{rendering}");
            assert!(rendering.contains("<redacted>"));
            assert!(rendering.contains(var()));
        }
    }

    #[test]
    fn a_parsable_value_builds_a_config_with_the_default_bound() {
        let config = StoreConfig::from_value(Some(OsString::from("user=u dbname=d"))).unwrap();
        assert_eq!(config.timeout, DEFAULT_CONNECT_TIMEOUT);
    }
}
