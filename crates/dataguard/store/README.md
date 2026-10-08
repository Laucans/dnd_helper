# dataguard-store

Store plumbing of the Data layer: the connection, the migration runner, the
test schema harness, and the initial schema (migration 1).

## Connection

One environment variable, `DATABASE_URL`, and nothing else: no default, no
other variable, no command-line argument. Export it from your local,
uncommitted environment file. Its value never appears in an error, a debug
output or a log line.

The connection is plain TCP, with no TLS: the store is local. An address that
demands TLS fails as a rejected connection.

A failure (variable absent, empty, not UTF-8, not a connection address,
database unreachable, authentication refused, timeout after 5 s) is a typed
error that names `DATABASE_URL` and carries no part of its value.

## Migrate command

```sh
cargo run --locked -p dataguard-store --bin dataguard-migrate
```

- Prints `applied <n> migration(s); schema at number <k>`.
- Exit 0 on success and on replay (`applied 0 ...`), 1 on any error, 2 on an
  unexpected argument.
- Objects land in the schema the connection resolves to; the runner never sets
  or qualifies a schema.
- Forward-only. A migration commits together with its bookkeeping row; a
  failing one leaves nothing and is retried from the start by the next run.
- Refuses to run when an applied migration's checksum changed, when the
  database records a number the binary does not know, or when the recorded
  numbers are not `1..=K`.
- Two runners at once: one waits for the other (advisory lock per schema).

## Schema

Migration `0001_initial_schema.sql` brings an empty schema to this, and
nothing else exists (a test compares the table list):

| table | holds |
|---|---|
| `data_queue` | one row per command: the fields of contract G (snake_case) plus `payload`, `idempotency_key`, `data_version`, `violations`, `created_at` |
| `data_version_counter` | one row, `value bigint` starting at 0; a table and not a sequence, so a rollback leaves no gap |
| `audit_log` | one row per event: `enqueued`, `applied`, `rejected`, `cancelled`, `refused` |
| `data_capability_registry` | DataCapability versions (contract F), keyed by `(name, version)`, plus the full `manifest` |
| `schema_migrations` | the runner's bookkeeping |

The migration creates no role. It needs `dnd_app` and `dnd_readonly` (#73) and
fails naming the missing one. Every foreign key is `ON DELETE RESTRICT ON
UPDATE RESTRICT`. No trigger, no function, no sequence.

### Grants

PUBLIC holds nothing. Both roles have `USAGE` on the schema and nothing more.

| table | `dnd_app` | `dnd_readonly` |
|---|---|---|
| `data_queue` | `SELECT`, `INSERT`, `UPDATE` on `state`, `based_on`, `projection`, `your_value`, `confirmation`, `parked`, `requeued_from`, `data_version`, `violations` | `SELECT` |
| `data_version_counter` | `SELECT`, `UPDATE (value)` | `SELECT` |
| `audit_log` | `SELECT`, `INSERT` | `SELECT` |
| `data_capability_registry` | `SELECT`, `INSERT`, `UPDATE (ref)` | `SELECT` |
| `schema_migrations` | `SELECT`, `INSERT` | `SELECT` |

- `dnd_app` migrates, so it owns every table. Immutability binds it because the
  migration makes the owner `REVOKE` its own privileges: there is no `DELETE`,
  `TRUNCATE`, `REFERENCES` or `TRIGGER` for anyone, and no `UPDATE` on the
  columns that keep a command in its place. Owner rights no privilege removes
  (`ALTER`, `DROP`, a new `GRANT`) remain.
- `UPDATE (ref)` on the registry exists only because PostgreSQL checks a foreign
  key as the referenced table's owner with `FOR KEY SHARE`, which needs `UPDATE`
  on one column. `ref` is generated, so any value written to it fails.
- `schema_migrations` keeps `INSERT` for `dnd_app`: the runner records every
  later migration as that role.

### Left to code, not to the schema

`dnd_app` holds the column `UPDATE` these need, so they belong to the enqueue
path and the applier (#79, #80): a terminal state never moves back, and the
counter only goes from `value` to `value + 1`.

Requires PostgreSQL 13 or later (`gen_random_uuid()` without an extension).

## Adding a migration

1. Add `migrations/<nnnn>_<name>.sql`.
2. Add the entry to `MIGRATIONS` in `src/migrations.rs` with the next number
   (`1..=N`, no gap).

The SQL runs inside the runner's transaction: no `BEGIN`/`COMMIT`, no
`CONCURRENTLY`, no schema qualifier. Never edit a migration already applied.

## Tests that need the database

```rust,ignore
let Some(schema) = TestSchema::acquire("my_test").await.expect("harness") else {
    return;
};
let store = schema.connect().await.expect("connect");
```

- `acquire` creates a schema of its own, `search_path` points to it, and it is
  dropped with cascade when the test ends, panic included.
- `DATABASE_URL` unset: one `SKIPPED <test>: ...` line on stderr per test, the
  test reports `ok`. The number of `SKIPPED` lines is the number of skips.
- `DATABASE_URL` set but empty or unreachable, or schema creation fails: the
  test fails. It never skips.
- `cargo test --workspace --locked` is green with no database, and proves
  nothing about the runner then: check the `SKIPPED` count.
- The tests that need the read-only role read `DATABASE_URL_READONLY` (test
  code only, never the shipped crate) and report their own `SKIPPED` line when
  it is unset: `readonly_role_fails_typed`, and in `src/schema_tests/` the
  audit, read-only and privilege tests.
- `src/schema_tests/` tests the schema itself, one module per table, plus
  replay, atomicity of a failing migration and the privilege matrix. CI has no
  database, so only a local run with both variables set proves the schema.
