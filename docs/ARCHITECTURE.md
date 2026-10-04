# Architecture

## Status

This document defines the intended architecture before implementation. Specific crates, storage engines, and APIs remain subject to validation.

## Architectural goals

Ephemeris must optimize for:

- native local interaction
- dense temporal corpora
- strong event identity
- preserved source/provenance
- correct time semantics
- fast arbitrary querying
- durable saved views
- repeatable ingestion and refresh
- schema evolution
- offline operation
- selective reuse of Rivetr calendar code

## Proposed layers

```text
+------------------------------------------------------+
|                    Native UI                         |
| calendar / agenda / table / timeline / inspectors   |
+-----------------------------+------------------------+
                              |
+-----------------------------v------------------------+
|                  View / Query Engine                 |
| filters / derived fields / sort / group / color     |
| saved views / overlays / density / projections      |
+-----------------------------+------------------------+
                              |
+-----------------------------v------------------------+
|                 Temporal Domain Core                 |
| events / occurrences / sources / relations /        |
| collections / annotations / snapshots / identity    |
+-----------------------------+------------------------+
                              |
+-----------------------------v------------------------+
|              Local Store + Search Index              |
| transactions / history / indexes / migrations       |
+-----------------------------+------------------------+
                              ^
                              |
+-----------------------------+------------------------+
|                Ingestion / Refresh Layer             |
| Taria / ICS / JSON / CSV / APIs / CalDAV / webcal   |
| normalization / provenance / duplicate resolution    |
+------------------------------------------------------+
```

## Native UI

The application should remain a Rust native desktop application.

Preferred direction:

- `eframe`
- `egui`
- no WebView application shell
- no browser-local storage for canonical product state

The UI should consume domain/query APIs. It should not encode canonical temporal semantics ad hoc inside widgets.

## Temporal domain core

The core owns:

- canonical event identity
- event values
- occurrence identity
- lifecycle state
- sources
- provenance
- recurrence definitions
- relations
- collections/sequences
- snapshots
- annotations
- extensible properties

The UI should be replaceable without redefining these semantics.

## Local store

The canonical local store must support:

- transactional writes
- indexes suitable for interactive filtering
- event/source identity constraints
- source refresh reconciliation
- schema migrations
- snapshots/history
- bulk operations
- query pagination/streaming where needed
- durable user annotations independent of source refresh
- backup/export

### Storage engine

No engine is canonically selected yet.

Candidates should be evaluated against:

- SQLite maturity and portability
- full-text search
- JSON/extensible-property support
- transaction quality
- indexing flexibility
- migration ergonomics
- ability to query hundreds of thousands of records interactively
- ease of packaging as a native desktop app

An embedded relational engine is currently a stronger conceptual fit than flat files because saved views and temporal queries will become relationally rich.

Selecting the engine requires an ADR.

## Query/view engine

The query engine should operate on canonical event/occurrence records and derived properties.

It should support two surfaces:

1. simple facets for ordinary interaction
2. composable expression/query representation for advanced use

The saved-view layer stores query plus presentation, not copied event membership unless an explicit materialized snapshot is requested.

## Ingestion boundary

Ingestion adapters translate source representations into a normalized mutation plan.

An adapter should not write arbitrary storage directly.

Conceptual flow:

```text
source bytes/records
    -> parsed source representation
    -> normalized candidate events
    -> identity/duplicate/reconciliation stage
    -> transactional mutation
    -> provenance + snapshot records
```

The original source representation or sufficient source record metadata should remain addressable when practical.

## Taria boundary

Taria is not treated as "just another ICS source."

The Taria adapter should consume a rich, versioned temporal interchange representation preserving:

- identifiers
- provenance
- ontology
- snapshots
- source metadata
- lifecycle/status
- relations
- custom fields

The exact interchange format must be versioned and documented before coupling implementation to it.

## Rivetr reuse boundary

Rivetr calendar code can be classified into:

### Good candidates for reuse

- pure calendar date calculations
- view navigation
- egui rendering patterns
- density behavior
- calendar marker rendering
- ICS parser behavior with tests
- import reconciliation tests
- keyboard interaction patterns

### Candidates for adaptation

- imported-calendar persistence
- source coloring
- calendar filter state
- UI state persistence
- task-backed calendar entry conversion

### Do not inherit blindly

- `TaskDto` as the canonical event
- task datastore as canonical temporal store
- task lifecycle as event lifecycle
- task tags as the only metadata model
- any hard-coded service addresses
- unrelated workspaces

## Dependency direction

Lower layers must not depend on UI concepts.

Preferred dependency direction:

```text
ui -> query/view -> domain -> store abstractions
ingest -> domain/store mutation APIs
```

Avoid circular dependencies between rendering and canonical state.

## Async/background work

Network refresh, large imports, indexing, and expensive reconciliation should not block egui's frame loop.

The application should use explicit worker/task boundaries and communicate completion/progress safely back to the UI.

The UI must make work visible:

- importing
- indexing
- refreshing
- stale
- failed
- complete

## Configuration

Configuration should distinguish:

- user preferences
- saved views
- source definitions
- secrets/credentials
- canonical event data
- cache/index state

These should not be collapsed into one configuration file.

## Schema evolution

The model must support:

- strongly typed core fields
- extensible properties
- explicit schema versions
- forward migrations
- import-version compatibility
- source adapter versioning

Schema evolution is a first-class requirement because Taria's ontology will grow.

## Error boundaries

Failures should be classified at minimum as:

- source acquisition failure
- source parse failure
- normalization failure
- identity/reconciliation conflict
- storage failure
- query failure
- rendering failure
- configuration failure

A failed source must not corrupt the existing local corpus.

## Security boundary

External source content is untrusted input.

Parsers should:

- avoid command execution
- avoid arbitrary local path writes
- bound pathological payloads where practical
- validate external URLs
- separate credentials from logs
- preserve malformed-source diagnostics without trusting source HTML/text as executable content
