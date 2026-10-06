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

`a84146c418b60540879f041074397a0833b61c84`

At that checkpoint:

- `cargo fmt --check` passes
- `cargo check` passes
- `cargo clippy --all-targets -- -D warnings` passes
- `cargo test` passes
- **453 library tests** pass

Later documentation-only commits do not change this code checkpoint.

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

ICS import is available through both the `ephemeris-import` CLI and GUI drag/drop.

### Taria

Filesystem-first Taria integration remains the richer upstream path.

Ephemeris can consume reconciled event sets, compact recovered indexes, TemporalBundleRelease metadata, CalendarSets, bundle membership, release coverage, immutable per-release event snapshots, and release diffs. Overlapping projections converge on canonical events while memberships remain separate.

The application can detect a local Resourcearium checkout and run **Update Taria Sources** in the background using a separate SQLite connection.

## Immediate frontier

The next interoperability slice is **stored-source ICS export**: select canonical events from the store and write a strict VCALENDAR payload to disk through the already-verified exporter.

After that, the Phase 9 priorities are:

- a GUI export surface
- refresh/export ergonomics for ICS sources
- remote ICS / webcal ingestion
- broader external formats where useful
- CalDAV only if its synchronization value justifies the added state machine

The long-horizon domain roadmap then returns to event relations, collections/sequences, uncertainty, and duplicate/entity resolution.

## Documentation roles

Current state belongs here.

Detailed historical implementation checkpoints and test-count progression are archived in [IMPLEMENTATION_HISTORY.md](IMPLEMENTATION_HISTORY.md).

Product intent belongs in [PRODUCT.md](PRODUCT.md), architecture in [ARCHITECTURE.md](ARCHITECTURE.md), temporal semantics in [TIME_SEMANTICS.md](TIME_SEMANTICS.md), and staged future work in [ROADMAP.md](ROADMAP.md).
