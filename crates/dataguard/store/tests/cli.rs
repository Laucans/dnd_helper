//! The migrate command's failure paths. No test runs it against the real
//! database: it would migrate the role's default schema, and no test may touch
//! `public`. The success path is covered by the library tests.

use std::net::TcpListener;
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_dataguard-migrate");
const VAR: &str = "DATABASE_URL";
const CANARY: &str = "CANARY-not-a-dsn-7f3a";

fn run(value: Option<&str>, args: &[&str]) -> Output {
    let mut command = Command::new(BIN);
    command
        .args(args)
        .env_remove(VAR)
        .env_remove("DATABASE_URL_READONLY");
    if let Some(value) = value {
        command.env(VAR, value);
    }
    command.output().expect("run the binary")
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn kv(key: &str, value: &str) -> String {
    format!("{key}={value}")
}

fn assert_failed_naming_the_variable(output: &Output) {
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(VAR), "{stderr}");
    assert!(!text(output).contains("CANARY"), "{}", text(output));
    assert!(!stderr.contains("panicked"), "{stderr}");
}

#[test]
fn absent_empty_blank_and_garbage_fail_naming_the_variable() {
    for value in [None, Some(""), Some("   "), Some(CANARY)] {
        assert_failed_naming_the_variable(&run(value, &[]));
    }
}

#[test]
fn an_unreachable_database_fails_naming_the_variable_only() {
    let port = {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
        listener.local_addr().expect("addr").port()
    };
    let value = [
        kv("host", "127.0.0.1"),
        kv("port", &port.to_string()),
        kv("user", "CANARY_user"),
        kv("password", "CANARY_password"),
        kv("dbname", "CANARY_db"),
    ]
    .join(" ");
    let output = run(Some(&value), &[]);
    assert_failed_naming_the_variable(&output);
    assert!(!text(&output).contains("127.0.0.1"));
}

#[test]
fn any_argument_is_a_usage_error() {
    let output = run(Some(CANARY), &["--url", CANARY]);
    assert_eq!(output.status.code(), Some(2));
    assert!(!text(&output).contains("CANARY"));
}
