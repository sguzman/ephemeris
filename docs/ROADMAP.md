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
- general-purpose snapshot/history tables beyond the implemented Taria release snapshot model
- annotations
- relations/collections
- richer indexed ontology

## Phase 3 - Taria ingestion and bundle-release adoption

Status: **filesystem-first release adoption, membership queries, release posture/history UI, whole-release atomicity, and nonblocking worker execution working**

Implemented:

- import of real Resourcearium reconciled temporal event sets
- import of pinned CompactReconciledEventIndex recovery payloads
- persisted local Resourcearium root + release channel
- auto-detection of local/sibling Taria checkout
- local release-registry/channel/manifest resolution
- local artifact path confinement + SHA-256 verification
- one-click Update Taria Sources UI
- explicit skipped-shard reporting
- immutable release metadata persistence
- immutable CalendarSet persistence + release association
- projected-calendar/event-membership persistence
- upstream event/reconciled identity aliasing
- cross-bundle canonical-event deduplication
- production `bundle_artifacts[]` adoption
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
- current-release bundle membership query predicate
- projected CalendarSet membership query predicate
- membership-aware base queries, overlays, color rules, and calendar algebra
- membership context remains separate from canonical event ownership
- adopted-release coverage/status UI
- per-bundle partial/pending/gap-only posture rendering
- release-backed bundle selector in query predicates
- release-backed projected-calendar selector in query predicates
- whole-release SQLite transaction across release metadata, all payloads, CalendarSets, aliases, and memberships
- rollback of every earlier mutation if a later shard/artifact fails
- regression coverage proving failed multi-shard releases leave no partial adoption state
- background worker execution for **Update Taria Sources**
- separate worker SQLite connection against the WAL database
- disabled/update-in-progress UI state while adoption runs
- nonblocking result polling and post-commit reload
- adopted release history from SQLite
- previous same-channel release comparison
- persisted release-to-source projection associations
- source projection rollover diffs
- immutable canonical event snapshots per adopted release
- exact release-present event capture, excluding retained-missing local records
- canonical event add/remove/move/status/newly-cancelled diffs
- bundle/projected-calendar/resolved-member-event membership diffs

Now established upstream:

- immutable TemporalBundleRelease schema/registry
- bootstrap and production release channels
- bootstrap-partial integration release
- production-partial / production-complete packaging
- canonical 13-projection bundle build
- CalendarSet packaging
- release validation and integrity hashes

Next Ephemeris implementation:

- richer per-field event-diff presentation and drill-down on top of the snapshot model
- persistent refresh-failure history and explicit stale/health policy

Still later:

- additional Taria artifact families as Resourcearium evolves

## Immediate program priority

The release-adoption execution boundary is now substantially complete: whole-release atomicity, off-frame-loop worker execution, and membership-level release history/diff inspection are implemented.

Resourcearium is independently building/populating frozen bundles. Ephemeris should consume those releases rather than duplicate acquisition work.

The consumer contract is frozen in:

- `docs/TARIA_BUNDLE_CONTRACT.md`

The local filesystem transport, bootstrap r10 mixed-payload adoption, CalendarSet persistence, identity aliasing, production overlap-safe adoption, membership-aware programmable query predicates, release-posture UI, and ergonomic membership selectors are implemented.

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

## Deferred UI/layout debt

The native egui shell needs a dedicated responsive-layout pass after the current programmable-view work. A 2026-10-05 real-app screenshot demonstrated a severe failure mode at constrained width: Sources/Inspector and presentation controls can squeeze the calendar into a narrow strip, controls/text overlap, labels wrap into near-vertical fragments, and the resulting surface is functionally unreadable.

This is explicitly tracked as a product bug, not accepted polish. The later layout pass should establish minimum/maximum side-panel widths, sane collapse/scroll behavior, non-overlapping toolbar/presentation controls, and a protected minimum width for the calendar canvas.

## Phase 5 - Query and saved views

Status: **substantially complete**

Implemented:

- text search
- domain facet
- jurisdiction facet
- event-type facet
- institution facet
- renderability facet
- exact tag-membership facet
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
- configurable ordered visible-column schema over 16 event/presentation fields
- add/hide/reorder/reset Table-column controls
- direct row selection into the existing inspector
- reuse of saved query, sort, grouping, semantic color, and Table-column state
- true grouping partitions independent from sorting
- transient UI-state persistence with backward-compatible defaults
- SavedView + SQLite persistence through schema v10

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

The composition-layer GUI editor is implemented: layers can be enabled/disabled, renamed, assigned Union/Intersect/Subtract, reordered, deleted, and edited through the recursive query editor.

Implemented saved-view-reference composition:

- composition layers can reference another SavedView by stable UUID;
- referenced logical sets recursively include query, source visibility, composition, and overlays;
- referenced presentation state is not inherited;
- missing references are explicit no-ops;
- direct/indirect cycles are rejected on save and at the store boundary;
- runtime evaluation defensively skips cyclic edges in corrupt/external data.

Next major work moves into Phase 6 source management/history. Programmable-view inheritance is no longer the blocking Phase 5 boundary.

Exit criterion remains:

- user can create durable conceptual calendars without duplicating events

## Phase 6 - Source management and history

Status: **active / substantial first slice implemented**

Implemented:

- source inspector over canonical `TemporalSource` metadata
- source visibility kept independent from inspection selection
- local refresh timestamp and upstream-generation metadata
- canonical event counts across direct ownership and import-record mappings
- release-to-source projection persistence in schema v11
- current-release versus historical source posture
- source projection rollover diffs between adjacent releases
- immutable canonical event snapshots in schema v12
- exact release-present snapshot membership from importer-reported canonical UUIDs
- cross-projection deduplication before snapshot capture
- snapshot capture inside the whole-release atomic transaction
- canonical event add/remove diffs
- moved-event detection from temporal-state changes
- lifecycle-status change detection
- newly-cancelled event detection
- release-history UI summary for source and canonical-event changes
- regression coverage for overlapping-source counts, snapshot deduplication, replay stability, and rollback

Remaining Phase 6 work:

- richer changed-event drill-down beyond counts/IDs
- persistent refresh failure/attempt history
- explicit stale/health policy and thresholds
- broader non-Taria source refresh/history integration

## Phase 7 - Rich temporal visualization

Status: **started through Agenda and dense Table**

Implemented:

- Agenda layout
- dense Table layout
- true group partitioning in dense views
- direct selection into the event inspector
- ordered configurable Table columns with UI-state and SavedView persistence

Goals:

- compact agenda
- chronological stream
- timeline
- heatmap/density
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
