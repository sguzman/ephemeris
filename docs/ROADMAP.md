# Roadmap

## Roadmap philosophy

Ephemeris should become useful against real Taria temporal data early.

Do not spend months rebuilding every Rivetr calendar feature before establishing the canonical event model and ingestion boundary.

## Phase 0 - Documentation and inheritance audit

Status: active at project creation.

Goals:

- establish product identity
- document architecture and temporal model
- document Rivet/Rivetr lineage
- inventory reusable Rivetr calendar code
- identify task-backed assumptions that must not become canonical

Exit criteria:

- documentation surface exists
- first implementation boundary is explicit
- Rivetr is treated as source material, not a dependency by accident

## Phase 1 - Native application skeleton

Goals:

- Rust workspace
- `eframe`/`egui` app
- logging
- config
- local data directory
- CI
- test harness
- visible diagnostics/status

Prefer extraction of proven Rivetr shell/calendar utilities where appropriate.

Exit criteria:

- app starts natively
- empty temporal store opens
- no Tauri/React/WebView dependency

## Phase 2 - Canonical temporal core

Goals:

- choose embedded storage engine via ADR
- implement schema/migrations
- TemporalEvent
- EventOccurrence
- TemporalSource
- provenance/snapshot minimum
- annotation minimum
- stable IDs
- time semantics

Exit criteria:

- events can be persisted and queried without task-model mediation
- tests cover identity and time semantics

## Phase 3 - Taria ingestion vertical slice

Goals:

- define versioned Taria temporal interchange
- ingest one real Taria temporal resource family
- preserve source/provenance
- repeatable refresh
- source status
- event inspector

Exit criteria:

- real Taria events render locally
- re-import/refresh does not duplicate unchanged events
- provenance is inspectable

## Phase 4 - Calendar views

Goals:

- year
- quarter
- month
- week
- day
- agenda
- dense-day handling

Reuse/adapt proven Rivetr calendar rendering where useful.

Exit criteria:

- real corpus is navigable across standard calendar views
- dense days remain usable

## Phase 5 - Query and saved views

Goals:

- facets
- boolean query representation
- text search
- saved views
- independent color rules
- group/sort foundation

Exit criteria:

- user can create durable conceptual calendars without duplicating events

## Phase 6 - Source management and history

Goals:

- source inspector
- refresh state
- snapshot diff
- added/moved/cancelled changes
- stale/rollover states

Exit criteria:

- source-backed temporal data is auditable over time

## Phase 7 - Rich temporal visualization

Goals:

- compact agenda
- table mode
- chronological stream
- timeline
- heatmap/density
- user-defined columns
- pivot/summary foundation

## Phase 8 - Advanced temporal semantics

Goals:

- full recurrence behavior
- recurrence exceptions
- relations
- collections/sequences
- uncertainty
- duplicate/entity resolution
- advanced annotations

## Phase 9 - External interoperability

Goals:

- ICS export/projection
- webcal/remote ICS ingestion
- CalDAV where valuable
- JSCalendar/jCal
- CSV/JSON import/export
- optional mobile/cloud bridges

## Phase 10 - Personal scheduling completeness

Goals:

- rich local event editing
- reminders/query-based notification rules
- attendees/invitations where justified
- availability/free-busy
- personal scheduling workflows

## Always-on constraints

Every phase must preserve:

- local-first behavior
- provenance
- canonical identity
- correctness
- low interaction latency
- no accidental return to task-centric canonical storage
