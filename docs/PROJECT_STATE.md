# Project State

Last verified implementation milestone: 2026-10-04.

## Current status

Ephemeris is now a working native Rust + egui calendar foundation backed by an event-native SQLite temporal store.

The original documentation/inheritance phase is complete. The project has crossed the important architectural boundary that motivated the spinoff from Rivetr: calendar data is no longer forced through `TaskDto` or task-file persistence.

The current application can ingest real Taria Resourcearium reconciled temporal event sets, preserve rich upstream temporal semantics and provenance references, query the resulting corpus, render standard calendar ranges, inspect records, and persist named programmable calendar views.

## Verified quality gate

The 2026-10-04 milestone passes:

```bash
cargo fmt --check
cargo check
cargo clippy --all-targets -- -D warnings
cargo test
```

At the checkpoint, all 26 tests pass.

## Implemented architecture

### Native application

- Rust 2024
- `eframe` / `egui`
- no Tauri
- no React/WebView shell
- native local UI-state persistence

### Canonical local store

SQLite via bundled `rusqlite`.

Current schema version: 4.

The database owns:

- temporal sources
- temporal events
- durable saved views

Transient UI state remains separate.

### Canonical temporal model

Implemented time forms:

- date-only civil date/range
- explicit all-day date/range
- exact UTC instant with retained source-timezone context
- floating/local date-time
- month precision
- year precision
- unresolved/blocked temporal value

Month/year precision is not coerced onto an invented day. Date-only data is not silently promoted to an instant or conflated with explicit all-day semantics.

Implemented event metadata includes:

- canonical local UUID
- source UUID
- source-record identity
- Taria event ref
- Taria reconciled-event key
- assertion refs
- source refs
- provenance refs
- renderability
- normalized/raw title
- description
- event type
- domain
- jurisdiction
- institution
- lifecycle status
- confidence
- importance
- personal relevance
- tags
- extensible JSON properties

### Taria ingestion

Implemented against the real Resourcearium reconciled-event-set shape.

The importer supports:

- projection identity
- reconciled projection identity
- regular and top-level blocked events
- date precision
- date ranges
- month precision
- year precision
- offset date-times
- timezone/local date-times
- unresolved temporal values
- source contexts
- geography
- field-resolution/provenance payload retention

Import behavior:

- transactional per source artifact
- stable source-record identity
- creates new records
- updates changed records in place
- distinguishes unchanged records
- retains records missing from later snapshots rather than assuming deletion/cancellation
- reports blocked/unplaced and imprecise counts
- chooses a useful initial focus date after import

Entry surfaces:

- drag/drop reconciled JSON onto the GUI
- `ephemeris-import` CLI

### Calendar presentation

Date-range modes:

- Year
- Quarter
- Month
- Week
- Day

Layouts are independent from date range:

- Grid
- Agenda

That separation is deliberate. A Month view can be rendered as a grid or agenda; presentation does not define the temporal query.

### Querying

Current simple query surface:

- text search
- domain
- jurisdiction
- lifecycle status
- source visibility

Source visibility is independent from the event query.

The advanced boolean/query-expression system is not implemented yet.

### Saved views

Named saved views are real durable product objects stored in SQLite.

A saved view currently retains:

- event query
- hidden/visible source selection
- date-range mode
- layout
- display timezone
- week-start behavior

Saved views can be:

- created
- applied
- updated from current state
- deleted

They do not copy or own events.

Legacy saved-view data briefly stored in `ui-state.json` is migrated into SQLite on startup.

### Inspection

The event inspector exposes the normalized event plus rich Taria identity/provenance context.

Unplaced/conflicted events remain visible and inspectable rather than being dropped or assigned fake calendar dates.

## Completed inheritance work

The focused Rivetr calendar audit is complete.

Reused/adapted ideas:

- date navigation/math
- native calendar rendering patterns
- year/quarter/month/week/day behavior
- keyboard-first interaction
- local persisted UI state
- import reconciliation lessons

Explicitly rejected as canonical Ephemeris architecture:

- `TaskDto` as event
- task statuses as complete temporal lifecycle
- tags as the only ontology/provenance structure
- task datastore as temporal store
- unrelated Rivetr workspaces

See `IMPLEMENTATION_AUDIT_2026-10-04.md` and `RIVETR_INHERITANCE.md`.

## Important incomplete areas

The current milestone is a foundation, not the finished calendar.

Not yet implemented:

- arbitrary boolean query algebra
- grouping rules
- multiple sort rules
- independent color-rule sets
- saved-view inheritance/composition
- overlays/calendar algebra
- event-occurrence/recurrence engine
- dedicated provenance/snapshot/history tables
- snapshot diffs
- source health/rollover
- annotations
- relations and collections
- duplicate/entity resolution
- timeline/table/heatmap/pivot views
- reminders
- ICS/webcal/CalDAV ingestion/export
- full personal event editing

## Immediate next implementation boundary

Build the next layer of programmable-view semantics without coupling it to event storage:

1. grouping foundation
2. stable sort-rule foundation
3. independent color strategy/rule foundation
4. persist those dimensions in saved views
5. apply them first to Agenda and existing calendar event rendering

After that, expand the query representation toward boolean composition and add richer dense-data views.

## Rule going forward

Do not regress to a container-centric calendar.

The same canonical event corpus must remain independently:

- filterable
- groupable
- sortable
- colorable
- renderable in multiple layouts
- reusable by unlimited saved views
