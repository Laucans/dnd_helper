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
}
