# Temporal Data Model

## Principle

The canonical model must describe temporal information directly.

Ephemeris must not use a task object as the permanent event ontology merely because Rivetr historically projected calendar entries through tasks.

## Core entities

### TemporalEvent

A `TemporalEvent` is the durable identity for a conceptual event.

Representative fields:

```text
id
canonical_key?
normalized_title
raw_title?
description?
event_type?
domain?
status
importance?
personal_relevance?
confidence?
source_timezone?
time_spec
location?
jurisdiction?
institution?
participants[]
tags[]
properties{}
created_at
updated_at
```

Not every field is mandatory. Stable semantics should become typed fields; evolving or source-specific semantics can live in extensible properties.

### EventOccurrence

Recurring events need a distinction between the recurring conceptual event and a particular occurrence.

The implemented materialized occurrence foundation now carries:

```text
id
event_id
recurrence_index?
origin
original_time
time
status
override_applied
cancelled_by_override
```

The persisted recurrence definition also carries weekly `by_weekday` selection with an independent recurrence `week_start`/WKST, signed monthly `by_month_day` selection (`-31..=-1` or `1..=31`), monthly `by_month_weekday` ordinal weekday selection (`±1..±5`) with intersection against `by_month_day` when both are present, positive yearly `by_month` selection, RDATE additions, EXDATE exclusions, and occurrence overrides keyed by original occurrence time. A moved occurrence retains identity/lineage to the original recurrence slot; its replacement time is presentation/effective state, not a new identity. A cancelled override remains an occurrence with cancelled status, while EXDATE removes the slot from materialization.

### TemporalSource

Represents a publisher/feed/resource rather than an event.

Representative fields:

```text
id
name
publisher?
authority_kind
source_kind
locator(s)
format
geographic_scope?
domain_scope?
refresh_policy?
freshness_expectation?
enabled
read_only
created_at
updated_at
```

Possible `source_kind` values include:

- native ICS/webcal
- API
- structured JSON
- Taria projection
- webpage extraction
- document extraction
- manual
- CalDAV
- generated/derived

### SourceRecord

When useful, Ephemeris should retain the source-level record that produced a normalized event.

Representative fields:

```text
id
source_id
source_record_key
snapshot_id?
raw_payload_ref?
raw_title?
raw_time?
content_hash?
acquired_at
```

This enables provenance inspection and re-normalization.

### ProvenanceRecord

Records transformations and assertions that connect normalized data to source evidence.

Possible information:

- source
- acquisition time
- source record
- parser/adapter version
- normalization version
- transformation notes
- confidence
- authoritative/secondary classification
- parent provenance record

### Snapshot

Represents a frozen observation of a source or projection at a point in time.

Representative fields:

```text
id
source_id
observed_at
content_hash?
adapter_version
status
record_count
metadata{}
```

Snapshots enable diffs and "as-of" views.

### EventRelation

Events may relate without being recurring occurrences.

Representative relation types:

- announced_by
- precedes
- follows
- rescheduled_from
- supersedes
- part_of
- runoff_of
- primary_for
- hearing_for
- markup_for
- vote_on
- release_for
- final_of

Relations should be extensible.

### EventCollection

Represents a meaningful set or sequence that is not merely a saved query.

Examples:

- 2026 Texas election cycle
- a legislative bill's hearing/markup/vote sequence
- a sports season/playoff/final structure

Collections may contain ordered or unordered members.

### Annotation

User-owned metadata layered over source-owned events.

Examples:

- note
- watched
- personal relevance
- reminder
- custom tag
- suppression
- rating
- local classification

Source refresh must not destroy annotations.

### SavedView

Saved query plus presentation configuration.

See `QUERY_AND_VIEWS.md`.

## Identity

Identity is one of the hardest problems in the system.

Ephemeris must distinguish:

1. source identity
2. source-record identity
3. canonical event identity
4. recurring-series identity
5. occurrence identity
6. snapshot observation identity

These must not be collapsed.

## Duplicate handling

Two sources may describe the same real-world event.

The system should support:

- exact identity matches
- likely duplicate candidates
- confirmed merges/linkage
- intentionally separate co-representations
- source precedence
- aliases

Do not deduplicate purely by title/date.

Blank or missing time fields must never accidentally collapse unrelated all-day records into one identity.

## Time representation

The model should distinguish:

- instant
- zoned local date-time
- floating local date-time
- all-day date
- date range
- time interval
- uncertain/approximate time where supported

See `TIME_SEMANTICS.md`.

## Event lifecycle

Lifecycle state must be independent from source-record existence.

Possible states:

- announced
- tentative
- scheduled
- confirmed
- rescheduled
- postponed
- cancelled
- completed
- observed
- superseded
- estimated
- projected
- disputed

A source deletion does not automatically mean "the real-world event never existed."

Reconciliation policy must consider source semantics.

## Ontology and tags

Ephemeris supports both:

### Typed fields

Use when semantics are stable and important for querying/interoperability.

Examples:

- jurisdiction
- event type
- status
- source
- timezone
- institution

### Tags/extensible properties

Use for:

- ad hoc classification
- source-specific attributes
- emerging ontology
- experimental fields

Do not force every concept into a schema migration; do not reduce all semantics to free-form tags.

## Importance and relevance

Keep at least the conceptual distinction between:

- objective/public importance
- personal relevance

They may drive different presentation and notification behavior.

## Locations

Location should support structure beyond a single string:

- venue name
- physical address
- locality
- region
- country
- jurisdiction
- coordinates where available
- virtual URL
- conferencing information

## Custom fields

Custom properties require:

- stable namespacing
- typed values where possible
- preservation during import/export
- query access
- schema/version metadata

## Immutability and mutation

Source-backed event fields should distinguish:

- source-owned values
- normalized canonical values
- user annotations/overrides

User changes must not be silently overwritten by refresh.

Where editing source-backed data is allowed, ownership/conflict rules must be explicit.
