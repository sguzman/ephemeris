# Project State

Last verified implementation milestone: 2026-10-04.

## Current status

Ephemeris is now a working native Rust + egui calendar foundation backed by an event-native SQLite temporal store.

The original documentation/inheritance phase is complete. The project has crossed the important architectural boundary that motivated the spinoff from Rivetr: calendar data is no longer forced through `TaskDto` or task-file persistence.

The current application can ingest real Taria Resourcearium reconciled temporal event sets and compact recovered reconciled indexes, preserve rich upstream temporal semantics and provenance references, query the resulting corpus, render standard calendar ranges, inspect records, and persist named programmable calendar views. Taria has established immutable TemporalBundleRelease packaging, and Ephemeris now has the first filesystem-first release consumer slice: local Resourcearium discovery, local channel/manifest resolution, hash validation, and one-click supported-shard adoption.

## Verified quality gate

The 2026-10-04 milestone passes:

```bash
cargo fmt --check
cargo check
cargo clippy --all-targets -- -D warnings
cargo test
```

At the release-posture/query-selector checkpoint, all **50** library tests pass. Verified implementation head: `0392969043ec8684f2f9b2abf78c0612da733476`, with format, check, strict Clippy, and tests all green.

## Implemented architecture

### Native application

- Rust 2024
- `eframe` / `egui`
- no Tauri
- no React/WebView shell
- native local UI-state persistence

### Canonical local store

SQLite via bundled `rusqlite`.

Current schema version: 9.

The database owns:

- temporal sources
- temporal events
- durable saved views
- Taria source/import-record mappings
- Taria upstream event/reconciled identity aliases
- immutable Taria release metadata
- immutable CalendarSets
- release-to-CalendarSet associations
- projected calendars
- CalendarSet event memberships

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

- one-click **Update Taria Sources** against a configured local Resourcearium tree
- automatic sibling/local Taria checkout detection
- drag/drop reconciled JSON onto the GUI
- `ephemeris-import` CLI

### Taria bundle-release contract

Resourcearium now owns a consumer-facing `TemporalBundleRelease` layer over frozen derivative products.

The accepted Ephemeris contract is documented in:

- `docs/TARIA_BUNDLE_CONTRACT.md`

Current upstream state:

- 429 canonical temporal Resources
- 177 direct single-Resource RICS profiles in the first exact-lineage production tranche
- canonical 13-projection build tooling
- frozen release packaging and validation
- bootstrap channel currently pointing at `temporal-bundle-release:bootstrap:2026-10-04:r10`
- bootstrap posture: 1,357 unique ready events, 15 represented canonical Resources, 38 selected ingestion profiles, 196 recovered source surfaces, 8 partial canonical domain slots, 4 explicit gap-only slots, 0 pending slots
- production channel independently advances as live acquisition-backed releases become available

Important consumer rule:

- ReconciledProjectionEventSet carries event payload.
- CalendarSet carries membership/navigation metadata.
- overlapping bundle membership must never clone event identity.

The direct reconciled-event-set importer and CompactReconciledEventIndex importer are now low-level payload adapters beneath the local release updater.

Current bootstrap r10 update behavior:

- recovered U.S. Politics and U.S. Holidays are consumed through pinned `CompactReconciledEventIndex` payloads;
- European Elections and U.S. Sports are consumed through full `ReconciledProjectionEventSet` payloads;
- additional Economics, Finance, Business, Culture, and Education frozen-rebuild shards are consumed through compact reconciled indexes;
- Science/Technology/Space, Public Health, Environment/Weather, and Transportation/Civic Infrastructure are explicitly represented as gap-only rather than silently empty;
- all populated bootstrap shards expose accepted post-reconciliation payloads;
- CalendarSets and their event memberships are persisted independently from canonical event identity;
- bootstrap content-fingerprint semantics are validated exactly as Resourcearium defines them;
- local artifact paths remain confined to the configured Resourcearium tree.

Production support is also implemented:

- `bundle_artifacts[]` are resolved from the same filesystem updater;
- production file SHA-256 semantics are validated;
- upstream event/reconciled aliases provide global deduplication across overlapping projections;
- one canonical event can therefore carry several release/calendar memberships without cloning;
- the production overlap regression test proves one shared event across Politics and Finance remains one local event with two memberships.

The live Resourcearium production channel is still unset, so this production path is implemented and tested synthetically but not yet exercised against a live production release.

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
- Table

That separation is deliberate. A Month view can be rendered as a grid, agenda, or dense table; presentation does not define the temporal query.

Agenda and Table both use true group partitioning: grouping remains independent from the active stable sort rules rather than merely emitting repeated headings whenever the sort order changes group values.

### Querying and presentation

Current query surface:

- text search
- domain
- jurisdiction
- lifecycle status
- source visibility
- current-release Taria bundle membership
- current-release projected CalendarSet membership

Source visibility is independent from the event query.

Current presentation dimensions are also independent:

- grouping by date, week, month, source, domain, jurisdiction, institution, event type, or status
- stable multi-key sorting
- semantic fallback coloring by source, domain, jurisdiction, institution, event type, or status
- ordered query-driven color rules with first-match precedence
- Grid, Agenda, and Table layouts

These dimensions persist in saved views and do not reorganize or duplicate canonical events.

The advanced boolean/query-expression system is now implemented as a first working vertical slice.

Advanced queries support:

- arbitrarily nested AND / OR / NOT groups
- typed text predicates
- text set membership
- lifecycle-status sets
- integer comparisons for importance and personal relevance
- exists / missing predicates
- temporal-kind sets
- explicit civil-date overlap
- timezone-aware instant-to-date evaluation
- opt-in month/year imprecise-span matching
- relative civil-date windows anchored to the view timezone's current day
- Taria bundle-membership predicates by stable bundle ref
- Taria projected-calendar-membership predicates by stable calendar ID
- recursive GUI editing
- persistence through UI state and saved views
- backward-compatible loading of older saved query JSON

Simple facets remain convenient top-level filters and are ANDed with the advanced expression tree.

### Saved views

Named saved views are real durable product objects stored in SQLite.

A saved view currently retains:

- event query
- hidden/visible source selection
- date-range mode
- layout
- grouping
- stable multi-key sort rules
- semantic color fallback
- ordered color rules
- ordered composition layers
- overlays
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

- richer release-history/diff inspection
- whole-release atomic rollback across multiple source imports
- background/worker execution for large release adoption
- composition-layer GUI editor
- saved-view-reference composition / inheritance with cycle-safe semantics
- event-occurrence/recurrence engine
- dedicated provenance/snapshot/history tables
- snapshot diffs
- source health/rollover
- annotations
- relations and collections
- duplicate/entity resolution
- timeline/heatmap/pivot views
- user-defined table columns
- reminders
- ICS/webcal/CalDAV ingestion/export
- full personal event editing

### Ordered color rules

Color is now a rule engine rather than a calendar-container property.

Implemented:

- rules reuse the full recursive query language, including temporal predicates
- rules are ordered and deterministic
- first enabled matching rule wins
- explicit RGB colors
- enable/disable, edit, add/delete, and reorder controls
- existing semantic ColorBy strategy remains the fallback
- rules apply consistently across Grid, Agenda, Table, and unplaced records
- rules persist in saved views

### Calendar algebra foundation

The ordered composition foundation is implemented and verified.

A saved view can persist sequential logical composition layers with these operators:

- union
- intersection
- subtraction

Evaluation is deterministic and ordered:

```text
base query
    -> composition layer 1
    -> composition layer 2
    -> ...
    -> overlay union
```

Composition layers are currently wired through:

- runtime event visibility
- transient UI state
- SavedView capture/apply
- SQLite persistence/migration

The composition model is intentionally event-native and does not copy membership.

**Not implemented yet:** a GUI editor for composition layers. The model/runtime/persistence foundation exists before the interaction surface.

### Overlays

The first overlay slice is implemented.

An overlay is an independently defined query rendered into the same canonical event result without copying event membership.

Implemented:

- enabled overlays union their query matches with the base view query
- global source visibility remains independent
- overlay order is styling precedence
- overlays have independent semantic fallback coloring
- overlays can carry their own ordered color rules
- recursive overlay query editing
- overlay enable/disable, naming, reorder, and deletion
- overlays persist in saved views

## Immediate next implementation boundary

Harden **release adoption execution** now that filesystem transport, identity, membership, query exposure, and release-posture UX are materialized.

Implemented in the current slice:

1. the Sources/Taria panel shows the adopted release ID, channel, status, completeness, generated/adopted times, coverage counts, and per-canonical-bundle population posture;
2. partial, pending, and gap-only states are rendered from the persisted manifest rather than hard-coded release assumptions;
3. bundle-membership predicates offer selectable refs from the adopted release;
4. projected-calendar predicates offer selectable stable calendar IDs with human-readable names/bundle context;
5. raw ref text remains editable for portability/debugging;
6. query choices come from SQLite, so normal view editing does not require live Taria filesystem access.

The next slice should:

1. move large release adoption to a worker boundary so **Update Taria Sources** never stalls egui;
2. add whole-release transaction/rollback semantics across all payloads, CalendarSets, aliases, and release metadata;
3. add release-history/diff inspection;
4. then continue programmable-view expansion with the composition-layer GUI editor, user-defined Table columns, richer facets, and saved-view inheritance.

## Rule going forward

Do not regress to a container-centric calendar.

The same canonical event corpus must remain independently:

- filterable
- groupable
- sortable
- colorable
- renderable in multiple layouts
- reusable by unlimited saved views
