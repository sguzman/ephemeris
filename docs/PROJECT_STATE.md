# Project State

Last verified implementation milestone: 2026-10-07.

## Current status

Ephemeris is a working native Rust + egui temporal-information application backed by a canonical SQLite store.

The project has crossed the architectural boundary that justified separating it from Rivetr: temporal events are first-class canonical data, not tasks with calendar fields attached. Presentation is programmable over that canonical corpus through queries, saved views, overlays, grouping, sorting, colors, and multiple calendar layouts.

The application currently has three mature foundations:

1. **Canonical temporal storage and views.** Events, sources, recurrence, source identity, Taria release metadata, CalendarSet memberships, saved views, and refresh history persist locally in SQLite.
2. **Taria / Resourcearium consumption.** Ephemeris can adopt local temporal releases, preserve upstream identity and provenance, validate content integrity, reconcile overlapping projections into one canonical event identity, and expose release/history state in the UI.
3. **RFC 5545 interoperability.** The supported iCalendar subset is bidirectional from VCALENDAR / VEVENT into canonical events and back out again, including recurrence masters, detached RECURRENCE-ID instances, RDATE, EXDATE, and moved/cancelled occurrence overrides.

## Verified quality gate

Verified implementation code checkpoint:

`b53bf2d2d830dc52c7894d8b21fcb63d3c94d935`

At that checkpoint:

- `cargo fmt --check` passes
- `cargo check` passes
- `cargo clippy --all-targets -- -D warnings` passes
- `cargo test` passes
- **594 library tests** pass
- **5 export-CLI tests** pass
- **1 import-CLI test** passes

## What works now

### Local application and store

Ephemeris runs as a native `eframe` / `egui` application with bundled SQLite storage.

The canonical store supports exact instants, floating date-times, explicit all-day values, date-only civil values, month/year precision, unresolved temporal values, persisted recurrence, source metadata, upstream identity, saved views, and release/history records.

### Recurrence

The canonical recurrence engine supports secondly, minutely, hourly, daily, weekly, monthly, and yearly cadence; interval; COUNT; civil-date UNTIL; the implemented BY-part matrix; RDATE; EXDATE; BYSETPOS; WKST; and single-occurrence moved/cancelled overrides.

Expansion preserves canonical original-slot identity and source-wall-clock behavior across DST. The recurrence editor exposes canonical presets, structured selectors, exception dates, and moved/cancelled override rows while retaining raw fallbacks.

### iCalendar

The `ical` module provides strict RFC 5545 transport for the supported subset.

Implemented boundaries include:

- RFC content-line parsing, escaping, unfolding, and folding
- VEVENT DTSTART / DTEND transport
- RRULE, RDATE, EXDATE, and RECURRENCE-ID codecs
- typed VEVENT binding
- master/detached instance assembly by UID
- canonical `TemporalEvent` projection
- strict VCALENDAR grouping and ingestion
- canonical VEVENT / VCALENDAR export
- deterministic native UID and DTSTAMP generation
- preservation of representable imported source properties
- moved and cancelled recurrence exception round-trips

Unsupported semantics are rejected instead of silently weakened. Current deliberate exclusions include `RANGE=THISANDFUTURE`, RDATE-only recurrence, VEVENT `DURATION` transport, occurrence-specific non-temporal detached overrides, `VTIMEZONE`, `VALARM`, DATE-TIME UNTIL, RDATE PERIOD, and leap-second `BYSECOND=60`.

### Local ICS sources

The `ics` source adapter imports local `.ics` / `.ical` calendars into the real canonical store.

Each local file becomes a first-class `SourceKind::Ics` source. The canonical file path supplies stable source identity and VEVENT UID supplies stable record identity.

Refresh behavior is transactional:

- unchanged UID records remain unchanged
- changed UID records update in place
- newly seen UIDs are created
- records absent from a later snapshot are retained rather than inferred deleted or cancelled
- invalid calendars fail before source creation

ICS import is available through both the `ephemeris-import` CLI and GUI drag/drop. A selected local ICS source can be refreshed explicitly from its original file in the source inspector, reusing the same transactional UID-stable import path. Stored ICS sources can also be exported atomically from the canonical store through the `ephemeris-export` CLI or the selected-source GUI inspector. Export preserves supported calendar-level source properties while serializing the current canonical event state.

### Remote ICS / Webcal sources

HTTP, HTTPS, `webcal://`, and `webcals://` feeds are supported as optional remote iCalendar sources. Remote acquisition runs off the egui thread against a separate SQLite connection, so a slow feed does not freeze the UI. `webcal` schemes normalize to HTTPS. Stable external source identity uses a SHA-256 digest of the normalized URL so private feed tokens are not exposed in source keys; the actual URL remains only as the refresh locator.

Remote subscriptions can be added from the CLI or GUI and refreshed from the source inspector. Their refresh attempts use the same durable refresh-attempt store as local ICS while remaining visually separated from Taria release history. Success, failure, incomplete/interrupted state, target, timestamps, and summaries survive application restart.

Remote refresh is conditional when the server supplies HTTP validators. Ephemeris persists ETag / Last-Modified metadata, sends If-None-Match / If-Modified-Since on later refreshes, and treats HTTP 304 as a successful not-modified refresh without re-downloading, reparsing, or rewriting canonical events. CLI and GUI surfaces report that state explicitly.

### CSV sources

The `csv` adapter implements a strict, versioned tabular subset rather than a generic flattening layer.

Local CSV files import through stable canonical file identity and required row-level `record_key` values. Re-import preserves source/event identity, updates changed rows, and retains rows missing from later snapshots. CLI import/export and GUI drag/drop/refresh/export are implemented.

Only semantics with an explicit round-trip representation are accepted. Unsupported richer semantics fail loudly.

### Taria

Filesystem-first Taria integration remains the richer upstream path.

Ephemeris can consume reconciled event sets, compact recovered indexes, TemporalBundleRelease metadata, CalendarSets, bundle membership, release coverage, immutable per-release event snapshots, and release diffs. Overlapping projections converge on canonical events while memberships remain separate.

The application can detect a local Resourcearium checkout and run **Update Taria Sources** in the background using a separate SQLite connection.

## Immediate frontier

The versioned native JSON interchange boundary is implemented and verified. Current **v9** snapshots preserve canonical sources/events, relations, collections/sequences and memberships, identity assessments, user annotations, structured provenance, structured event location, structured participants, the canonical entity registry, and Ephemeris-local participant→entity bindings. Versions 1–8 remain readable under their historical capability gates. Validation rejects duplicate/dangling identities, malformed topology, invalid recurrence/uncertainty/location/participants, invalid entity registries, and participant bindings whose event/entity/fingerprint no longer resolve. Import remains transactional and merge-only; collection memberships are authoritative only for collections explicitly present in the imported snapshot. CLI import/export and GUI drag/drop use the explicit `.ephemeris.json` suffix. Saved views, refresh history, and Taria release tables remain outside this snapshot boundary.

Strict CSV v1 projection/import is now implemented and verified. CSV files become first-class `SourceKind::Csv` sources with stable file/source identity and required per-row `record_key` identity. The schema supports non-recurring exact instants, floating date-times, explicit all-day values, date-only values, and scalar event metadata. Refresh updates matching rows in place and retains rows absent from later snapshots. Source-scoped export is available from the CLI and GUI, and empty exports retain a valid schema header.

CSV v1 deliberately rejects recurrence, temporal uncertainty, structured location, structured participants, month/year/unknown precision, list-valued tags/reference metadata, and arbitrary event properties rather than flattening them into lossy cells.

CSV refresh attempts now participate in the same durable external-source history as ICS/Webcal, including persisted success/failure state and immediate GUI visibility.

The core Phase 9 interoperability boundary is now strong enough to stop driving the roadmap: local/remote iCalendar, conditional Webcal refresh, canonical JSON snapshots, and strict CSV are all implemented. JSCalendar/jCal and CalDAV remain optional future adapters when concrete ecosystem value justifies them.

Event relations and collections/sequences now have a canonical external topology layer. Directed relations and ordered/unordered collections have domain types, schema-v15 persistence, validation, semantic uniqueness, FK cascades, atomic collection membership replacement, reverse lookups, and programmable-view predicates. Collection membership and incoming/outgoing relation-type context remain external to `TemporalEvent`, so topology does not contaminate event ownership or fields.

User-facing topology inspection and core editing are now implemented. The event inspector resolves recurrence occurrences back to their canonical event, shows collection/sequence membership and incoming/outgoing relation edges with counterpart titles, and allows local collection membership changes, collection/sequence creation, bounded relation-target search, typed incoming/outgoing relation creation, and relation deletion. These edits remain Ephemeris-local even when the underlying event source is read-only.

The event-topology milestone is now substantially complete. Ordered sequence reordering, broader collection management (rename/description, collection↔sequence conversion, deletion without deleting events), and topology-aware canonical JSON v2 are implemented and verified.

Temporal uncertainty now has a canonical first slice. `TemporalEvent.time_uncertainty` is an optional typed bounded **start-placement** window, independent from coarse precision, lifecycle status, and general event `confidence`. Date/all-day, floating, and exact-instant events use date, wall-clock, and UTC uncertainty coordinates respectively; the representative canonical start must lie inside a non-zero window. Schema v16 persists the field losslessly, canonical JSON preserves it, the inspector displays it separately from the representative placement, and programmable views can query both uncertainty presence and overlap against the possible-start window. Recurring events and month/year/unresolved precision deliberately reject this uncertainty form for now, and CSV v1 / iCalendar export reject uncertain events rather than erasing the semantics.

Duplicate/entity assessments, user-owned annotations, and structured provenance are implemented canonical subsystems. Identity assessments remain distinct from ordinary semantic relations; annotations survive source refresh; structured provenance carries typed assertion/source/provenance roles, optional linked sources, notes, and extensible properties. All three are persisted, queryable, inspector-visible, and preserved by canonical JSON.

General-purpose canonical event history is now implemented as immutable schema-v20 event revisions. A first revision is captured on creation; later revisions are appended only for real canonical changes, not timestamp-only touches. Live-event writes and revision appends are atomic, revisions survive deletion of the current event, import refreshes inherit the same history boundary, and the inspector exposes a read-only canonical history panel. Existing pre-v20 events are intentionally not backfilled.

Structured event location and participants are now implemented canonical fields. Location is persisted in schema v21 with structured venue/address/locality/region/country/coordinates/virtual-URL semantics. Participants are persisted in schema v22 as ordered structured rows with required name plus optional role, participant type, stable entity reference, and extensible properties. Both are queryable and inspector-visible; canonical JSON v7 preserves both losslessly. Editable/local events can add or remove participants through the inspector, while read-only source-backed events remain source-owned. CSV v1 and VEVENT export reject participant-bearing events rather than silently dropping them.

Canonical entity resolution is now a verified Phase-2 subsystem. Schema v23 introduced the durable entity registry; schema v24 adds Ephemeris-local participant→entity bindings without rewriting source-owned participant labels or `entity_ref`. Resolution order is explicit: manual local binding, source stable reference, then unique exact canonical-name/alias match with participant-type narrowing. Ambiguous and unresolved references remain unresolved rather than guessed.

The UI now exposes participant resolution on read-only as well as editable events, a searchable/editable canonical entity registry, guarded entity merge, reverse event-usage visibility, and saved-view predicates by stable canonical entity UUID or entity type. Merges transfer aliases, external refs, old local refs, and local bindings while rejecting incompatible entity types or conflicting opaque properties. Canonical JSON v9 preserves the local binding layer losslessly.

Phase 10 personal scheduling is now materially underway. Canonical events have explicit Busy/Free availability behavior persisted in schema v27 and mapped to iCalendar TRANSP; the free/busy engine respects that behavior across recurrence materialization, reports skipped unsupported temporal shapes, finds candidate slots inside persisted personal work-hour/workday preferences, and can prefill local event creation directly from a free interval or suggested slot.

Reminder workflows are already end-to-end in-app: event- and saved-view-targeted before-start rules persist in SQLite, recurrence-aware evaluation materializes due occurrences, deliveries deduplicate durably, active reminders survive restart, and users can dismiss them from the reminder center.

Local authoring is also broader now. Quick-create captures description, event type, domain, lifecycle status, and Busy/Free behavior in addition to date/time. Writable non-recurring exact, floating, all-day, and date-only events have a canonical time editor that preserves their temporal kind and source clock context; recurring masters and uncertain placements are deliberately blocked rather than mutated unsafely. Writable events can also add/edit/remove the full structured location object, including venue/address/locality/region/postal/country, paired coordinates, and virtual URL, with canonical validation.

The next Phase 10 frontier is **deeper personal scheduling workflow** on top of these verified primitives: reminder follow-up behavior such as durable snooze, attendee/invitation semantics where they provide real value, and richer scheduling assistance without weakening local-first canonical semantics.

## Documentation roles

Current state belongs here.

Detailed historical implementation checkpoints and test-count progression are archived in [IMPLEMENTATION_HISTORY.md](IMPLEMENTATION_HISTORY.md).

Product intent belongs in [PRODUCT.md](PRODUCT.md), architecture in [ARCHITECTURE.md](ARCHITECTURE.md), temporal semantics in [TIME_SEMANTICS.md](TIME_SEMANTICS.md), and staged future work in [ROADMAP.md](ROADMAP.md).
