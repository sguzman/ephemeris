# Ephemeris

Ephemeris is a local-first temporal information system for treating events as canonical data and calendars as programmable views.

It is a native Rust + egui desktop application built for dense temporal corpora: personal scheduling, historical and institutional events, elections, courts, economics, sports, holidays, research timelines, and the temporal resources produced by Taria.

The central rule is simple:

> An event should exist once. Changing how it is viewed should not require duplicating or reorganizing the event itself.

## What Ephemeris is

Most calendar software organizes events into containers and then makes those containers carry too much meaning: ownership, visibility, grouping, color, and navigation all become entangled.

Ephemeris separates those concerns.

Canonical events live in a local SQLite store. Queries, saved views, overlays, grouping, sorting, color rules, and calendar layouts are projections over that store. A single event can therefore participate in many calendars without being copied into each one.

This matters especially for large temporal corpora where provenance, recurrence, lifecycle state, uncertainty, and source identity are part of the data rather than incidental metadata.

## Current state

Ephemeris is an active native application, not a design-only repository.

The current application has a canonical temporal store, recurrence engine and editor, programmable calendar views, Taria/Resourcearium ingestion, release-aware source history, and bidirectional RFC 5545 interoperability for the supported iCalendar subset.

Local `.ics` / `.ical` files can be imported through the CLI or by dropping them onto the running application. Imported calendars become first-class sources in the canonical store and refresh by stable source/UID identity.

The current verified implementation checkpoint is tracked in [docs/PROJECT_STATE.md](docs/PROJECT_STATE.md). Detailed historical milestone notes live in [docs/IMPLEMENTATION_HISTORY.md](docs/IMPLEMENTATION_HISTORY.md), not in this README.

## Architecture

Ephemeris is deliberately small at the platform level:

- Rust 2024
- native `eframe` / `egui`
- SQLite through bundled `rusqlite`
- local-first storage and processing
- no Tauri
- no React/WebView shell
- filesystem-first Taria integration

The important architectural boundary is between canonical temporal data and presentation.

```text
Sources
  Taria / ICS / future adapters
        |
        v
Ingestion + identity + normalization
        |
        v
Canonical temporal store
        |
        +--> recurrence / occurrences
        +--> provenance / source state
        +--> queries / saved views / overlays
        |
        v
Calendar surfaces
  grid / agenda / stream / timeline / density / table
```

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) and [docs/DATA_MODEL.md](docs/DATA_MODEL.md).

## Taria

Taria / Resourcearium is the primary rich upstream temporal-data producer.

Ephemeris consumes Taria artifacts from the local filesystem, validates release metadata and hashes, reconciles stable upstream identities into the local canonical store, preserves coverage state, and keeps CalendarSet membership separate from event identity.

Ephemeris is a consumer of that temporal knowledge. It is not intended to duplicate Taria's acquisition and reconciliation pipeline.

See [docs/TARIA_INTEGRATION.md](docs/TARIA_INTEGRATION.md) and [docs/TARIA_BUNDLE_CONTRACT.md](docs/TARIA_BUNDLE_CONTRACT.md).

## iCalendar interoperability

Ephemeris has a strict RFC 5545 adapter around the canonical temporal model.

For the supported subset it can ingest VCALENDAR / VEVENT payloads, group recurrence masters and detached instances by UID, map recurrence exceptions into canonical occurrence overrides, and export canonical events back to VEVENT / VCALENDAR.

Unsupported semantics are rejected rather than silently flattened. The exact supported boundary and deliberate omissions are documented in [docs/TIME_SEMANTICS.md](docs/TIME_SEMANTICS.md) and [docs/INGESTION_AND_SYNC.md](docs/INGESTION_AND_SYNC.md).

## Run

Ephemeris requires a graphical Linux desktop session.

```bash
cargo run
```

## Import data

### Taria

Point the application at a local Taria repository or Resourcearium directory and use **Update Taria Sources**.

A reconciled Taria artifact can also be imported directly:

```bash
cargo run --bin ephemeris-import -- path/to/reconciled-event-set.json
```

### iCalendar

Import a local calendar from the CLI:

```bash
cargo run --bin ephemeris-import -- path/to/calendar.ics
```

Or drop an `.ics` / `.ical` file onto the running application.

Refresh is identity-aware. Unchanged records stay unchanged, changed records update in place, and records absent from a later snapshot are retained unless the source semantics explicitly justify removal or cancellation.

## Verify

```bash
cargo fmt --check
cargo check
cargo clippy --all-targets -- -D warnings
cargo test
```

CI enforces the same gate.

## Documentation

The documentation is organized by purpose rather than by implementation chronology.

- [docs/PRODUCT.md](docs/PRODUCT.md) — product definition and scope
- [docs/PROJECT_STATE.md](docs/PROJECT_STATE.md) — concise current implementation state and immediate frontier
- [docs/ROADMAP.md](docs/ROADMAP.md) — staged direction
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — system boundaries and layering
- [docs/DATA_MODEL.md](docs/DATA_MODEL.md) — canonical temporal model
- [docs/TIME_SEMANTICS.md](docs/TIME_SEMANTICS.md) — recurrence, timezones, all-day and exception semantics
- [docs/QUERY_AND_VIEWS.md](docs/QUERY_AND_VIEWS.md) — programmable calendar behavior
- [docs/INGESTION_AND_SYNC.md](docs/INGESTION_AND_SYNC.md) — source refresh, reconciliation and export
- [docs/TARIA_INTEGRATION.md](docs/TARIA_INTEGRATION.md) — Taria consumer boundary
- [docs/UX.md](docs/UX.md) — interaction and visualization contract
- [docs/QUALITY.md](docs/QUALITY.md) — correctness and testing expectations
- [docs/IMPLEMENTATION_HISTORY.md](docs/IMPLEMENTATION_HISTORY.md) — archived detailed milestone ledger
- [docs/README.md](docs/README.md) — complete documentation map

## Product boundary

Ephemeris owns temporal data: events, occurrences, recurrence, lifecycle, sources, provenance, views, calendar visualization, imports, refreshes, and temporal history.

It is not a general productivity suite. Tasks, Kanban, contacts, and unrelated workspaces belong elsewhere unless they directly participate in temporal semantics.

Ephemeris descends from Rivetr's calendar work, but it does not inherit Rivetr's task-centric canonical model. See [docs/LINEAGE.md](docs/LINEAGE.md).

## Design test

When adding a feature, ask two questions:

1. Does this make Ephemeris better at locally interrogating, understanding, editing, or visualizing temporal data?
2. Can presentation change without duplicating or physically reorganizing canonical events?

If the first answer is no, the feature is probably outside scope. If the second answer is no, the design is probably wrong.
