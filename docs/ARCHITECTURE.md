# Agent-native application architecture

Reference document · installed by `harness init-repo` · version 1

<!-- harness:architecture v1 -->

## Intent and principles

This architecture optimises the **parallelism of development agents** rather
than the reuse of code. The world of reading is massively parallelisable; truth
and mutation are centralised behind one control frontier.

It combines known patterns — Vertical Slice Architecture, CQRS and a command
bus, Generative UI, optimistic concurrency, human-in-the-loop — but aims at a
more precise property: **units of reading that agents can develop
independently, and data protected by a single layer.**

### Fundamental principles

1. A **Micro-UI** describes a need to read and declares the actions it can
   trigger.
2. **Capabilities** collect, transform and enrich. They never modify data
   directly.
3. A Capability is strictly **read-only**: it knows neither the other
   Capabilities nor any mutation infrastructure. Mutations are triggered by
   Micro-UIs through **DataCapabilities**, behind the **DataGuard**.
4. Capabilities are as independent as possible.
5. Code is reused *inside* a system; duplication *between* systems is accepted
   when it reduces coupling.
6. Consistency is about **meaning** (versioned Concepts with conformance tests),
   not about implementation.
7. The **Data** is the source of truth; every read goes through a persisted,
   declared query.
8. Every mutation goes through the **DataQueue**, then the DataGuard.
9. A pure addition follows a **fast path**; any mutation of existing data goes
   through the **Resolver**.
10. A command is never rejected because of concurrency: it waits for its turn,
    and its author confirms whether it overwrites a value declared ahead of it.
11. AI may propose a reconciliation but never decides a critical change alone.
12. Contracts are verified automatically in CI; that is what lets many agents
    work without a human review of every change.
13. A Capability knows only the data it reads and the Concepts it uses. It
    knows neither the other Capabilities nor any mutation infrastructure
    (DataCapability, DataQueue, DataGuard, Resolver).

## Overview

The system is cut in two worlds. On the left, **reading**: many independent
units, developed in parallel. On the right, **writing**: a single controlled
frontier, which owns the truth.

Micro-UIs and Capabilities never write directly. Micro-UIs trigger
DataCapabilities, whose commands go through the DataQueue. The **Integrity
DataGuard** sends its messages (confirmations, parked, applied) to the authors
and to the infrastructure components concerned; a Capability does not depend on
the DataGuard.

### Existing code and hosts without a server

The architecture is adopted by repositories that already exist, and some of
them run nowhere but inside a host written in TypeScript — an Obsidian or
VS Code plugin, a static web app. Neither changes the rules. The existing
program is the Micro-UI shell; what it already does stays where it is until
a task moves it. Every unit added from now on is built where the layout says
and in the language the stack says: Rust for Capabilities, DataCapabilities,
the Data layer and infrastructure, TypeScript for Micro-UIs. When no server
can run a Rust unit, it is compiled to WebAssembly and the host loads it; the
build and the loading step are part of the unit's task, and its manifest sits
at the same place as on a server.

## UI / Micro-UI

A Micro-UI is the **specification of a need**: what it displays says what must
be read, computed or enriched. It consumes Capabilities and declares the actions
(commands) it can trigger.

- **Reading**: the Micro-UI declares the Capabilities it needs (`needs`). It
  never reads the Data directly.
- **Writing**: a user action produces a command toward a DataCapability
  (`actions`). The Micro-UI then displays the command's lifecycle: queued,
  awaiting confirmation, applied, rejected.
- **Composition**: a screen is composed on demand by an AI from a user goal.
  The AI picks the Micro-UIs from a registry, through their description,
  without knowing the rest of the application.
- **Reproducibility**: a validated composition is frozen in a versioned
  manifest, then replayed as is. The screen is not regenerated on every
  visit, which makes it testable and deterministic.

Example: the goal "assess whether we can raise this customer's limit"
produces a screen composed of `RiskBadge`, `LatePaymentsList` and
`LimitReviewForm`.

## Capabilities

A Capability is an autonomous unit that produces a value or a view: it
collects, transforms, enriches or combines data. It is strictly read-only with
respect to business data. It knows only the data it reads and the Concepts it
uses; it knows neither the other Capabilities, nor the DataCapabilities, nor
the DataQueue, nor the DataGuard, nor the Resolver.

### System

A system is a **bounded context**: one owner, one namespace (`credit.*`) and a
local registry of Capabilities. This frontier tells an agent whether it should
look for an existing Capability or write a new one.

| Situation | Rule |
|---|---|
| A Micro-UI needs a Capability that exists in the same system | It is reused, by reference (`needs:`), never by importing code into the Micro-UI |
| A Capability would need the behaviour of another Capability | It neither invokes nor imports it; it implements locally what it needs, reusing the same Concepts if necessary |
| A business rule shared between systems | Shared as a Concept with conformance tests, not as a library |
| A write-side validation or invariant | Never in a Capability: it lives in the DataGuard |

### Duplication

The problem is not duplicated code but **divergent meaning**. Duplicating
display or enrichment is harmless. Duplicating a business computation is
acceptable only if each copy declares the Concept it implements and passes its
conformance tests.

### Expected properties

- Read-only on business data, **guaranteed by read-only credentials**, not by
  convention.
- Reads declared as persisted queries.
- Every important value is returned with the version of its Concept and the
  version of the data read (`asOf`).
- A Capability depends on no other Capability: every piece of logic it needs is
  local to it. When a shared behaviour must stay semantically coherent, it is
  defined by a Concept and its conformance tests.
- Its cache is invalidated by the Data infrastructure when the data it depends
  on changes; the Capability does not subscribe to the DataGuard. **A cache is
  keyed by the authorisation context it was filled under** (role, scope): a
  value computed for one viewer is never served to another with fewer rights.

### Semantic consistency

What is shared between systems is **meaning, not code**. A versioned Concept
(`Risk@3`) fixes the definition, the rules, the examples and executable
conformance fixtures. Each implementation stays independent but must pass
them.

Concepts change rarely: that makes them a good central point, unlike a shared
library that many agents would modify.

### Derived or authoritative

Every value is classified explicitly:

| Kind | Examples | Can be recomputed locally | Mutation |
|---|---|---|---|
| Derived | `riskScore`, `customerAge`, `displayName` | Yes, if the Concept is respected | None: it is recomputed |
| Authoritative | `creditLimit`, `accountBalance`, `contractStatus` | No, it is read as canonical data | Only through a DataCapability and the DataGuard |

A stored authoritative value that could be deduced from other rows (a balance
updated at every payment) turns every addition into a modification. When
possible, model it as derived: adding a payment then stays a pure addition.

### Traceability

An important value carries the version of its Concept:
`{ "riskScore": 83, "riskLevel": "HIGH", "riskModel": "v3", "asOf": 1042 }`.
A deprecated Concept stays available as long as a Capability declares that it
implements it.

## Data and reading

The Data is the source of truth. It is exposed for reading through persisted
queries over read models; the technology matters less than the rule: **every
read is declared**.

### Persisted queries

A Capability invokes no other Capability. It sends no free-form query. It
declares only persisted queries, hashed and registered under its own
identifier. This registry makes it possible:

- to know exactly which Capabilities use which field;
- to compute the impact of a schema change before making it;
- for the Resolver to know who depends structurally on a table or a field.

The schema evolves by **expand/contract** migrations: add the new field,
migrate the Capabilities, then remove the old one.

### Read authorisation

Rights are not only about writing. The Data layer applies **row-level and
field-level filtering** (personal data, customer scope, role). This filtering
is never delegated to the Capabilities.

### Consistent read after write

An applied command returns a `dataVersion`. A read may require a version at
least equal to it, which lets the author see their own change despite the
asynchronous write. A read may also target a precise version, for audit or for
tests. **`dataVersion` is one global sequence**, issued by the single applier
(see below): partitions advance independently, but every applied command
takes the next number in that one sequence.

## Writing: DataCapabilities and the DataGuard

Only DataCapabilities write, and always behind the DataGuard. A Micro-UI action
references a DataCapability, which produces a command in the DataQueue. A
Capability has no dependency on the mutation system and references no
DataCapability.

### DataCapability

A DataCapability is a **typed command** that declares its effect — `insert`,
`update`, `delete` or `upsert` — and the existing fields it touches
(`touches`). It belongs to the DataGuard, not to the Micro-UI that triggers it.

| DataCapability | Who may create it | Review |
|---|---|---|
| `effect: insert`, no `touches` | A read agent may propose it | Auto-approved if the CI gates pass |
| `update`, `delete`, `upsert`, or with `touches` | The DataGuard agent | Human review mandatory |

This split keeps the fast path open to parallel agents without opening the
mutation of existing data.

### Fast path: pure addition

An `insert` is a pure addition if it meets all these conditions:

- a new row with a new identifier, no upsert;
- no collision on a unique key;
- no trigger or side effect that writes into an existing row;
- no stored authoritative value that changes;
- outgoing foreign keys toward existing, non-archived rows.

The fast path skips the Resolver, not the DataGuard: invariants are still
checked, because an addition can violate a rule over several rows (sum of
reservations ≤ stock).

The classification is static (what the DataCapability declares), then dynamic:
if a collision or an unforeseen effect appears at execution, the command is
routed to the Resolver instead of failing.

### Responsibilities of the DataGuard

Authorisation and permissions, validation, business invariants, concurrency,
classification of the effect, orchestration of the Resolver and the DataQueue,
audit. **No write bypasses it.**

## Resolver

Every modification, deletion or addition that touches existing data goes
through the Resolver. It checks three things and produces an **impact plan**:
the dependencies, the consequences on links, then the invariants on the final
state.

### Dependencies

| Level | Question | Source |
|---|---|---|
| Structural | Which Capabilities read this table or field? | The registry of persisted queries |
| Instance | Does someone depend on this row right now? | Queued commands on the row, incoming references, holds |

The instance level cannot be guessed: a workflow in progress declares it with a
**hold** (for example a credit review open on customer 881). A hold is an entry
of the DataQueue: it freezes the partition from its position, has a `ttl`, can
be renewed and disappears at expiry.

### Consequences on links

Every relation declares its policy (`restrict`, `cascade`, `nullify`,
`archive`) and can be marked `semantic` when the link exists outside a foreign
key. The Resolver walks the incoming references, computes the complete effect
and lists the derived values to invalidate.

### Decision

The impact plan ends in `auto`, `confirm`, `human_review`, `blocked` or
`rejected`. A `blocked` is not a rejection: the command keeps its place in the
DataQueue.

### Deletion

Deletion is a **soft delete** by default (archive with a tombstone). A hard
delete is rare and goes through human review. Everything stays reversible
thanks to the command journal. **The Data layer filters tombstones**: no
persisted query sees an archived row unless it asks for it explicitly.

## DataQueue and concurrency

A command is never rejected because of concurrency. It takes a place in a queue
ordered **by partition — one partition per aggregate** — keeps it, and its
author confirms whether it overwrites a value declared ahead of it. Different
partitions advance in parallel.

### Roles

| Component | Role |
|---|---|
| DataQueue | FIFO queue per partition; no global order |
| Integrity DataGuard | Watches the queue: computes the projected state, detects stale commands, checks invariants, emits the messages |
| DataResolver | Processes the head of the queue: final check, cascades, application. **One applier** for the whole store, which is what makes `dataVersion` a single sequence and lets an invariant spanning several aggregates be checked on the real state at application |

### Rules

1. **Fixed position.** A command keeps its place while it waits for a
   confirmation. It blocks nobody as long as it is not at the head.
2. **Two-layer projected view.** A command submitted behind others sees the
   confirmed projected state, plus the list of commands awaiting confirmation
   placed ahead of it.
3. **Overwrite confirmation.** If a value is declared ahead of its command, the
   author is notified and confirms or cancels. A confirmed command overwrites
   in its turn.
4. **Parked.** A command that reaches the head without confirmation is set
   aside and the queue goes on. The author is notified.
5. **Back in the queue.** The author of a parked command may ask for a recompute
   of the projected state: the command returns to the end of the queue, with a
   new base, and they confirm against this new view.
6. **Expiry.** A parked command with no action before its `ttl` disappears; the
   author is told.
7. **Holds.** A hold freezes the partition from its position, can be renewed,
   and disappears at expiry.
8. **Double check of the invariants.** At enqueue, on the projected state, to
   warn early; at application, on the real state, which alone decides.

A confirmed command advances in its place up to the head. Only a command that
reaches the head unconfirmed leaves the queue, and comes back only at the end,
after a recompute. The author may also cancel at any time before application.

**Relative commands** (`stock -= 2`) compose with the commands ahead instead of
overwriting them: they are never stale and **need no confirmation**; only the
invariants are checked, at application.

### Default durations

| What | Default |
|---|---|
| A hold's `ttl` | 1 minute, renewable while the workflow that holds it is alive |
| A parked command's `ttl` | 24 hours — a human confirmation takes hours, not seconds |

### Why a confirmation stays valid

The queue is FIFO and a parked command only comes back at the end. The set of
commands ahead of a command can therefore only shrink: nothing can slip in.
When the author confirms, they have seen everything that can still apply before
them. Their confirmation covers every scenario and never needs to be asked
again.

A confirmation is not a guarantee of application: the DataResolver may still
reject at the head if an invariant or a cascade forbids it, and the
`CommandRejected` message says why.

### Two distinct confirmations

- **Overwrite confirmation**, by the author: they accept to overwrite what is
  declared ahead of them.
- **Critical review**, by an approver role: required for `critical` fields or
  an impact plan in `human_review`. It adds to the first; AI may propose a
  reconciliation, a human decides.

### Messages of the Integrity DataGuard

| Message | Recipients | When |
|---|---|---|
| `CommandQueued` | Author | At enqueue |
| `InvariantAtRisk` | Author | The projected state violates an invariant |
| `ValueDeclaredAhead` | Author | A value is declared ahead of their command |
| `BlockedByHold` | Author, hold owner | The partition is frozen |
| `HoldExpiring` | Hold owner | Before the end of the `ttl` |
| `ReviewRequired` | Approvers | Critical field or `human_review` |
| `Parked` | Author | Reached the head unconfirmed |
| `ParkedExpiring` | Author | Before the end of the `ttl` |
| `Expired` | Author | Parked command withdrawn |
| `AheadResolved` | Author | Optional: a command ahead was applied or cancelled |
| `CommandApplied` | Author, infrastructure components concerned | Application; invalidates caches |
| `CommandRejected` | Author | Rejection at the head, with the reason |

Messages travel over **server-sent events** from the Integrity DataGuard, one
subscription per author; infrastructure components subscribe the same way.

## Interface contracts

Twelve contracts define every frontier. They live as **JSON Schema (draft
2020-12) in `contracts/`**, one file per contract; the examples below are YAML
for readability. Contracts A, B, F and I fix the frontier between reading and
writing: they are the first to stabilise.

| Contract | Frontier | Owner |
|---|---|---|
| A. Concept | Shared semantics | Semantic agent + human |
| B. Capability manifest | Capability ↔ registry and composer | Read agent |
| C. Persisted query | Capability ↔ Data | Read agent |
| D. Micro-UI manifest | Micro-UI ↔ Capabilities and actions | Read agent |
| E. Screen composition | User goal ↔ Micro-UIs | Composer agent |
| F. DataCapability | DataCapability ↔ DataGuard | DataGuard agent |
| G. DataQueue entry | Queued command | DataGuard |
| H. Command result | DataGuard ↔ caller | DataGuard |
| I. Aggregate: criticality, invariants, relations | Integrity rules | DataGuard agent + human |
| J. Impact plan | Resolver ↔ DataGuard | Resolver |
| K. Reconciliation proposal | AI ↔ approver | Integrity DataGuard |
| L. Message | Integrity DataGuard ↔ users and infrastructure components | Integrity DataGuard |

### A. Concept

```yaml
concept: Risk
version: 3
kind: derived                 # derived | authoritative
definition: >
  Probability of the customer defaulting within 90 days, on a 0-100 scale.
output:
  riskScore: { type: integer, min: 0, max: 100 }
  riskLevel: { enum: [LOW, MEDIUM, HIGH], rule: "HIGH if score >= 70" }
inputs: [customer.payments, customer.outstandingBalance]
conformance: ./risk-v3.fixtures.json
deprecates: 2
```

### B. Capability manifest

```yaml
capability: customer.calculateRisk
system: credit
version: 1.4.0
description: "Computes the customer risk per Risk v3"   # read by the composer
implements: Risk@3
input:  { customerId: ID }
output: Risk@3 & { asOf: DataVersion }
reads:  [queries/customerPayments.graphql]
cache:  { ttl: 5m }
```

### C. Persisted query

```graphql
query CustomerPayments($id: ID!) @capability(id: "customer.calculateRisk") {
  customer(id: $id) { outstandingBalance payments(last: 24) { dueAt paidAt amount } }
}
```

### D. Micro-UI manifest

```yaml
microUi: RiskBadge
description: "Shows a customer's risk score and level"
needs:   [customer.calculateRisk]
actions: [credit.requestLimitReview]
props:   { customerId: ID }
```

### E. Screen composition

```yaml
screen: credit-review
generatedFrom: "Assess whether we can raise this customer's limit"
version: 7                    # frozen once validated, replayed as is
layout:
  - RiskBadge:        { customerId: $ctx.customerId }
  - LatePaymentsList: { customerId: $ctx.customerId }
  - LimitReviewForm:  { customerId: $ctx.customerId }
```

### F. DataCapability

```yaml
dataCapability: credit.requestLimitChange
owner: dataguard
version: 2
effect: update                # insert | update | delete | upsert
target: { aggregate: Account, id: $accountId }
touches: [Account.creditLimit]   # empty + insert ⇒ fast-path candidate
payload: { newLimit: Money, reason: string }
mode: confirm_on_stale        # confirm_on_stale | overwrite | relative
invariants: [limit-under-ceiling, limit-change-requires-risk]
permissions: [credit.limit.write]
callableBy: [credit.*]
idempotencyKey: required
```

### G. DataQueue entry

```yaml
command: c_130
dataCapability: credit.requestLimitChange@2
by: carol
partition: Account/881
position: 3
basedOn: { version: 10, values: { creditLimit: 5000 } }
projection:
  confirmed: { creditLimit: 8000, after: [c_120] }
  pendingAhead:
    - { command: c_123, by: bob, value: 3000, state: awaiting_confirmation }
yourValue: 4500
state: awaiting_confirmation  # queued | awaiting_confirmation | confirmed | awaiting_review
                              # | parked | applied | rejected | cancelled | expired
confirmation: { by: carol }
parked: { ttl: 24h, onExpire: drop }
requeuedFrom: null            # the original commandId if requeued
```

### H. Command result

```yaml
commandId: c_130
status: applied               # applied | rejected | expired | cancelled
dataVersion: 1043             # for the consistent read after write
violations: []                # violated invariants if rejected
reviewId: null
```

### I. Aggregate: criticality, invariants, relations

```yaml
aggregate: Account
fields:
  creditLimit:  { kind: authoritative, criticality: critical, review: role:credit-manager }
  contactEmail: { kind: authoritative, criticality: normal }
  tags:         { kind: authoritative, criticality: low, merge: set }
invariants:
  - id: limit-under-ceiling
    rule: "creditLimit <= policy.maxLimit(riskLevel)"
    onViolation: reject
  - id: limit-change-requires-risk
    rule: "Risk@3 computed less than 24h ago"
    onViolation: human_review
relations:
  - { from: Payment.accountId, onDelete: restrict, onUpdate: propagate }
  - { from: Document.subject, semantic: true, onDelete: review }
holds: { maxTtl: 1m, renewable: true }
```

### J. Impact plan

```yaml
command: c_201                # credit.archiveCustomer(881)
classification: delete
dependencies:
  readers: [customer.calculateRisk, billing.invoiceList, crm.timeline]
  holds:   [{ by: credit.limitReview, id: w_77, expires: 2026-10-09 }]
  pendingCommands: [c_198]
cascade:
  - { relation: Payment, rows: 42, policy: restrict, blocking: true }
  - { relation: Note, rows: 3, policy: cascade }
  - { relation: Document, rows: 1, policy: review, semantic: true }
invalidates: [Risk@3(customer:881), Dashboard(customer:881)]
invariantsAfter: ok
decision: blocked             # auto | confirm | human_review | blocked | rejected
reasons: ["active hold w_77", "Payment restrict"]
```

### K. Reconciliation proposal

```yaml
review: r_45
aggregate: Account/881
field: creditLimit
base:    { version: 10, value: 5000 }
candidates:
  - { command: c_120, by: alice, value: 8000 }
  - { command: c_123, by: bob,   value: 3000 }
proposal:
  by: ai
  value: 3000
  rationale: "Risk went HIGH between the two requests; c_120 was based on a stale Risk"
  invariantsChecked: [limit-under-ceiling]
decision: { status: pending, approver: role:credit-manager }
```

### L. Message

```yaml
message: ValueDeclaredAhead
to: [bob]
command: c_123
field: Account/881.creditLimit
yourValue: 3000
declaredAhead: { value: 8000, by: alice, command: c_120 }
actions: [confirm_overwrite, cancel, edit]
```

## CI gates

Contracts protect only if they are verified automatically. These gates block
any agent contribution that violates them. They live in
`.github/workflows/gates.yml`; the ones that can be checked mechanically today
are implemented there, the others are declared with their rule and switched
off until an agent implements them.

| Gate | Checks | Contracts |
|---|---|---|
| Semantic conformance | `implements: Risk@3` ⇒ the Concept's fixtures pass | A, B |
| Read-only | No Capability has write access: lint and read-only credentials | B |
| Capability isolation | No Capability imports, invokes or references another Capability or any mutation infrastructure | B |
| Queries | Every persisted query is valid against the current schema and the next | C |
| Schema impact | A schema change lists the Capabilities touched and blocks until each is migrated | C, I |
| System frontier | `needs:` — a Micro-UI references only the authorised Capabilities of its system | B |
| Write actions | `actions:` references existing DataCapabilities, with compatible `callableBy` and `permissions`; no Capability has an `invokes` field | D, F |
| Effect classification | An `insert` DataCapability without `touches` contains no update, upsert or trigger | F |
| Ownership | An `update`, `delete`, `upsert` DataCapability, or one with `touches`, requires a DataGuard and a human review | F, I |
| Invariants | Every declared invariant has tests; every DataCapability lists the ones it may affect | F, I |
| Relations | Every foreign key has a declared `onDelete` and `onUpdate` policy | I |
| Compositions | Every Micro-UI of a frozen screen exists in the referenced version | D, E |

## Organisation of the agents

Many agents in parallel on the read side, few agents and a human review on the
truth side.

| Agent | Number | Produces | Review |
|---|---|---|---|
| Read agents | Many, one per system or per Capability | Capabilities, persisted queries, Micro-UIs, pure-addition DataCapabilities | CI gates only |
| Semantic agent | One | Concepts, conformance fixtures | Human |
| DataGuard agent | One | Mutating DataCapabilities, invariants, relations, schema, migrations | Human, mandatory |
| Composer agent | At run time or at design time | Screen compositions | Validation before freezing |
| Reconciliation AI | At run time | Reconciliation proposals | Human approver |

Example of a conflict-free split: Customer Risk, Orders, Revenue, Customer
Timeline and Dashboard developed by five read agents in parallel, without
touching a shared component outside the Concepts and the DataGuard.

In the harness that drives this repository, a task on the read side carries
`harness:read-side` and runs in parallel with its peers; a task on the write
side carries `harness:write-side`, runs alone, and its pull request waits for a
human merge.

## Technology choices

Kept as simple as the rules allow:

- **Rust** for everything but the Micro-UIs: the Capabilities, the Data layer,
  the DataQueue, the DataGuard, the Resolver, the DataCapabilities. One
  workspace, one crate per Capability, one crate for the write side.
- **TypeScript** for the Micro-UIs and the composer: thin, declarative, calling
  Capabilities and DataCapabilities by identifier only.
- **PostgreSQL alone** as the store: the DataQueue is a table (one row per
  command, `SELECT … FOR UPDATE SKIP LOCKED` per partition), `LISTEN/NOTIFY`
  carries the invalidations, the read models are views, the persisted queries
  are files in the repository registered by hash. No Kafka, no NATS.
- Development data: one isolated database per agent (a branch, a schema, a
  container); in production, one `main` fed by the DataQueue.

## Known limits and open questions

### Limits

- **Invariants not expressible in SQL.** A rule over several rows or over a
  derived value must be executed by the DataGuard; no database constraint
  covers it.
- **Write latency.** The asynchronous write, the holds and the human review
  suit a back office, not real-time transactional work.
- **Point of concentration.** The DataGuard, the schema and the Concepts
  concentrate the risk and the human review load. It is intended, but their
  throughput bounds that of the agents as soon as an evolution touches the
  write side.

### Decided

- Partitions: **one per aggregate**.
- Default durations: holds **1 minute**, renewable; parked commands **24
  hours**.
- Relative commands: **exempt from confirmation**; invariants checked at
  application.
- Contracts: **JSON Schema**, in `contracts/`.
- DataQueue and read layer: **PostgreSQL only**, as above.

<!-- /harness:architecture -->
