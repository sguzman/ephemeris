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

Status: **substantial foundation implemented; recurrence continued in Phase 8**

Implemented:

- SQLite schema and migrations
- `TemporalEvent`
- `TemporalSource`
- `EventOccurrence` materialization with stable canonical-event lineage
- stable local IDs
- source-record identity
- lifecycle status
- rich extensible properties
- source/provenance reference retention
- explicit date-only/all-day/instant/floating/month/year/unresolved semantics
- indexed date/source/status/domain querying
- saved-view storage
- recurrence foundation and exception-aware occurrence materialization, detailed under Phase 8

Still required:

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

- broader non-Taria source refresh/history integration when those adapters exist

Still later:

- general-purpose provenance/history beyond the implemented Taria release snapshot model
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
- rename detection with before/after titles
- moved-event detection with before/after temporal values
- lifecycle-status change detection with before/after status
- newly-cancelled event detection
- expandable release-history drill-down over canonical event changes
- generic persistent refresh-attempt history in schema v13
- success/failure/incomplete refresh records that survive release rollback
- running versus interrupted refresh distinction
- explicit Taria refresh-health states
- visible 7-day Taria stale threshold
- regression coverage for overlapping-source counts, snapshot deduplication, replay stability, rollback, refresh history, and health policy

Remaining Phase 6 work:

- broader non-Taria source refresh/history integration

## Phase 7 - Rich temporal visualization

Status: **complete at the current foundation scope**

Implemented:

- Agenda layout
- Compact Agenda high-density one-line layout
- chronological Stream layout with fixed typed temporal ordering and date/precision markers
- proportional Timeline with exact-point, range, all-day/date-only, month-precision, and year-precision placement
- scale-aware Timeline ticks for Year/Quarter/Month/Week/Day
- Density/heatmap layout using genuine per-day occurrence counts and explicit coarse-precision accounting
- Density day drill-down into Agenda + Day
- pivot-style Summary layout driven by the existing GroupBy dimension
- Summary counts, shares, earliest/latest positioned dates, and temporal-precision breakdown
- persistence of the new layouts through the existing UI-state/SavedView layout boundary
- dense Table layout
- true group partitioning in configurable dense views
- direct selection into the event inspector where the surface represents individual events
- ordered configurable Table columns with UI-state and SavedView persistence

The roadmap can now move to Phase 8 temporal semantics. Future visual variants (for example more elaborate continuous/Gantt-style timelines) are enhancements rather than blockers for the Phase 7 foundation.

## Phase 8 - Advanced temporal semantics

Status: **active - recurrence selectors and exceptions implemented**

Implemented recurrence foundation:

- recurrence definitions persisted on canonical events in schema v14
- daily / weekly / monthly / yearly frequency
- interval, count, and inclusive-until bounds
- daily BYDAY filtering over active interval days
- daily BYMONTH limiting over active interval days, including composition with daily BYDAY and finite Gregorian-cycle reachability detection
- signed daily BYMONTHDAY limiting for `-31..=-1` and `1..=31`, intersecting with daily BYMONTH/BYDAY and sharing the finite Gregorian-cycle reachability guard
- weekly BYMONTH limiting after weekly BYDAY expansion, including cross-month week filtering before BYSETPOS and finite 400-year / 20,871-week reachability detection
- monthly BYMONTH limiting over active recurrence months, with a 4,800-month Gregorian-cycle reachability guard shared across monthly BYDAY/BYMONTHDAY/ordinal-BYDAY/BYSETPOS candidate semantics
- yearly selector reachability over the finite 400-year Gregorian cycle, covering permanently empty BYMONTH/BYMONTHDAY/BYYEARDAY/BYDAY/BYWEEKNO/BYSETPOS combinations without weakening first-year DTSTART filtering
- weekly multi-day BYDAY selection with explicit WKST recurrence-week anchoring
- monthly plain BYDAY expansion for every matching weekday in the active month, including composition with ordinal BYDAY, BYMONTHDAY filtering, and BYSETPOS
- signed monthly BYMONTHDAY selection for `-31..=-1` and `1..=31`, including month-end-relative selectors, resolved-date ordering/deduplication, and impossible-date skipping
- monthly ordinal BYDAY selection for `±1..±5` weekdays, including last-weekday forms and missing-fifth skipping
- BYMONTHDAY + monthly ordinal BYDAY intersection over resolved civil dates before occurrence counting/exceptions
- positive yearly BYMONTH selection for months 1-12 while preserving DTSTART's civil day where valid
- signed yearly BYMONTHDAY expansion across every month when BYMONTH is absent, or across only selected months when BYMONTH is present
- yearly BYMONTH + signed BYMONTHDAY composition over selected months
- yearly plain BYDAY expansion across the active recurrence year, with BYMONTH limiting expansion to selected months and BYYEARDAY acting as an intersecting filter
- context-aware yearly ordinal BYDAY: `±1..±53` resolves against the whole year when BYMONTH is absent; BYMONTH retains month-scoped `±1..±5` semantics; numeric BYDAY remains invalid with BYWEEKNO
- yearly BYMONTH + ordinal BYDAY composition with ordinal weekdays resolved within each selected month
- yearly BYMONTH + BYMONTHDAY + ordinal BYDAY intersection over resolved civil dates
- signed yearly BYWEEKNO selection for `-53..=-1` and `1..=53`, including WKST-aware week-year numbering, yearly plain-BYDAY expansion, missing-week-53 skipping, and filtering by existing yearly selectors
- signed yearly BYYEARDAY selection for `-366..=-1` and `1..=366`, including leap-year invalid-date skipping and filtering by existing yearly month-scoped selectors
- BYHOUR expansion for `0..=23` on floating/exact date-time recurrence bases, preserving DTSTART minute/second and source-local DST semantics, applied before BYMINUTE/BYSETPOS
- BYMINUTE expansion for `0..=59` on floating/exact date-time recurrence bases, preserving DTSTART second and duration, composing cartesianly with BYHOUR and applying before BYSETPOS
- signed BYSETPOS selection for `-366..=-1` and `1..=366`, applied generically after the supported BY-selector candidate set is resolved within each recurrence interval
- active-window occurrence expansion rather than eager infinite materialization
- deterministic occurrence identity and canonical-event lineage
- date-only / all-day / floating / exact-time recurrence
- source-wall-clock preservation for zoned exact recurrence across DST
- invalid monthly/yearly calendar dates skipped without inventing replacement dates
- persistence-boundary recurrence validation
- RDATE additions and EXDATE exclusions
- moved occurrence overrides with identity anchored to the original recurrence slot
- cancelled occurrence overrides retained as queryable cancelled facts
- moved-in exception materialization when the original slot lies outside the active view horizon
- override-target validation so detached overrides cannot manufacture occurrences outside the RRULE/RDATE set
- recurrence definition + occurrence origin/index/identity/override inspection in the GUI

Next Phase 8 work:

- broader RRULE dimensions and selector families beyond daily/weekly/monthly/yearly plain BYDAY, all-frequency BYMONTH, daily/monthly/yearly BYMONTHDAY, month/year-scoped ordinal BYDAY, BYWEEKNO/BYYEARDAY/BYHOUR/BYMINUTE/BYSETPOS, and the implemented selector combinations plus recurrence-exception interoperability
- richer recurrence/exception authoring and editing
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
