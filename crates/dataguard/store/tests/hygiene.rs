//! Rule 30: no URL, DSN or credential in the crate. Only the variable name
//! may appear. The patterns are assembled at run time so this file does not
//! match itself.

use std::fs;
use std::path::Path;

fn forbidden() -> [String; 3] {
    [
        format!("{}{}", "postgres", "://"),
        format!("{}{}", "postgresql", "://"),
        format!("{}{}", "pass", "word="),
    ]
}

fn scan(dir: &Path, hits: &mut Vec<String>) {
    let patterns = forbidden();
    for entry in fs::read_dir(dir).expect("read dir").flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan(&path, hits);
            continue;
        }
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        for (number, line) in content.lines().enumerate() {
            if patterns
                .iter()
                .any(|pattern| line.contains(pattern.as_str()))
            {
                hits.push(format!("{}:{}", path.display(), number + 1));
            }
        }
    }
}

#[test]
fn no_url_or_credential_appears_in_the_crate() {
    let mut hits = Vec::new();
    scan(Path::new(env!("CARGO_MANIFEST_DIR")), &mut hits);
    assert!(hits.is_empty(), "URL or credential text at: {hits:?}");
}
