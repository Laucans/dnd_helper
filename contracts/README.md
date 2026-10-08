# Contracts

The twelve interface contracts of the agent-native architecture
(`docs/ARCHITECTURE.md`, "Interface contracts"), as JSON Schema draft 2020-12.
Installed by `harness init-repo`; the repository owns them from here. A change
to one of them is its own task, and lists the units it touches.

| File | Contract | Validates |
|---|---|---|
| `a-concept.schema.json` | A. Concept | `concepts/<Concept>/v<n>/concept.json` |
| `b-capability-manifest.schema.json` | B. Capability manifest | `crates/<system>/capabilities/<name>/capability.json` |
| `c-persisted-query.schema.json` | C. Persisted query registry | `queries/registry.json` |
| `d-micro-ui-manifest.schema.json` | D. Micro-UI manifest | `apps/<system>/<micro-ui>/micro-ui.json` |
| `e-screen-composition.schema.json` | E. Screen composition | `apps/screens/<screen>.json` |
| `f-data-capability.schema.json` | F. DataCapability | `crates/dataguard/data-capabilities/<name>/data-capability.json` |
| `g-queue-entry.schema.json` | G. DataQueue entry | what the queue stores and shows an author |
| `h-command-result.schema.json` | H. Command result | what the DataGuard answers a caller |
| `i-aggregate.schema.json` | I. Aggregate | `crates/dataguard/aggregates/<Aggregate>/aggregate.json` |
| `j-impact-plan.schema.json` | J. Impact plan | what the Resolver hands the DataGuard |
| `k-reconciliation.schema.json` | K. Reconciliation proposal | what the reconciliation AI hands an approver |
| `l-message.schema.json` | L. Message | what the Integrity DataGuard sends |

Validate a manifest by hand the way the `gates` workflow does:

```sh
npx --yes ajv-cli@5 validate --spec=draft2020 \
  -s contracts/b-capability-manifest.schema.json \
  -d crates/credit/capabilities/calculate-risk/capability.json
```

Contracts A, B, F and I fix the frontier between reading and writing: they are
the first to stabilise, and the ones a change to is reviewed hardest.
