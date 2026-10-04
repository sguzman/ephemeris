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

## Current implementation

Ephemeris now has a working native application foundation.

Implemented:

- Rust + `eframe`/`egui` desktop shell
- embedded SQLite canonical store via `rusqlite`
- explicit schema migrations
- canonical `TemporalEvent` and `TemporalSource` types
- event-native storage with no Taskwarrior mediation
- distinct temporal semantics for date-only, all-day, exact instants, floating/local time, month precision, year precision, and unresolved time
- Year, Quarter, Month, Week, and Day date ranges
- Grid, Agenda, and dense Table as independent layouts
- source visibility controls
- text, domain, jurisdiction, and lifecycle-status filtering
- nested AND / OR / NOT advanced queries with typed predicates and a recursive editor
- timezone-aware temporal filters, including exact date overlap, precision classes, and relative day windows
- independent true grouping and stable multi-key sorting
- ordered query-driven color rules with first-match precedence and semantic fallback coloring
- embedded overlays with independent query/styling rules
- durable named saved views stored in SQLite
- saved views that retain query, source visibility, date range, layout, grouping, sort rules, color rules, overlays, timezone, and week-start behavior
- event inspector with Taria identity/provenance details
- separate unplaced/conflicted event surface
- native persisted transient UI state
- direct ingestion of Taria Resourcearium reconciled temporal event sets
- repeatable Taria re-import using stable source-record identity
- transactional source import reconciliation
- created / updated / unchanged / retained-missing accounting
- drag-and-drop Taria JSON import in the GUI
- standalone local import CLI
- CI enforcing rustfmt, compile, strict Clippy, and tests

The verified implementation milestone at 2026-10-04 passes the full CI gate: format, compile, strict Clippy, and **38 library tests**.

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

Taria / Resourcearium is the upstream temporal-data producer. Ephemeris is the local interactive consumer.

The current low-level integration consumes Resourcearium `ReconciledProjectionEventSet` JSON directly and preserves upstream identity/provenance instead of degrading events into generic appointments.

Taria now also has a consumer-facing **TemporalBundleRelease** layer. The agreed direction is:

```text
Taria acquisition/normalization/reconciliation
    -> frozen TemporalBundleRelease
    -> Ephemeris validates/adopts release
    -> canonical local events + CalendarSet membership
    -> saved views / overlays / queries
```

Important contract rules:

- Taria owns upstream acquisition and coverage accounting.
- Ephemeris does not fetch all 429 upstream Resources itself.
- Reconciled event sets carry event payload.
- CalendarSets carry membership/navigation metadata.
- overlapping domain bundles must not clone event identity.
- partial/pending/gap-only coverage must remain visible and must not be interpreted as an empty world.
- ICS/JSCalendar/jCal/CSV are downstream projections, not the canonical interchange.

The current bootstrap Taria release is integration-safe but does not yet expose reconciled-event-set paths in its shard manifest, so the existing Ephemeris event importer cannot consume that release end-to-end from the manifest alone. Production bundle artifacts already expose the richer reconciled payload path/hash shape.

See:

- [docs/TARIA_INTEGRATION.md](docs/TARIA_INTEGRATION.md)
- [docs/TARIA_BUNDLE_CONTRACT.md](docs/TARIA_BUNDLE_CONTRACT.md)

## Technology

Current architecture:

- Rust 2024
- native `eframe` / `egui`
- SQLite via bundled `rusqlite`
- local-first storage/query/rendering
- no Tauri
- no React/WebView application shell
- network access reserved for future acquisition/synchronization paths

## Run

```bash
cargo run
```

Ephemeris requires a graphical desktop session.

## Import Taria temporal data

A reconciled Resourcearium event-set JSON file can be dropped onto the running application.

The same artifact can be imported from the command line:

```bash
cargo run --bin ephemeris-import -- path/to/reconciled-event-set.json
```

Re-import is identity-aware. Unchanged records remain unchanged, changed records update in place, and records missing from a later artifact are retained unless stronger source semantics explicitly justify deletion/cancellation.

## Verify

```bash
cargo fmt --check
cargo check
cargo clippy --all-targets -- -D warnings
cargo test
```

## Documentation map

Start here:

- [docs/PRODUCT.md](docs/PRODUCT.md) - product definition, principles, anti-spec, and scope
- [docs/PROJECT_STATE.md](docs/PROJECT_STATE.md) - current implementation state and immediate next boundary
- [docs/ROADMAP.md](docs/ROADMAP.md) - staged implementation plan
- [docs/LINEAGE.md](docs/LINEAGE.md) - Rivet → Rivetr → Ephemeris genealogy
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) - system boundaries and layering
- [docs/DATA_MODEL.md](docs/DATA_MODEL.md) - canonical temporal domain model
- [docs/TIME_SEMANTICS.md](docs/TIME_SEMANTICS.md) - timezone, recurrence, all-day, interval, and lifecycle rules
- [docs/QUERY_AND_VIEWS.md](docs/QUERY_AND_VIEWS.md) - filtering, saved views, overlays, grouping, sorting, and color
- [docs/TARIA_INTEGRATION.md](docs/TARIA_INTEGRATION.md) - Taria integration semantics and release boundary
- [docs/TARIA_BUNDLE_CONTRACT.md](docs/TARIA_BUNDLE_CONTRACT.md) - exact Taria/Resourcearium -> Ephemeris bundle consumer contract
- [docs/INGESTION_AND_SYNC.md](docs/INGESTION_AND_SYNC.md) - imports, release adoption, refresh, identity, synchronization, and export
- [docs/UX.md](docs/UX.md) - calendar surfaces, dense-data behavior, inspection, keyboard interaction
- [docs/PERFORMANCE.md](docs/PERFORMANCE.md) - scale and latency expectations
- [docs/QUALITY.md](docs/QUALITY.md) - correctness, testing, migrations, and observability
- [docs/RIVETR_INHERITANCE.md](docs/RIVETR_INHERITANCE.md) - selective inheritance from Rivetr
- [docs/FUTURE_CAPABILITIES.md](docs/FUTURE_CAPABILITIES.md) - long-horizon capability ledger
- [docs/adr/](docs/adr/) - durable architectural decisions

## Development rule

When choosing work, ask:

> Does this make Ephemeris better at locally interrogating, understanding, or visualizing temporal data?

If not, it is probably outside the current project.

A second test is:

> Does changing presentation require duplicating or physically reorganizing canonical events?

If yes, the design is probably wrong.
