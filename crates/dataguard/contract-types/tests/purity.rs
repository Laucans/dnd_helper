//! The crate's dependency closure holds no database, runtime, network or
//! capability crate (rule 1). Reads `cargo tree`, so transitive crates count.

use std::process::Command;

const FORBIDDEN: [&str; 8] = [
    "sqlx",
    "tokio",
    "postgres",
    "tokio-postgres",
    "reqwest",
    "hyper",
    "rustls",
    "rusqlite",
];

#[test]
fn dependency_tree_has_no_database_runtime_or_capability_crate() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let output = Command::new(cargo)
        .args([
            "tree",
            "-p",
            "dataguard-contract-types",
            "-e",
            "normal",
            "--prefix",
            "none",
            "--locked",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo tree must run");
    assert!(
        output.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let tree = String::from_utf8(output.stdout).expect("cargo tree prints UTF-8");
    assert!(tree.starts_with("dataguard-contract-types "), "{tree}");
    for line in tree.lines() {
        let name = line.split_whitespace().next().unwrap_or_default();
        assert!(!FORBIDDEN.contains(&name), "forbidden dependency: {line}");
        assert!(
            !line.contains("/capabilities/"),
            "depends on a Capability crate: {line}"
        );
    }
}
