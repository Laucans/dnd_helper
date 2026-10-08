# dataguard-store

Store plumbing of the Data layer: the connection, the migration runner and the
test schema harness. It ships only the runner's own bookkeeping table
(`schema_migrations`); the queue, counter, audit and role tables arrive as
numbered migrations in a later task.

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
- One unit test, `readonly_role_fails_typed`, reads `DATABASE_URL_READONLY`
  (test code only, never the shipped crate) and reports its own `SKIPPED` line
  when it is unset.
