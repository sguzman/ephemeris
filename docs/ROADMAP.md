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

Implemented canonical extensions:

- canonical entity registry/resolution — schema v23 entity registry plus schema v24 Ephemeris-local participant bindings, source-ref/exact-label/manual resolution semantics, searchable registry management, guarded merge, reverse usage, stable entity-ID/type saved-view predicates, and canonical JSON v9 preservation

- structured event participants — schema v22 persistence/migration, validation, aggregate and field-specific query predicates, inspector display/editing for editable events, canonical JSON v7, and explicit CSV/VEVENT loss guards
- structured event location — schema v21 persistence/migration, validation/query/inspector support, canonical JSON v6, and explicit CSV/VEVENT loss guards

- immutable canonical event revision history — schema v20, atomic append-on-change semantics, persistence after live-event deletion, inspector history
- dedicated structured provenance records/tables — schema v19, queries, inspector authoring, canonical JSON v5
- user-owned annotations — schema v18, queries, inspector authoring, canonical JSON v4
- duplicate/entity identity assessments — schema v17, queries, inspector authoring, canonical JSON v3
- event relations and collections/sequences — domain types, schema-v15 persistence, query predicates, inspector editing, ordered sequence reordering, broader collection management, and topology-aware canonical JSON v2

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
- secondly / minutely / hourly / daily / weekly / monthly / yearly frequency
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
- BYHOUR expansion for `0..=23` on floating/exact date-time recurrence bases, preserving DTSTART minute/second and source-local DST semantics, applied before BYMINUTE/BYSECOND/BYSETPOS
- BYMINUTE expansion for `0..=59` on floating/exact date-time recurrence bases, preserving DTSTART second and duration, composing cartesianly with BYHOUR and applying before BYSECOND/BYSETPOS
- BYSECOND handling for ordinary civil seconds `0..=59` on floating/exact date-time recurrence bases: SECONDLY uses it as a limiter, while MINUTELY and coarser frequencies expand selected seconds before BYSETPOS; RFC 5545 leap-second value `60` is explicitly rejected until the time model can represent it faithfully
- SECONDLY frequency over floating/exact date-time bases, advancing in source-local wall-clock seconds; BYMONTH/BYYEARDAY/BYMONTHDAY/plain-BYDAY/BYHOUR/BYMINUTE/BYSECOND limit the active second, and BYSETPOS applies afterward; reachability uses an 86,400-second time-of-day fast path or modular cadence residues over the 400-year Gregorian date cycle when calendar limiters participate
- MINUTELY frequency over floating/exact date-time bases, advancing in source-local wall-clock minutes; BYMONTH/BYYEARDAY/BYMONTHDAY/plain-BYDAY/BYHOUR/BYMINUTE limit the active minute, BYSECOND expands it, and BYSETPOS applies afterward; reachability uses a 1,440-minute time-of-day fast path or modular congruence over the 400-year Gregorian minute cycle while scanning only civil dates when calendar limiters participate
- HOURLY frequency over floating/exact date-time bases, advancing in source-local wall-clock hours; BYMONTH/BYYEARDAY/BYMONTHDAY/plain-BYDAY/BYHOUR limit the active hour, BYMINUTE/BYSECOND expand it, and BYSETPOS applies afterward; reachability uses a 24-hour BYHOUR fast path or the finite 400-year / 3,506,328-hour Gregorian cycle when calendar limiters participate
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
- canonical-series recurrence editor in the inspector: frequency, interval, COUNT, UNTIL, WKST, plain BYDAY, BYMONTH, advanced selector fields, RDATE/EXDATE, and moved/cancelled occurrence overrides are directly editable; read-only sources are protected and save/remove routes through canonical-event recurrence validation
- structured occurrence-override controls layered over the canonical compact syntax: explicit Move / Cancel / Cancel + move / Keep rows with original/replacement fields, add/remove controls, and a collapsed raw-syntax fallback
- structured RDATE/EXDATE controls with one occurrence start per row, add/remove controls, live blank-row validation, and collapsed raw compact-syntax fallbacks
- one-click canonical recurrence presets for Daily, Weekdays, Weekly, Monthly, Last weekday/month, and Yearly; presets reset cadence selectors/interval while preserving bounds and exceptions
- contextual recurrence-selector affordances driven by the domain frequency/time-kind matrix: irrelevant empty advanced fields are hidden, incompatible populated values are preserved with explicit clearing, WKST appears only in active week contexts, and date/all-day series do not offer sub-daily frequencies
- structured ordinal-BYDAY authoring with one ordinal + weekday row per selector, add/remove controls, live validation, and a collapsed compact `1MO,-1FR` raw-syntax fallback
- structured BYMONTHDAY authoring with one signed civil-day row per selector, add/remove controls, live validation, and a collapsed compact `1,15,-1` raw-syntax fallback
- structured BYWEEKNO authoring with one signed week-number row per selector, add/remove controls, live validation, and a collapsed compact `20,-1` raw-syntax fallback
- structured BYYEARDAY authoring with one signed year-day row per selector, add/remove controls, live validation, and a collapsed compact `1,100,-1` raw-syntax fallback
- structured BYHOUR authoring with one civil-hour row per selector, add/remove controls, live validation, and a collapsed compact `9,17` raw-syntax fallback
- structured BYMINUTE authoring with one civil-minute row per selector, add/remove controls, live validation, and a collapsed compact `0,30` raw-syntax fallback
- structured BYSECOND authoring with one ordinary civil-second row per selector, add/remove controls, live validation, and a collapsed compact `0,15,30,45` raw-syntax fallback; leap-second value `60` remains deliberately unsupported
- structured BYSETPOS authoring with one signed position per row, add/remove controls, live validation, preset synchronization, and a collapsed compact `1,-1` raw-syntax fallback

Next Phase 8 work:

- event relations and collections/sequences over canonical event identity
- remaining RFC edge semantics and uncommon selector combinations beyond the implemented secondly/minutely/hourly/daily/weekly/monthly/yearly BY-part matrix, including deliberate `BYSECOND=60` deferral until leap-second timestamps are representable without coercion
- richer structured recurrence editing ergonomics beyond presets, structured exceptions, contextual selector visibility, and the fully structured advanced BY-selector set, especially higher-level controls for cadence/bounds and explanations for uncommon combinations
- recurrence-exception source-adapter interoperability for external calendar payloads is bidirectional for the supported RFC subset: RRULE, strict RDATE/EXDATE parse/format, strict RECURRENCE-ID original-slot parse/format, DTSTART/DTEND transport, RFC content-line and VEVENT/VCALENDAR envelopes, typed VEVENT binding, master/detached temporal-exception assembly, canonical TemporalEvent projection, strict VCALENDAR grouping/ingestion, canonical VEVENT/VCALENDAR export, transactional local-ICS source/store import, CLI import, and GUI drag/drop import are implemented; `RANGE=THISANDFUTURE`, occurrence-specific non-temporal overrides, DURATION transport, RDATE-only recurrence, VTIMEZONE, and VALARM remain deliberately unsupported until their canonical/preservation semantics exist; stored-source ICS export and explicit local-source refresh are implemented; non-Taria refresh history/diagnostics are next
- relations — canonical directed storage/query plus inspector create/delete UI and canonical JSON v2 interchange implemented
- collections/sequences — canonical ordered/unordered storage/query, inspector membership/create/manage UI, sequence reordering, and canonical JSON v2 interchange implemented
- uncertainty — schema-v16 bounded start-placement windows, canonical JSON preservation, inspector visibility, presence queries, and uncertain-start overlap queries implemented; recurring-event uncertainty and richer visualization/editing remain future work
- duplicate/entity resolution — candidate/same/distinct assessments implemented with confidence/rationale, query/UI, and canonical JSON
- advanced annotations — user-owned structured JSON annotations implemented
- structured provenance — typed assertion/source/provenance records implemented

## Phase 9 - External interoperability

Status: **substantially complete at the current core scope — bidirectional RFC 5545, local/remote ICS/Webcal, durable refresh diagnostics, conditional HTTP refresh, canonical JSON, and strict CSV are implemented; JSCalendar/jCal and CalDAV are deferred until concrete ecosystem value justifies them**

Goals:

- ICS import/export projection — transport, local import, stored-source CLI/GUI export, explicit local refresh, and durable refresh diagnostics implemented
- webcal/remote ICS ingestion — implemented for HTTP/HTTPS/webcal feeds with persisted ETag/Last-Modified conditional refresh and explicit HTTP 304 reporting
- CalDAV where valuable
- versioned native JSON import/export — v9 preserves canonical sources/events, topology, uncertainty, annotations, provenance, locations, participants, entities, and local participant/entity bindings; historical versions remain readable with explicit compatibility gates and transactional merge semantics
- CSV import/export for representable tabular subsets — implemented as strict CSV v1 with durable refresh-attempt history
- JSCalendar/jCal
- optional mobile/cloud bridges

## Phase 10 - Personal scheduling completeness

Status: **active — durable reminders, Busy/Free scheduling, conflict-aware authoring and occurrence-level alternative slots, uncertainty-aware availability diagnostics, richer quick-create, time editing, and structured location editing are implemented**

Implemented:

- richer local event creation with description, event type, domain, lifecycle status, Busy/Free behavior, and multi-day all-day exclusive-end authoring
- canonical time editing for writable non-recurring instant/floating/all-day/date-only events, with recurrence/uncertainty guardrails
- structured writable-event location editing
- event- and saved-view-targeted before-start reminder rules with durable delivery, dismissal, snooze, early wake, and separate Due/Snoozed/Upcoming states
- conflict-aware local event creation and time editing, plus new-blocking status/availability transitions with explicit second-save confirmation
- canonical Busy/Free event availability, query predicates, iCalendar TRANSP interop, recurrence-aware free/busy calculation, and constrained slot search
- persisted personal availability profile: work hours, duration, step, and workdays
- creation of local events directly from free intervals and suggested slots
- collision-time suggestions for definite timed events against the full canonical corpus, respecting workday/hour/step preferences and preserving draft metadata, source time kind, and exact duration
- collision-time suggestions for definite all-day events against the full canonical corpus, preserving civil-day duration across DST, respecting enabled start weekdays, and using background draft-safe evaluation
- conservative free/busy handling of bounded start-placement uncertainty, including possible-start windows extending beyond a representative event date
- actionable skipped-interval diagnostics with direct navigation to the affected event inspector
- focused original-slot recurrence editing for writable occurrences, including moved occurrences, without shifting the series master
- nonblocking alternative-slot suggestions for a definite recurring instance while preserving its other sister occurrences, source time kind, duration, and DST behavior
- save-time individual-occurrence conflict detection against the full canonical corpus, with explicit second-save confirmation of conflicting moves, cancellation, and cancelled-slot reactivation (including uncertain availability warnings)
- cancelled-occurrence restoration suggestions using the full Busy corpus with sister instances preserved, optional free original-slot selection, background search, civil all-day DST safeguards, and Save-time confirmation

Remaining goals:

- attendee/invitation exchange where justified by real scheduling workflows
- uncertainty-aware rescheduling only after its temporal and confirmation semantics are specified
- recurring master-time editing only when occurrence identity and existing exceptions can be preserved
- optional system notification delivery only with a fully specified local-first lifecycle
- careful extension of time editing to recurring masters only when exception identity can remain correct

## Always-on constraints

Every phase must preserve:

- local-first behavior
- provenance
- canonical identity
- correctness
- low interaction latency
- no accidental return to task-centric canonical storage
- no coupling of event organization to visibility/color/layout
