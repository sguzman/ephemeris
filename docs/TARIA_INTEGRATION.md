# Taria Integration

## Relationship

Taria and Ephemeris solve different problems.

- **Taria** owns knowledge acquisition, Resourcearium source curation, source evidence, temporal assertions, identity/revision reasoning, frozen normalized snapshots, reconciliation, projections, CalendarSets, rollover, and rendered/export artifacts.
- **Ephemeris** owns interactive local temporal exploration, visualization, querying, inspection, annotations, saved views, overlays, and ordinary personal calendar behavior.

Ephemeris consumes Taria's rich temporal state. It must not force Taria to imitate a desktop calendar database, and it must not flatten Taria into ICS before ingestion.

## Existing Taria contracts

The integration is no longer speculative. Resourcearium already defines the temporal contracts Ephemeris needs.

The important upstream contracts include:

- `temporal-event-assertion-schema.yml`
- `temporal-source-time-interpretation-schema.yml`
- `temporal-event-identity-revision-schema.yml`
- `temporal-normalized-event-snapshot-schema.yml`
- `temporal-calendar-projection-schema.yml`
- `temporal-calendar-set-schema.yml`
- `temporal-bundle-projection-ontology.yml`

Ephemeris should align with those contracts rather than invent a competing temporal ontology.

## Upstream lifecycle

Taria's accepted staged projection lifecycle is:

```text
CalendarProjectionSpec
    -> SourceResolutionSet
    -> AcquisitionSnapshotSet
    -> NormalizedEventSnapshot
    -> ProjectionEventSet
    -> ReconciledProjectionEventSet
    -> CalendarSet
    -> RenderedCalendarArtifact
```

Ephemeris is primarily a **consumer after normalization/reconciliation**, before lossy rendering.

The initial integration boundary should therefore be a frozen/reconciled JSON artifact, not an ICS rendering.

## Initial consumer boundary

The first implementation target is:

**ReconciledProjectionEventSet + its source/provenance context**

This is a better application boundary than raw TemporalEventAssertions because Taria has already done identity grouping and conflict handling.

A reconciled event can provide:

- `reconciled_event_key`
- `event_ref`
- assertion references
- source references
- source contexts/facets
- retained provenance references
- renderability state
- resolved display fields
- field-resolution decisions

Ephemeris must preserve these upstream references even when it maps them into local indexed fields.

## CalendarSet boundary

A Taria `CalendarSet` is useful as imported organizational/view metadata.

Its semantics explicitly match Ephemeris:

- calendar membership is not event identity
- the same event may belong to multiple calendars
- partitioning does not clone events
- hierarchy is logical, not an ICS-folder constraint
- merged and partitioned calendars may coexist
- unresolved/blocked/undated events remain accounted for

Therefore Ephemeris should import CalendarSet membership as **view/grouping metadata**, never by duplicating events.

## Bundle boundary

Taria's canonical temporal bundle ontology currently includes:

- Temporal Everything
- Politics & Government
- Economics & Public Statistics
- Finance & Markets
- Business & Corporate
- Sports & Competition
- Culture & Media
- Science, Technology & Space
- Public Health
- Environment & Weather
- Transportation & Civic Infrastructure
- Holidays & Observances
- Education & Academia
- Temporal Unclassified
- Temporal Projection Gaps

Domain membership is explicitly nonexclusive and orthogonal to geography.

Runtime views are consumer-owned. Taria explicitly allows consumers to:

- filter
- group
- sort
- color
- search
- overlay
- facet
- annotate
- save views

A runtime view does not rebuild the upstream source pipeline. This is exactly the Ephemeris programmable-view model.

## Identity

Taria explicitly separates:

- assertion identity
- event identity
- occurrence identity
- series identity
- event-version identity

Ephemeris must preserve that distinction.

A stable event ID does not imply immutable title, time, or status.

A changed time alone does not prove a new event.

A cancellation does not delete event identity.

Unresolved continuity must remain unresolved rather than being forced.

### Local identity

Ephemeris may use an internal UUID as a database primary key, but it must also preserve upstream Taria identity strings such as:

- `event_ref`
- `reconciled_event_key`
- assertion refs
- occurrence refs
- series refs
- version refs

The internal UUID must never replace or erase those identifiers.

## Temporal precision

Taria's source-facing temporal contract distinguishes:

### Value kinds

- instant
- local-datetime
- date-only
- all-day-date
- interval
- unknown

### Clock bases

- UTC
- explicit offset
- named timezone
- jurisdiction local
- venue local
- floating local
- timezone unknown
- not applicable

### Precision

- second
- minute
- hour
- date
- multi-day
- unknown

Critical upstream rules include:

- all-day date is not a midnight instant
- date-only is not automatically all-day
- timezone-unknown must not be guessed
- floating-local is not timezone-unknown
- normalization must preserve the original published value
- timezone inference must record its evidence
- ingestion-default timezone is provenance, not a source assertion

Ephemeris must retain those distinctions.

## Broader projection precision

Taria projection fixtures also contain intentionally imprecise future timing such as:

- year-only
- month-only

Ephemeris must not invent a day merely to place these events on a conventional grid.

The local model therefore needs explicit support for imprecise temporal values and undated/unrenderable temporal records.

## Renderability

A reconciled Taria event can be:

- ready
- blocked by temporal conflict
- blocked by operative-status conflict
- blocked by multiple conflicts
- undated

Blocked and undated events must remain locally inspectable/accounted for.

They must not be silently dropped and must not be coerced to arbitrary dates.

## Provenance

Taria provenance includes source/resource lineage, captures, transformations, assertion refs, snapshot refs, and evidence classes.

For a Taria event, Ephemeris should eventually be able to answer:

- Which canonical event is this?
- Which assertions support it?
- Which resources/surfaces support those assertions?
- Which provenance traces were retained?
- Which snapshot produced this state?
- Which reconciliation decisions resolved conflicting fields?
- Was the evidence direct, derived, historical, or otherwise classified?
- Was the source official/first-party/secondary/etc.?

## Import ownership

Taria-supplied event state is upstream-owned.

Ephemeris may layer user-owned data such as:

- watched state
- personal relevance
- notes
- reminder rules
- local tags
- suppression
- display classification
- saved-view membership/rules

A Taria refresh must not destroy local annotations.

## Snapshots

Taria's `NormalizedEventSnapshot` is a frozen state, not a fresh observation merely because it was materialized later.

Ephemeris must preserve the difference between:

- source observation time
- acquisition time
- snapshot creation time
- local import time

This enables reliable history and diffing.

## Refresh

The preferred first workflow is deterministic local import of a Taria-produced frozen/reconciled JSON artifact.

Later integrations may automate discovery of new Taria snapshots, but Ephemeris rendering and querying must never depend on live GitHub/Taria access.

## Rendered artifacts

ICS, JSCalendar, jCal, CSV, and remote calendar targets are downstream projections.

They are not the canonical Taria -> Ephemeris interchange layer.

## Rollover and source health

Taria already owns rich rollover/source-health semantics.

Ephemeris should consume and display that state rather than implementing a competing source-discovery system unless a future workflow explicitly requires it.
