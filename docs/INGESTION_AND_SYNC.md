# Ingestion, Refresh, Synchronization, and Export

## Principle

Import should be promiscuous; canonical storage should be normalized.

Source formats are adapters, not the ontology.

## Input classes

Long-term inputs may include:

- Taria native temporal bundles
- ICS
- webcal/webcals
- CalDAV
- JSCalendar
- jCal
- JSON
- CSV
- APIs
- generated projections
- manual local events

## Ingestion pipeline

```text
acquire
  -> parse
  -> validate
  -> normalize
  -> identify/match
  -> reconcile
  -> transact
  -> index
  -> record provenance/snapshot
```

Each stage should provide diagnostics.

## Source definitions

A source definition should describe:

- identity
- locator(s)
- source kind
- format
- publisher/authority
- refresh policy
- authentication reference if required
- read-only/write ownership
- expected freshness
- rollover behavior
- adapter configuration

## Refresh semantics

A refresh should be repeatable and idempotent where possible.

It should report:

- records observed
- events created
- events updated
- events unchanged
- events moved/rescheduled
- events cancelled
- events removed/superseded
- conflicts
- parse failures
- identity ambiguities

## Deletion semantics

A source record disappearing does not always mean the real-world event should be erased.

Policies may include:

- source record removed -> canonical event remains with provenance history
- source record removed -> mark source assertion inactive
- source explicitly cancelled -> event becomes cancelled
- generated projection rebuild -> remove obsolete projection records while preserving history

Adapters/source classes may require different policy.

## Duplicate resolution

Matching should use the strongest available identifiers first.

Potential signals:

- source-native UID
- canonical publisher ID
- stable Taria ID
- recurrence ID
- canonical URL
- normalized time
- institution
- title similarity
- location
- relation context

Weak heuristics should produce candidates/diagnostics, not irreversible silent merges.

## Raw source retention

Depending on format and size, retain:

- raw payload
- content hash
- source-record excerpt
- raw field map
- or a stable reference to an external snapshot

This supports debugging and future re-normalization.

## Network refresh

Network refresh should be subordinate to local operation.

Requirements:

- background execution
- timeout
- cancellation
- retry policy
- visible status
- ETag/Last-Modified where useful
- CalDAV sync tokens where applicable
- no frame-loop blocking

## Source health

The UI should eventually show:

- last successful update
- last attempt
- failure state
- last source change
- expected next check
- stale state
- rollover state
- HTTP/cache metadata where useful

## External synchronization

External calendar services are optional integration surfaces.

If CalDAV/Google-style synchronization is added, define:

- source authority
- local authority
- read-only/read-write mode
- conflict policy
- UID mapping
- sync token/ETag behavior
- deletion policy
- attendee/invitation ownership

The external service must not become the canonical representation of Taria's richer event model.

## Export

Export is a projection.

Examples:

- export a saved view to ICS
- export query results to CSV
- export selected events to JSON
- generate a mobile-compatible calendar feed

Export does not require moving events into a dedicated storage container.

## Personal events

Locally authored personal events may have Ephemeris as their authoritative source.

That ownership should be explicit and separate from read-only external or Taria-owned events.
