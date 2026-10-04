# Ephemeris

**Ephemeris is a local-first temporal information system in which events are canonical data and calendars are programmable views.**

Ephemeris is a native Rust desktop application for ingesting, normalizing, querying, inspecting, and visualizing rich temporal data. It is designed first for the temporal resources produced by Taria, while remaining capable of ordinary personal scheduling.

This is deliberately **not** a general productivity suite. Tasks, Kanban, contacts, dictionaries, and unrelated workspaces are outside the product boundary unless they participate directly in temporal data.

## Why Ephemeris exists

Conventional calendar applications assume a person has a modest number of appointments divided into calendar containers. Those containers often simultaneously determine ownership, visibility, grouping, and color.

Ephemeris assumes something much richer:

- thousands to hundreds of thousands of temporal facts
- politics, elections, legislatures, courts, economics, finance, sports, holidays, institutions, research, and personal events
- overlapping and heterogeneous sources
- source provenance and authority
- tentative, projected, rescheduled, cancelled, observed, and superseded events
- historical snapshots and diffs
- rich ontology and ad hoc tags
- many simultaneous ways to interrogate the same corpus

The core design rule is therefore:

> Changing how an event is viewed must not require duplicating or reorganizing the underlying event.

## Product boundary

Ephemeris owns the temporal problem:

- canonical events and occurrences
- sources and provenance
- recurrence and timezone semantics
- saved views and overlays
- filtering, grouping, sorting, and color rules
- calendars, agendas, timelines, tables, and dense visualizations
- event inspection and annotations
- imports, refreshes, snapshots, and projections
- temporal history, lifecycle, relations, collections, and sequences
- local-first search and bulk operations

Ephemeris does **not** exist to become Rivetr again. It is a descendant of Rivetr's calendar work, not a replacement for Rivetr's broader productivity-suite identity.

## Lineage

```text
Rivet
  broad Tauri productivity application
    |
    v
Rivetr
  native Rust + egui successor to Rivet
    |
    +--> Ephemeris
           dedicated temporal-information descendant
```

Rivetr is the immediate implementation ancestor. Useful calendar code may be extracted or adapted from Rivetr, but Ephemeris does not inherit Rivetr's task-centric data model or non-calendar product obligations.

See [docs/LINEAGE.md](docs/LINEAGE.md).

## Core model

The intended domain centers on objects such as:

```text
TemporalEvent
EventOccurrence
TemporalSource
ProvenanceRecord
Snapshot
EventRelation
EventCollection
SavedView
Overlay
Query
ColorRuleSet
Annotation
```

An event exists once canonically. "US Politics", "California Elections", "Economic Releases", or "Sports This Week" are normally queries/views over the event corpus, not physical containers that duplicate events.

## Taria

Taria is expected to supply rich temporal resources. Ephemeris is the interactive desktop surface over those resources.

Ephemeris must preserve information such as:

- stable identity
- raw and normalized titles
- source identity and authority
- provenance and acquisition metadata
- domain and category
- geography and jurisdiction
- institution and participants
- event type
- lifecycle/status
- confidence and uncertainty
- original timezone and normalized instants
- recurrence and occurrence identity
- relations, collections, and sequences
- snapshot/history membership
- extensible/custom properties

Taria data must not be flattened to the lowest-common-denominator ICS event model.

See [docs/TARIA_INTEGRATION.md](docs/TARIA_INTEGRATION.md).

## Technology direction

The intended implementation direction is:

- Rust
- native desktop UI with `eframe` / `egui`
- local-first storage and query
- no Tauri
- no React/WebView application shell
- network access only for acquisition/synchronization paths, never required for ordinary rendering/querying

The exact storage engine and crate boundaries are intentionally documented as architectural decisions to be validated before implementation.

## Documentation map

Start here:

- [docs/PRODUCT.md](docs/PRODUCT.md) - product definition, principles, anti-spec, and scope
- [docs/LINEAGE.md](docs/LINEAGE.md) - Rivet → Rivetr → Ephemeris genealogy
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) - proposed system boundaries and layering
- [docs/DATA_MODEL.md](docs/DATA_MODEL.md) - canonical temporal domain model
- [docs/TIME_SEMANTICS.md](docs/TIME_SEMANTICS.md) - timezone, recurrence, all-day, interval, and lifecycle rules
- [docs/QUERY_AND_VIEWS.md](docs/QUERY_AND_VIEWS.md) - filtering, saved views, overlays, grouping, sorting, and color
- [docs/TARIA_INTEGRATION.md](docs/TARIA_INTEGRATION.md) - Taria ingestion contract and source preservation
- [docs/INGESTION_AND_SYNC.md](docs/INGESTION_AND_SYNC.md) - imports, refresh, identity, snapshots, synchronization, and export
- [docs/UX.md](docs/UX.md) - calendar surfaces, dense-data behavior, inspection, keyboard interaction
- [docs/PERFORMANCE.md](docs/PERFORMANCE.md) - latency and scale requirements
- [docs/QUALITY.md](docs/QUALITY.md) - correctness, testing, migrations, and observability expectations
- [docs/ROADMAP.md](docs/ROADMAP.md) - staged implementation plan
- [docs/PROJECT_STATE.md](docs/PROJECT_STATE.md) - current concrete state and next implementation boundary
- [docs/adr/](docs/adr/) - durable architectural decisions

## Current state

**Documentation bootstrap only.**

No production architecture should be inferred from repository emptiness. The first implementation phase will selectively inherit proven calendar code from Rivetr while establishing an Ephemeris-native temporal model and Taria ingestion boundary.

See [docs/PROJECT_STATE.md](docs/PROJECT_STATE.md).

## Development rule

When choosing work, ask:

> Does this make Ephemeris better at locally interrogating, understanding, or visualizing temporal data?

If not, it is probably outside the current project.

A second test is:

> Does changing presentation require duplicating or physically reorganizing canonical events?

If yes, the design is probably wrong.
