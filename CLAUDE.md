<!-- harness:architecture v1 -->
## Agent-native architecture — the rules every change is checked against

This repository follows the agent-native application architecture described in
`docs/ARCHITECTURE.md`. Read it once; these are the rules a diff is reviewed
against, and the CI gates in `.github/workflows/gates.yml` enforce the ones
that can be checked mechanically.

### Stack

- **Rust** for everything but the UI: Capabilities, the Data layer, the
  DataQueue, the DataGuard, the Resolver, the DataCapabilities. Edition 2024,
  one Cargo workspace, `cargo clippy --workspace --all-targets -- -D warnings`
  and `cargo fmt --all -- --check` clean before every commit.
- **TypeScript** for the Micro-UIs and the screen composer, strict mode,
  `tsc --noEmit` and the project's lint clean before every commit. A Micro-UI
  calls Capabilities and DataCapabilities **by identifier only**: it never
  imports their code.
- **PostgreSQL** alone as the store: the DataQueue is a table, invalidations
  travel over `LISTEN/NOTIFY`, read models are views. No message broker.
- **These rules bind every new unit, even where the existing code predates
  them.** A repository that began as one TypeScript program (a plugin, a
  static app) is the Micro-UI shell; every Capability, DataCapability or
  infrastructure unit it gains is a Rust crate at its place in the layout.
  When no server runs it, the crate is compiled to WebAssembly
  (`wasm-bindgen`, `wasm-pack`) and the TypeScript side loads it. Writing the
  unit in TypeScript because that is where the code is today is a violation,
  not a pragmatic choice.

### Layout

```
apps/<system>/<micro-ui>/          one Micro-UI (TypeScript) + micro-ui.json (contract D)
apps/screens/<screen>.json         one frozen screen composition (contract E)
crates/<system>/capabilities/<name>/   one Capability crate + capability.json (contract B)
crates/<system>/queries/<name>.graphql persisted queries, registered in queries/registry.json (contract C)
crates/<system>/<name>/            one infrastructure crate of the system (an adapter, a source, the Data layer's plumbing)
crates/dataguard/                  the write side: DataQueue, DataGuard, Resolver
crates/dataguard/data-capabilities/<name>/  one DataCapability + data-capability.json (contract F)
crates/dataguard/aggregates/<Aggregate>/aggregate.json   criticality, invariants, relations (contract I)
concepts/<Concept>/v<n>/concept.json + fixtures.json      a versioned Concept (contract A)
contracts/*.schema.json            the twelve contracts, JSON Schema 2020-12
```

### Rules

1. **One unit per task.** A task builds exactly one unit of the architecture —
   the `## Architecture` section of its issue says which — and nothing beside
   it. Need a second unit? That is a second task.
2. **A Capability reads, and only reads.** It uses read-only credentials, reads
   through its persisted queries only, and never imports, invokes or
   references another Capability, a DataCapability, the DataQueue, the
   DataGuard or the Resolver. Its `Cargo.toml` depends on no other crate under
   `capabilities/` and on nothing under `crates/dataguard/`.
3. **A Capability's logic is local.** Shared *meaning* is a Concept with
   conformance fixtures, never a shared library of business code. Duplicate the
   computation; declare the Concept it implements; pass its fixtures.
4. **Every read is a persisted query**, declared in its Capability's manifest
   and registered by hash. No free-form query anywhere.
5. **Only DataCapabilities write**, always behind the DataGuard, always through
   the DataQueue. A DataCapability declares its `effect` and the existing
   fields it `touches`. `insert` with empty `touches` is the fast path and may
   be proposed by a read agent; everything else is the DataGuard agent's and a
   human reviews it.
6. **Invariants live in the DataGuard**, declared on the aggregate, each with
   tests. Never in a Capability, never in a Micro-UI.
7. **Every relation declares `onDelete` and `onUpdate`.** Deletion is a soft
   delete with a tombstone unless a human approves otherwise.
8. **A command is never rejected for concurrency.** It queues per aggregate,
   keeps its position, and its author confirms an overwrite. Relative commands
   need no confirmation. Parked commands expire after 24 hours, holds after 1
   minute unless renewed.
9. **Every value classifies itself**: derived (recomputed, carries its Concept
   version) or authoritative (canonical, mutated only through the DataGuard).
   Prefer derived when a stored value could be deduced from other rows.
10. **A Micro-UI declares `needs` and `actions`** and reads the Data through
    nothing else. Screens are composed from the registry and frozen in a
    versioned manifest; a frozen screen is replayed, never regenerated.
11. **Contracts first.** A unit lands with its manifest or contract file and
    passes the gate that validates it. A change to a contract schema is its
    own task and lists the units it touches.
12. **Read side merges on green gates; write side waits for a human.** In the
    harness, `harness:read-side` tasks run in parallel and merge on their own;
    `harness:write-side` tasks run one at a time and their pull request is
    merged by a human.

### What a session does before writing code

- Read `docs/ARCHITECTURE.md` and the `## Architecture` section of the issue.
- Find the unit's place in the layout above; create its manifest from the
  matching `contracts/*.schema.json` first, the code second.
- Run the gates locally where they exist (`npx ajv-cli validate` on the
  manifest, the isolation check), then the project's own tests.
<!-- /harness:architecture -->
