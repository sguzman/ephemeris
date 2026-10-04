# Roadmap

## Roadmap philosophy

Ephemeris should become useful against real Taria temporal data early.

Do not spend months rebuilding every Rivetr calendar feature before establishing the canonical event model and ingestion boundary.

## Phase 0 - Documentation and inheritance audit

Status: **complete**

Completed:

- product identity and documentation contract
- architecture and temporal model
- Rivet/Rivetr/Ephemeris lineage
- focused Rivetr calendar implementation audit
- explicit reuse/adapt/reject inheritance map
- SQLite storage ADR

## Phase 1 - Native application skeleton

Status: **substantially complete**

Implemented:

- Rust application
- `eframe`/`egui`
- native local data directory
- SQLite bootstrap
- native UI-state persistence
- CI
- test harness
- visible import/error/status messaging
- no Tauri/React/WebView dependency

Still to deepen:

- structured runtime logging/observability
- richer configuration surface

## Phase 2 - Canonical temporal core

Status: **active / substantial foundation implemented**

Implemented:

- SQLite schema and migrations
- `TemporalEvent`
- `TemporalSource`
- stable local IDs
- source-record identity
- lifecycle status
- rich extensible properties
- source/provenance reference retention
- explicit date-only/all-day/instant/floating/month/year/unresolved semantics
- indexed date/source/status/domain querying
- saved-view storage

Still required:

- `EventOccurrence`
- recurrence engine
- dedicated provenance records/tables
- snapshot/history tables
- annotations
- relations/collections
- richer indexed ontology

## Phase 3 - Taria ingestion vertical slice

Status: **first vertical slice working**

Implemented:

- import of real Resourcearium reconciled temporal event sets
- Taria projection/source identity retention
- assertion/source/provenance refs
- source contexts and rich Taria properties
- blocked/unplaced event preservation
- transactional import
- repeatable identity-aware re-import
- created/updated/unchanged/retained-missing accounting
- GUI drag/drop import
- CLI import
- provenance-rich event inspector

Still required:

- richer native interchange/version evolution
- snapshot/history persistence
- source health
- rollover state
- additional Taria artifact families

## Phase 4 - Calendar views

Status: **substantial**

Implemented date ranges:

- Year
- Quarter
- Month
- Week
- Day

Implemented layouts:

- Grid
- Agenda

Also implemented:

- navigation
- keyboard shortcuts
- imprecise month/year presentation without fake dates
- separate unplaced/conflicted surface
- source visibility
- event inspection

Still required:

- stronger dense-day aggregation
- compact agenda modes
- richer layout customization

## Phase 5 - Query and saved views

Status: **active**

Implemented:

- text search
- domain facet
- jurisdiction facet
- lifecycle-status facet
- source visibility as independent dimension
- canonical `EventQuery` model
- durable named `SavedView`
- SQLite saved-view persistence
- saved view create/apply/update/delete
- saved date-range mode
- saved layout
- saved timezone/week-start/source visibility

Next:

- grouping
- stable multi-key sorting
- independent color rules
- richer facets
- boolean/nested query representation
- view composition/calendar algebra

Exit criterion remains:

- user can create durable conceptual calendars without duplicating events

## Phase 6 - Source management and history

Status: **not started beyond source identity/import metadata**

Goals:

- source inspector
- refresh state
- snapshot diff
- added/moved/cancelled changes
- stale/rollover states

## Phase 7 - Rich temporal visualization

Status: **started through Agenda**

Goals:

- compact agenda
- table mode
- chronological stream
- timeline
- heatmap/density
- user-defined columns
- pivot/summary foundation

## Phase 8 - Advanced temporal semantics

Status: **not started**

Goals:

- full recurrence behavior
- recurrence exceptions
- relations
- collections/sequences
- uncertainty
- duplicate/entity resolution
- advanced annotations

## Phase 9 - External interoperability

Status: **not started**

Goals:

- ICS import/export projection
- webcal/remote ICS ingestion
- CalDAV where valuable
- JSCalendar/jCal
- CSV/JSON import/export
- optional mobile/cloud bridges

## Phase 10 - Personal scheduling completeness

Status: **not started**

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
- no coupling of event organization to visibility/color/layout
