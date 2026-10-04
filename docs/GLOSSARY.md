# Glossary

## Canonical event

The durable Ephemeris representation of a conceptual temporal event. It is not synonymous with one source record or one calendar container.

## Occurrence

A particular instance of a recurring event or other series where occurrence identity matters.

## Source

A publisher/feed/resource from which temporal assertions are acquired.

## Source record

The source-level item that produced or supports a normalized event.

## Provenance

Evidence and transformation metadata connecting canonical data to its origins.

## Snapshot

A frozen observation of a source/projection at a point in time.

## Saved view

A durable query plus presentation configuration over the canonical corpus.

## Overlay

An independently defined view rendered together with another view.

## Collection

A meaningful group or sequence of events that is not merely a saved query and not necessarily recurrence.

## Relation

A typed connection between events, such as rescheduled-from, runoff-of, or part-of.

## Annotation

User-owned metadata layered onto an event/source without mutating source-owned assertions.

## Projection

An output representation derived from canonical data or a query, such as an ICS feed.

## Materialized snapshot

A frozen result set. Unlike a saved view, it records membership at a point in time rather than re-running a query.

## Taria temporal resource

A temporal dataset/resource produced or curated by Taria, potentially containing richer ontology and provenance than standard calendar formats.

## Calendar algebra

Union, intersection, subtraction, and composition of saved queries/views.

## All-day event

An event represented by civil date semantics rather than an instant at midnight.

## Floating time

A local wall-clock time intentionally not bound to a timezone.

## Source timezone

The timezone used by the authoritative/source representation.

## Display timezone

The timezone selected for rendering a view. Changing it must not mutate the source event.

## Lifecycle state

The state of an event in the world: tentative, scheduled, postponed, cancelled, observed, and so on.

## Source health

Operational metadata about retrieval success, staleness, rollover, and expected refresh.

## Rollover

The transition from an edition/year-specific source to its successor.

## Dense calendar

A calendar workload with enough events that sparse appointment-style rendering is insufficient.
