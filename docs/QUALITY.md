# Quality, Correctness, and Testing

## Priority order

For temporal software:

1. correctness
2. data integrity
3. recoverability
4. interaction latency
5. feature breadth

A visually impressive calendar with incorrect dates, identity collapse, or destructive refresh behavior is unacceptable.

## Core test categories

### Domain tests

Test:

- event identity
- occurrence identity
- lifecycle transitions
- annotations
- relations
- collections
- custom properties

### Time tests

See `TIME_SEMANTICS.md`.

Include:

- timezone conversion
- DST
- all-day semantics
- floating times
- recurrence
- exceptions
- month/year boundaries
- leap years

### Ingestion fixtures

Each adapter should have fixture corpora representing:

- valid input
- malformed input
- duplicate IDs
- changed records
- deleted records
- cancelled records
- rescheduled events
- missing optional fields
- large source payload

### Reconciliation tests

Given snapshot A then snapshot B, assert exact:

- created
- updated
- unchanged
- moved
- cancelled
- removed
- ambiguous

behavior.

### Query tests

Test:

- boolean composition
- ranges
- null checks
- set membership
- relative dates
- derived fields
- stable sorting
- grouping
- saved-view roundtrip

### Persistence tests

Test:

- migrations
- transaction rollback
- crash-safe import boundaries
- annotation survival across refresh
- backup/export
- corrupted optional cache recovery

### Performance tests

Maintain representative datasets for:

- 1k events
- 10k events
- 100k+ events

Measure important query and render paths.

## Migration policy

Schema changes require:

- explicit version
- forward migration
- tests from prior supported versions
- backup/recovery consideration

Do not silently discard unknown data.

## Import atomicity

A failed import or source refresh must not leave a half-updated canonical corpus.

Prefer parse/normalize/reconcile first, then transactional application.

## Auditability

For bulk or source-driven changes, record enough information to answer:

- what changed
- why
- from which source/import
- when
- whether it was user-owned or source-owned

Full undo/redo may arrive later, but the model should not make audit history impossible.

## Logging

Development logging should include:

- source ID
- snapshot/import ID
- adapter
- event counts
- phase timing
- failure classification

Do not log credentials or sensitive tokens.

## No hidden hacks

Hard-coded machine-specific addresses, environment-specific paths, or temporary workarounds must be:

- configuration
- clearly marked temporary
- or removed before becoming canonical architecture

The Rivetr dictionary host override is an example of the kind of migration-era hack Ephemeris should not inherit.

## Code quality

Preferred defaults:

- Rust edition 2024
- forbid unsafe code unless an explicit ADR later justifies otherwise
- avoid panicking in normal data paths
- typed errors at subsystem boundaries
- narrow modules
- pure functions for parsing/query/date logic where possible
- tests close to temporal semantics
