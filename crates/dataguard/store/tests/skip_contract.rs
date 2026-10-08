//! The harness's skip contract (rules 26 and 27), proved end to end without a
//! database: this test binary re-runs its own ignored probe in a child process
//! with a controlled `DATABASE_URL`, and reads what the child prints.

use std::net::TcpListener;
use std::process::{Command, Output};

use dataguard_store::testing::TestSchema;

const VAR: &str = "DATABASE_URL";
const PROBE: &str = "probe_acquire";
const CANARY: &str = "CANARY-not-a-dsn-7f3a";

/// Only meant to be run by the tests below, in a child process.
#[tokio::test]
#[ignore = "run by the skip-contract tests in a child process"]
async fn probe_acquire() {
    let _schema = TestSchema::acquire(PROBE).await.expect("harness");
}

/// Runs the probe in a child with `value` as the variable (or none). The child
/// is not given `--nocapture`: the skip line must show without it.
fn run_probe(value: Option<&str>) -> Output {
    let exe = std::env::current_exe().expect("current test binary");
    let mut command = Command::new(exe);
    command
        .args(["--ignored", "--exact", PROBE])
        .env_remove(VAR)
        .env_remove("DATABASE_URL_READONLY");
    if let Some(value) = value {
        command.env(VAR, value);
    }
    command.output().expect("run the probe")
}

fn skipped_lines(output: &Output) -> Vec<String> {
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .filter(|line| line.starts_with("SKIPPED"))
        .map(str::to_owned)
        .collect()
}

fn everything(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn an_unset_variable_prints_one_skipped_line_outside_capture_and_passes() {
    let output = run_probe(None);
    assert!(output.status.success(), "{}", everything(&output));
    let lines = skipped_lines(&output);
    assert_eq!(lines.len(), 1, "{}", everything(&output));
    let line = lines.first().expect("one line");
    assert!(line.contains(PROBE), "{line}");
    assert!(line.contains(VAR), "{line}");
}

#[test]
fn a_set_but_broken_variable_fails_and_never_skips() {
    let unreachable = {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let secret = format!("{}{}", "pass", "word");
        format!("host=127.0.0.1 port={port} user={CANARY} {secret}={CANARY} dbname={CANARY}")
    };
    for value in ["", "   ", CANARY, unreachable.as_str()] {
        let output = run_probe(Some(value));
        let text = everything(&output);
        assert!(!output.status.success(), "{value:?} passed: {text}");
        assert!(
            skipped_lines(&output).is_empty(),
            "{value:?} skipped: {text}"
        );
        assert!(!text.contains("CANARY"), "{value:?} leaked: {text}");
    }
}
