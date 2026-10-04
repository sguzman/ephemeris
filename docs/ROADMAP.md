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

## Phase 3 - Taria ingestion and bundle-release adoption

Status: **direct event payload slice working; release adoption next**

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

Now established upstream:

- immutable TemporalBundleRelease schema/registry
- bootstrap and production release channels
- bootstrap-partial integration release
- production-partial / production-complete packaging
- canonical 13-projection bundle build
- CalendarSet packaging
- release validation and integrity hashes

Next Ephemeris implementation:

- TemporalBundleRelease v1 manifest importer
- support bootstrap `shards[]` and production `bundle_artifacts[]`
- resolve full ReconciledProjectionEventSet and accepted CompactReconciledEventIndex payloads from release artifacts
- CompactReconciledEventIndex adapter for recovered bootstrap shards
- CalendarSet membership persistence
- release/channel/coverage metadata persistence
- partial/pending/gap-only UI posture
- atomic hash/fingerprint validation
- release-to-release adoption/diff foundation

Still later:

- snapshot/history persistence
- source health/rollover presentation
- additional Taria artifact families as Resourcearium evolves

## Immediate program priority

The highest-priority integration boundary is now Taria **TemporalBundleRelease v1** consumption.

Resourcearium is independently building/populating frozen bundles. Ephemeris should consume those releases rather than duplicate acquisition work.

The consumer contract is frozen in:

- `docs/TARIA_BUNDLE_CONTRACT.md`

Current low-level direct reconciled-event-set import remains valid and will be reused underneath the release importer. Bootstrap r3 also requires a CompactReconciledEventIndex adapter for its recovered Politics/Holidays shards.

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
- Table

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

Implemented presentation dimensions:

- grouping independent from filtering
- stable multi-key sorting
- semantic coloring independent from grouping/source visibility
- persistence of grouping/sorting/color in saved views
- application across Grid, Agenda, and Table layouts

Implemented query algebra:

- nested AND / OR / NOT expressions
- typed text predicates
- text/status set membership
- integer comparisons
- exists/missing checks
- recursive GUI query editor
- saved-view persistence
- backward-compatible legacy query loading

Implemented temporal query predicates:

- explicit query context with display timezone and today anchor
- temporal-kind membership
- civil-date overlap
- timezone-aware exact-instant date evaluation
- explicit month/year imprecise-span inclusion
- relative date windows with deterministic day offsets
- editor presets for Today, Next 7, Next 30, and Previous 7 days

Implemented dense table foundation:

- reusable Table layout over the same event corpus
- Taria-oriented core columns
- direct row selection into the existing inspector
- reuse of saved query, sort, grouping, and semantic color state
- true grouping partitions independent from sorting

Implemented color-rule engine:

- recursive query expressions as rule conditions
- ordered first-match precedence
- explicit RGB rule colors
- semantic ColorBy fallback
- base-view and overlay-specific rule sets
- SQLite saved-view persistence
- full editor controls

Implemented overlay foundation:

- independent embedded overlay queries
- union with base result set without copied membership
- overlay styling precedence
- independent overlay fallback coloring and color rules
- saved-view persistence and UI editing

Implemented calendar-algebra foundation:

- ordered `CompositionLayer` model
- union
- intersection
- subtraction
- deterministic sequential evaluation over the base query
- runtime visibility integration
- UI-state persistence
- SavedView persistence
- SQLite schema v8 persistence and migration
- tests for ordering and disabled layers

The composition-layer GUI editor is not implemented yet.

Next after the Taria release-consumer slice:

- composition-layer GUI editor
- richer facets
- user-defined table columns
- saved-view inheritance with cycle-safe semantics

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

Status: **started through Agenda and dense Table**

Implemented:

- Agenda layout
- dense Table layout
- true group partitioning in dense views
- direct selection into the event inspector

Goals:

- compact agenda
- chronological stream
- timeline
- heatmap/density
- user-defined Table columns
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
