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
- persisted recurrence definitions with secondly/minutely/hourly/daily/weekly/monthly/yearly frequency, interval, count, until bounds, secondly/minutely/hourly/daily BYDAY filtering, secondly/minutely/hourly/daily/weekly/monthly BYMONTH limiting, signed secondly/minutely/hourly/daily BYMONTHDAY filtering, secondly/minutely/hourly/yearly BYYEARDAY limiting/selection, weekly multi-day BYDAY + explicit WKST, monthly plain BYDAY expansion, signed monthly BYMONTHDAY selection, monthly ordinal BYDAY selection with plain/ordinal union and BYMONTHDAY intersection, yearly plain BYDAY expansion across the recurrence year or selected BYMONTH months, context-aware yearly ordinal BYDAY (whole-year without BYMONTH, month-scoped with BYMONTH), signed yearly BYMONTHDAY expansion across all months or selected BYMONTH months, yearly BYMONTH composition with ordinal BYDAY selectors, yearly BYWEEKNO selection with WKST-aware week numbering and plain BYDAY, frequency-aware BYHOUR/BYMINUTE/BYSECOND handling for floating/exact date-time recurrences, including SECONDLY cadence where BYHOUR/BYMINUTE/BYSECOND all limit, MINUTELY cadence where BYHOUR/BYMINUTE limit and BYSECOND expands, and HOURLY cadence where BYHOUR limits and BYMINUTE/BYSECOND expand, generic BYSETPOS filtering over supported BY-selector candidate sets, RDATE additions, EXDATE exclusions, and moved/cancelled occurrence overrides
- query-window recurrence expansion with deterministic original-slot occurrence identity instead of eager infinite materialization
- recurrence support for date-only, all-day, floating, and exact/source-timezone events, including source-wall-clock preservation across DST and moved instances that enter the active window
- canonical recurrence authoring with quick presets, structured exceptions, structured BYWEEKNO, BYYEARDAY, BYMONTHDAY, BYHOUR, BYMINUTE, BYSECOND, BYSETPOS, and ordinal-BYDAY rows with raw-syntax fallbacks, live validation, and frequency/time-kind-aware selector affordances that hide irrelevant empty fields while preserving incompatible existing values for explicit recovery or clearing
- strict RFC 5545 recurrence-property adapters: RRULE parse/format for the canonical recurrence model plus RDATE/EXDATE parse/format for DATE, floating DATE-TIME, UTC DATE-TIME, and matching-TZID exact series; lossy DATE-TIME UNTIL, RDATE PERIOD, mismatched exception shapes, and unsupported parameters are rejected rather than coerced
- Year, Quarter, Month, Week, and Day date ranges
- Grid, Agenda, Compact Agenda, chronological Stream, proportional Timeline, Density, Summary, and dense Table as independent layouts
- ordered user-defined Table columns with add, hide, reorder, and reset controls
- source visibility controls plus a source inspector with identity, authority, locator, refresh metadata, properties, and canonical event counts
- text, domain, jurisdiction, event-type, institution, renderability, tag, and lifecycle-status filtering
- nested AND / OR / NOT advanced queries with typed predicates and a recursive editor
- timezone-aware temporal filters, including exact date overlap, precision classes, and relative day windows
- independent true grouping and stable multi-key sorting
- ordered query-driven color rules with first-match precedence and semantic fallback coloring
- embedded overlays with independent query/styling rules
- ordered union / intersection / subtraction composition layers over the base query with a full GUI editor
- cycle-safe composition operands that can reuse another saved view's logical event set by stable UUID
- durable named saved views stored in SQLite
- saved views that retain query, source visibility, date range, layout, grouping, sort rules, color rules, embedded/referenced composition layers, overlays, ordered Table columns, timezone, and week-start behavior
- event inspector with Taria identity/provenance details
- separate unplaced/conflicted event surface
- native persisted transient UI state
- direct ingestion of Taria Resourcearium reconciled temporal event sets
- direct ingestion of pinned `CompactReconciledEventIndex` recovery payloads
- filesystem-first Taria/Resourcearium workspace integration
- persisted local Resourcearium root and release channel
- automatic local/sibling Taria checkout detection
- one-click **Update Taria Sources** from local release manifests
- background/nonblocking Taria release adoption using a dedicated SQLite worker connection
- Resourcearium-compatible integrity validation: bootstrap content fingerprints and production file SHA-256
- immutable Taria release metadata in SQLite
- release-to-source projection associations and current/historical source posture
- immutable per-release canonical event snapshots
- immutable CalendarSet storage plus release-to-CalendarSet associations
- CalendarSet projected-calendar and event-membership persistence
- many-to-one upstream identity aliases across overlapping Taria projections
- production `bundle_artifacts[]` adoption with cross-bundle canonical-event deduplication
- current-release Taria bundle membership predicates
- projected CalendarSet membership predicates
- membership-aware saved views, overlays, color rules, and calendar algebra without copying membership into events
- adopted-release coverage/status and per-bundle population posture in the Sources panel
- local adopted-release history with source/bundle/calendar/member-event diffs
- canonical release-event diffs for added, removed, renamed, moved, status-changed, and newly-cancelled events, with before/after drill-down
- persisted generic refresh-attempt history with success/failure/incomplete states
- explicit Taria refresh health posture with running/healthy/stale/failed/interrupted/never-refreshed states and a visible 7-day stale threshold
- release-backed bundle/projected-calendar selectors in the recursive query editor
- repeatable Taria re-import using stable source-record and upstream event identity
- transactional source import reconciliation
- whole-release atomic Taria adoption/rollback across payloads, release metadata, CalendarSets, aliases, and memberships
- created / updated / unchanged / retained-missing accounting
- drag-and-drop Taria JSON import in the GUI
- standalone local import CLI
- CI enforcing rustfmt, compile, strict Clippy, and tests

The verified Phase 8 recurrence + RFC 5545 adapter checkpoint at 2026-10-06 is `41e8aa62a090df82751506cf2ab30436563d936a`: format, compile, strict Clippy, and **407 library tests** all pass.

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
CompositionLayer
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

The current bootstrap channel is `temporal-bundle-release:bootstrap:2026-10-04:r10`: 1,357 unique ready events, eight partially populated canonical domain slots, four explicit gap-only slots, and zero pending slots. The release combines compact reconciled recovery/frozen-rebuild indexes with full ReconciledProjectionEventSets. Ephemeris resolves and ingests that mixed release shape directly from the local Resourcearium checkout, persists CalendarSet membership, and renders the adopted release's coverage/posture in the Sources panel.

Production `bundle_artifacts[]` adoption is implemented and regression-tested with overlapping Politics/Finance projections: stable upstream identities converge on one canonical local event while bundle/calendar memberships remain separate. The live Resourcearium production channel is still unset, so this path is implemented and tested but not yet exercised against a live production release.

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
- Taria consumption is local-filesystem-first
- network/GitHub release discovery is optional future convenience, never required

## Run

```bash
cargo run
```

Ephemeris requires a graphical desktop session.

## Import / update Taria temporal data

Normal operation is filesystem-first.

Point Ephemeris once at either the local Taria repo or `incubator/resourcearium`. Ephemeris persists that path and the selected release channel. Then use **Update Taria Sources** from the toolbar or Sources panel.

The updater reads the local release registry and referenced local artifacts directly from disk. It does not download them.

A reconciled Resourcearium event-set JSON file can still be dropped onto the running application for one-off testing.

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
- [docs/TARIA_FILESYSTEM_WORKFLOW.md](docs/TARIA_FILESYSTEM_WORKFLOW.md) - local path setup and one-click Update Taria Sources workflow
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
