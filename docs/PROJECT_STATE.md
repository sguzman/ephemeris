# Project State

Last verified implementation milestone: 2026-10-06.

## Current status

Ephemeris is a working native Rust + egui temporal-information application backed by a canonical SQLite store.

The project has crossed the architectural boundary that justified separating it from Rivetr: temporal events are first-class canonical data, not tasks with calendar fields attached. Presentation is programmable over that canonical corpus through queries, saved views, overlays, grouping, sorting, colors, and multiple calendar layouts.

The application currently has three mature foundations:

1. **Canonical temporal storage and views.** Events, sources, recurrence, source identity, Taria release metadata, CalendarSet memberships, saved views, and refresh history persist locally in SQLite.
2. **Taria / Resourcearium consumption.** Ephemeris can adopt local temporal releases, preserve upstream identity and provenance, validate content integrity, reconcile overlapping projections into one canonical event identity, and expose release/history state in the UI.
3. **RFC 5545 interoperability.** The supported iCalendar subset is bidirectional from VCALENDAR / VEVENT into canonical events and back out again, including recurrence masters, detached RECURRENCE-ID instances, RDATE, EXDATE, and moved/cancelled occurrence overrides.

## Verified quality gate

Verified implementation code checkpoint:

`44e42f8441a7219ddb0436171572f4023ebc1fc8`

At that checkpoint:

- `cargo fmt --check` passes
- `cargo check` passes
- `cargo clippy --all-targets -- -D warnings` passes
- `cargo test` passes
- **489 library tests** pass
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

The versioned native JSON interchange boundary is implemented and verified. Current **v2** snapshots preserve canonical `TemporalSource`, `TemporalEvent`, `EventRelation`, `EventCollection`, and ordered/unordered collection-membership records with stable UUIDs, timestamps, recurrence, time semantics, and arbitrary properties. Version 1 sources/events-only snapshots remain readable. Validation rejects duplicate identities, dangling topology references, duplicate semantic relations, malformed sequence positions, unsupported versions, and invalid recurrence. Import is transactional and merge-only at the corpus level; membership lists are authoritative only for collections explicitly present in the imported v2 snapshot. CLI import/export and GUI drag/drop use the explicit `.ephemeris.json` suffix. Saved views, refresh history, Taria release tables, and auxiliary alias tables remain outside this snapshot boundary.

Strict CSV v1 projection/import is now implemented and verified. CSV files become first-class `SourceKind::Csv` sources with stable file/source identity and required per-row `record_key` identity. The schema supports non-recurring exact instants, floating date-times, explicit all-day values, date-only values, and scalar event metadata. Refresh updates matching rows in place and retains rows absent from later snapshots. Source-scoped export is available from the CLI and GUI, and empty exports retain a valid schema header.

CSV v1 deliberately rejects recurrence, month/year/unknown precision, list-valued tags/reference metadata, and arbitrary event properties rather than flattening them into lossy cells.

CSV refresh attempts now participate in the same durable external-source history as ICS/Webcal, including persisted success/failure state and immediate GUI visibility.

The core Phase 9 interoperability boundary is now strong enough to stop driving the roadmap: local/remote iCalendar, conditional Webcal refresh, canonical JSON snapshots, and strict CSV are all implemented. JSCalendar/jCal and CalDAV remain optional future adapters when concrete ecosystem value justifies them.

Event relations and collections/sequences now have a canonical external topology layer. Directed relations and ordered/unordered collections have domain types, schema-v15 persistence, validation, semantic uniqueness, FK cascades, atomic collection membership replacement, reverse lookups, and programmable-view predicates. Collection membership and incoming/outgoing relation-type context remain external to `TemporalEvent`, so topology does not contaminate event ownership or fields.

User-facing topology inspection and core editing are now implemented. The event inspector resolves recurrence occurrences back to their canonical event, shows collection/sequence membership and incoming/outgoing relation edges with counterpart titles, and allows local collection membership changes, collection/sequence creation, bounded relation-target search, typed incoming/outgoing relation creation, and relation deletion. These edits remain Ephemeris-local even when the underlying event source is read-only.

The event-topology milestone is now substantially complete. Ordered sequence reordering, broader collection management (rename/description, collection↔sequence conversion, deletion without deleting events), and topology-aware canonical JSON v2 are implemented and verified.

The next canonical-model frontier is **temporal uncertainty**: represent approximate/estimated temporal knowledge without confusing it with coarse precision (`Month`/`Year`), lifecycle state (`tentative`/`estimated`/`projected`), or general event confidence. After that come duplicate/entity resolution, richer provenance records, and annotations.

## Documentation roles

Current state belongs here.

Detailed historical implementation checkpoints and test-count progression are archived in [IMPLEMENTATION_HISTORY.md](IMPLEMENTATION_HISTORY.md).

Product intent belongs in [PRODUCT.md](PRODUCT.md), architecture in [ARCHITECTURE.md](ARCHITECTURE.md), temporal semantics in [TIME_SEMANTICS.md](TIME_SEMANTICS.md), and staged future work in [ROADMAP.md](ROADMAP.md).
