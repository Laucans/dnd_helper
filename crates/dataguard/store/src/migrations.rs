//! The shipped migration list.

use crate::migrate::Migration;

/// Every migration the binary knows, numbered `1..=N` in order.
///
/// Empty in the plumbing slice: the runner ships only its own bookkeeping
/// table, which is not a numbered migration. The Data layer schema arrives as
/// entries here, each `include_str!`-ing a file under `migrations/`.
pub const MIGRATIONS: &[Migration] = &[];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate::validate;

    #[test]
    fn the_shipped_list_is_valid() {
        assert_eq!(validate(MIGRATIONS), Ok(()));
    }
}
