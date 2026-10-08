//! The shipped migration list.

use crate::migrate::Migration;

/// The initial schema: DataQueue, `dataVersion` counter, audit log and
/// DataCapability registry, with the grants to the two roles.
pub(crate) const INITIAL_SCHEMA: &str = include_str!("../migrations/0001_initial_schema.sql");

/// Every migration the binary knows, numbered `1..=N` in order.
///
/// Entry 1 brings an empty schema to the full Data layer schema. The runner's
/// own bookkeeping table, `schema_migrations`, is not a numbered migration.
/// Each entry `include_str!`-s a file under `migrations/`.
pub const MIGRATIONS: &[Migration] = &[Migration {
    number: 1,
    name: "initial_schema",
    sql: INITIAL_SCHEMA,
}];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate::validate;

    #[test]
    fn the_shipped_list_is_valid() {
        assert_eq!(validate(MIGRATIONS), Ok(()));
    }

    /// Upper-cased words of `sql`, comments dropped. No database needed, so
    /// this runs in CI, where the schema tests are all skipped.
    fn words(sql: &str) -> Vec<String> {
        let code: String = sql
            .lines()
            .map(|line| line.split("--").next().unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n");
        code.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .filter(|word| !word.is_empty())
            .map(str::to_ascii_uppercase)
            .collect()
    }

    fn has_sequence(words: &[String], pattern: &[&str]) -> bool {
        words
            .windows(pattern.len())
            .any(|window| window.iter().zip(pattern).all(|(w, p)| w == p))
    }

    /// Rules every shipped migration keeps, read from the SQL text alone:
    /// no sequence (gapless `dataVersion`), no trigger or function, no role or
    /// credential, no hard-coded schema, explicit FK actions, and no table
    /// left readable by PUBLIC.
    #[test]
    fn every_shipped_migration_keeps_the_schema_rules() {
        for migration in MIGRATIONS {
            let words = words(migration.sql);
            let at = format!("migration {}", migration.number);

            for banned in [
                "SEQUENCE",
                "SERIAL",
                "BIGSERIAL",
                "SMALLSERIAL",
                "IDENTITY",
                "TRIGGER",
                "FUNCTION",
                "PROCEDURE",
                "PASSWORD",
            ] {
                assert!(!words.iter().any(|w| w == banned), "{at}: uses {banned}");
            }
            for banned in [["CREATE", "ROLE"], ["CREATE", "USER"], ["ALTER", "ROLE"]] {
                assert!(!has_sequence(&words, &banned), "{at}: uses {banned:?}");
            }

            // PUBLIC may only be revoked from; `public.x` would hard-code a schema.
            for (index, word) in words.iter().enumerate() {
                if word == "PUBLIC" {
                    let previous = index.checked_sub(1).and_then(|i| words.get(i));
                    assert_eq!(previous.map(String::as_str), Some("FROM"), "{at}: PUBLIC");
                }
            }

            // Architecture rule 7: every foreign key spells both actions.
            let count = |pattern: &[&str]| {
                words
                    .windows(pattern.len())
                    .filter(|w| w.iter().zip(pattern).all(|(a, b)| a == b))
                    .count()
            };
            let references = count(&["REFERENCES"]);
            assert_eq!(
                count(&["ON", "DELETE", "RESTRICT"]),
                references,
                "{at}: ON DELETE"
            );
            assert_eq!(
                count(&["ON", "UPDATE", "RESTRICT"]),
                references,
                "{at}: ON UPDATE"
            );

            // A table is created already closed to PUBLIC.
            for (index, window) in words.windows(3).enumerate() {
                if window[0] == "CREATE" && window[1] == "TABLE" {
                    let table = &window[2];
                    assert!(
                        has_sequence(&words, &["REVOKE", "ALL", "ON", table, "FROM", "PUBLIC"]),
                        "{at}: table {table} (word {index}) is never revoked from PUBLIC"
                    );
                }
            }
        }
    }
}
