//! Opening a connection, bounded in time and with secret-free errors.

use std::fmt;

use tokio::task::JoinHandle;
use tokio_postgres::NoTls;
use tokio_postgres::error::SqlState;

use crate::config::{DATABASE_URL_VAR, StoreConfig};
use crate::error::{ConnectError, ConnectFailure};

/// An open connection to the database. Dropping it aborts the driver task and
/// closes the session.
pub struct Store {
    client: tokio_postgres::Client,
    driver: JoinHandle<()>,
}

impl Store {
    /// The underlying driver client, for statements the store does not wrap.
    #[must_use]
    pub const fn client(&self) -> &tokio_postgres::Client {
        &self.client
    }

    pub(crate) const fn client_mut(&mut self) -> &mut tokio_postgres::Client {
        &mut self.client
    }
}

impl fmt::Debug for Store {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Store").finish_non_exhaustive()
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        self.driver.abort();
    }
}

/// Opens a connection to the database `config` points at, within the bound
/// the config carries (5 s by default). TLS is not used: the store is local.
///
/// # Errors
///
/// [`ConnectError::Failed`] naming `DATABASE_URL` and one of four categories.
/// The driver's own text is dropped: it may carry the address.
pub async fn connect(config: &StoreConfig) -> Result<Store, ConnectError> {
    // The outer timeout also covers a server that accepts TCP and then stays
    // silent, which the driver's own bound does not.
    let attempt = tokio::time::timeout(config.timeout, config.inner.connect(NoTls)).await;
    let failed = |failure| ConnectError::Failed {
        var: DATABASE_URL_VAR,
        failure,
    };
    match attempt {
        Err(_) => Err(failed(ConnectFailure::Timeout)),
        Ok(Err(error)) => Err(failed(classify(&error))),
        Ok(Ok((client, connection))) => {
            // The result is discarded unlogged: it may carry server text.
            let driver = tokio::spawn(async move {
                let _ = connection.await;
            });
            Ok(Store { client, driver })
        }
    }
}

fn classify(error: &tokio_postgres::Error) -> ConnectFailure {
    if let Some(code) = error.code() {
        return if *code == SqlState::INVALID_PASSWORD
            || *code == SqlState::INVALID_AUTHORIZATION_SPECIFICATION
        {
            ConnectFailure::Authentication
        } else {
            ConnectFailure::Rejected
        };
    }
    let io =
        std::error::Error::source(error).and_then(|source| source.downcast_ref::<std::io::Error>());
    match io {
        Some(io) if io.kind() == std::io::ErrorKind::TimedOut => ConnectFailure::Timeout,
        Some(_) => ConnectFailure::Unreachable,
        None => ConnectFailure::Rejected,
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::config::DEFAULT_CONNECT_TIMEOUT;

    const CANARY_USER: &str = "CANARY-user-91c2";
    const CANARY_PASSWORD: &str = "CANARY-password-91c2";
    const CANARY_DB: &str = "CANARY-db-91c2";

    fn config_on(port: u16) -> StoreConfig {
        let mut inner = tokio_postgres::Config::new();
        inner
            .host("127.0.0.1")
            .port(port)
            .user(CANARY_USER)
            .password(CANARY_PASSWORD)
            .dbname(CANARY_DB);
        StoreConfig {
            inner,
            timeout: DEFAULT_CONNECT_TIMEOUT,
        }
    }

    /// Every rendering of the error, including its whole `source()` chain.
    fn renderings(error: &ConnectError) -> Vec<String> {
        let mut out = vec![
            error.to_string(),
            format!("{error:?}"),
            format!("{error:#?}"),
        ];
        let mut source = error.source();
        while let Some(inner) = source {
            out.push(inner.to_string());
            out.push(format!("{inner:?}"));
            source = inner.source();
        }
        out
    }

    fn assert_redacted(error: &ConnectError) {
        assert!(error.to_string().contains(DATABASE_URL_VAR));
        for rendering in renderings(error) {
            assert!(!rendering.contains("CANARY"), "{rendering}");
            assert!(!rendering.contains("91c2"), "{rendering}");
            assert!(!rendering.contains("127.0.0.1"), "{rendering}");
        }
    }

    #[tokio::test]
    async fn a_closed_port_is_unreachable() {
        let port = {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            listener.local_addr().unwrap().port()
        };
        let error = connect(&config_on(port)).await.unwrap_err();
        assert_eq!(
            error,
            ConnectError::Failed {
                var: DATABASE_URL_VAR,
                failure: ConnectFailure::Unreachable
            }
        );
        assert_redacted(&error);
    }

    #[tokio::test]
    async fn a_silent_server_times_out_within_the_bound() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let config = config_on(port).with_connect_timeout(Duration::from_millis(200));
        let started = Instant::now();
        let error = connect(&config).await.unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(
            error,
            ConnectError::Failed {
                var: DATABASE_URL_VAR,
                failure: ConnectFailure::Timeout
            }
        );
        assert_redacted(&error);
        drop(listener);
    }

    #[tokio::test]
    async fn auth_failure_is_redacted() {
        let Some(mut config) =
            crate::testing::base_config_or_skip_for_test("auth_failure_is_redacted")
        else {
            return;
        };
        // A trust-auth server ignores the password but still refuses an unknown
        // role with the same SQLSTATE class, so both are replaced.
        config.inner.user(CANARY_USER).password(CANARY_PASSWORD);
        let error = connect(&config).await.unwrap_err();
        assert_eq!(
            error,
            ConnectError::Failed {
                var: DATABASE_URL_VAR,
                failure: ConnectFailure::Authentication
            }
        );
        for rendering in renderings(&error) {
            assert!(!rendering.contains("CANARY"), "{rendering}");
        }
    }
}
