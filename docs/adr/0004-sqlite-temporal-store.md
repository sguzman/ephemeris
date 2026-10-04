# ADR 0004: SQLite is the canonical local temporal store

Status: Accepted

## Context

Ephemeris requires a local-first store capable of:

- transactional imports and refreshes
- indexed time-range queries
- source/event identity constraints
- provenance and snapshot tables
- schema migrations
- bulk operations
- full-text/search extensions later
- packaging inside a native desktop application
- operation without a separate database service

Flat files would make cross-cutting temporal queries, identity constraints, and transactional refresh significantly harder.

A separate PostgreSQL service would violate the desired self-contained local desktop model.

## Decision

Use SQLite as the canonical local database.

Use rusqlite in the initial Rust implementation, with bundled SQLite for predictable desktop builds.

The first schema will model temporal events directly rather than serializing Rivetr task records.

## Initial storage rules

- UUIDs are stored as canonical text identifiers.
- Time semantics use explicit kind-specific columns rather than coercing every event to UTC.
- All-day events retain civil dates.
- Exact/zoned events retain UTC instants and optional source timezone.
- Floating events retain local datetime values without invented timezone conversion.
- Source identity is a foreign key, not a tag.
- Extensible properties may use JSON while stable query dimensions graduate to typed/indexed columns.
- Schema versioning is explicit.
- Foreign keys are enabled.
- Imports that mutate multiple records must use transactions.

## Consequences

Positive:

- mature embedded relational storage
- strong local portability
- atomic refresh/import operations
- efficient indexed date/source/status queries
- no external service dependency
- straightforward backups

Costs:

- schema migrations must be managed carefully
- advanced ontology/custom fields require deliberate indexing strategy
- timezone-aware queries remain application-assisted rather than native timezone operations

## Revisit conditions

Reconsider only if measured scale or query requirements cannot be met without unacceptable complexity or latency.
