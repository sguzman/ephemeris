# Time Semantics

## Goal

Calendar correctness depends on modeling time explicitly rather than treating every temporal value as a UTC timestamp.

## Required temporal forms

Ephemeris should distinguish at least:

### Instant

An exact point on the global timeline.

Example: a webcast starts at a known UTC instant.

### Zoned local date-time

A wall-clock time interpreted in a named timezone.

Example: 09:00 America/New_York.

Both the named timezone and resulting instant matter.

### Floating local date-time

A wall-clock time intentionally not tied to a timezone.

This is valid in iCalendar semantics and must not be silently converted as if it were UTC.

### All-day date

A civil date without a time-of-day.

An all-day event must remain on its intended date when rendered in another timezone.

### Interval

A start plus optional end/duration.

### Date range

A range of civil dates, potentially inclusive/exclusive depending on source semantics.

### Approximate or uncertain time

Future Taria data may include temporal uncertainty. The data model should leave room for ranges, estimated instants, or confidence metadata instead of inventing false precision.

## Source timezone vs display timezone

Preserve:

- source timezone
- source-local representation
- canonical instant when one exists

Presentation may default to a user timezone such as `America/Mexico_City` without rewriting source semantics.

A view may temporarily render in another timezone.

## DST

Timezone conversion must use real timezone rules.

Tests should cover:

- spring-forward nonexistent times
- fall-back ambiguous times
- timezone database changes where relevant
- events crossing DST boundaries

## Recurrence

The recurrence engine now includes the first exception-aware slice.

Current canonical recurrence definitions support:

- daily, weekly, monthly, and yearly frequency;
- positive interval;
- optional count;
- optional inclusive civil-date `until`;
- RDATE additions;
- EXDATE exclusions;
- per-occurrence moved and cancelled overrides;
- date-only, all-day, floating, and exact/source-timezone base events.

Month/year/unresolved precision cannot be a recurrence base and is rejected before persistence. Exception timestamps must use the same temporal kind as the series, duplicate overrides are rejected, and one original occurrence slot cannot simultaneously be EXDATE-excluded and overridden.

Exact recurrence with a retained source timezone advances in source-local wall-clock time, then resolves each occurrence back to UTC. This preserves a series such as 09:00 America/New_York across DST rather than preserving a fixed UTC hour. Nonexistent local times are skipped; ambiguous local times resolve deterministically to the earlier instant.

Monthly/yearly recurrence preserves the original calendar day. An invalid target date is skipped rather than coerced to month-end. Count applies to valid RRULE-generated occurrences; RDATE additions do not consume the RRULE count, and EXDATE is applied after candidate generation.

Conceptually, the visible occurrence set is built from RRULE candidates plus RDATE additions, with EXDATE removing matching original slots and occurrence overrides transforming matching slots. A moved override changes the rendered time without changing the identity of the original slot. A cancelled override remains materialized with cancelled lifecycle status; cancellation is therefore not equivalent to EXDATE disappearance.

Still required for fuller interoperable recurrence semantics:

- broader RRULE dimensions beyond the current frequency/interval/count/until subset;
- richer recurrence/exception authoring and editing;
- source-adapter mapping for external recurrence-exception representations.

The system distinguishes the persisted recurrence definition from materialized view occurrences.

## Expansion strategy

Infinite recurrence is not eagerly materialized.

Ephemeris retrieves recurring canonical events independently of the base date-window query, then expands each series only inside the active Year/Quarter/Month/Week/Day horizon. Materialized occurrences carry deterministic occurrence UUIDs, canonical event lineage, recurrence origin/index, original occurrence time, effective time/status, and override posture.

Occurrence identity is derived from canonical event identity plus the original recurrence slot: civil start date for date-only/all-day events, UTC start instant for exact events, and local start date-time for floating events. Replacement time is deliberately excluded from identity, so moving an occurrence preserves its identity and lineage.

Moved overrides are also considered when their original slot lies outside the currently scanned horizon. This allows an occurrence moved into the active window to appear even though its original date would not otherwise have been materialized.

Stable original-slot identity is intended to support:

- annotations;
- richer exceptions;
- moved instances;
- source refresh;
- diffing.

## Rescheduling

A rescheduled event should preferably preserve lineage.

Depending on source evidence, this may be represented as:

- same event with changed schedule plus history
- occurrence override
- explicit `rescheduled_from` relation

Do not silently turn every move into an unrelated new event.

## Cancellation

Cancellation is a lifecycle state, not necessarily deletion.

For recurrence, an EXDATE means the original slot is excluded from the materialized set. A cancelled occurrence override instead preserves the occurrence and marks its effective lifecycle status as cancelled. Cancelled events and occurrences may still be valuable historical facts and remain queryable.

## Due dates and deadlines

A deadline is temporal but not always an appointment.

Ephemeris may project task/deadline objects into views while retaining their semantic type.

A deadline should not be forced to pretend it has a duration or attendance semantics.

## Day boundaries

Day grouping depends on the view timezone.

All-day events group by civil date semantics, not by arbitrary UTC midnight conversion.

## Week semantics

Week start is a view/user preference, not canonical event data.

Derived fields may include:

- ISO week
- locale/user week
- quarter
- weekend
- electoral cycle
- relative date buckets

## Tests required

Time tests should cover:

- all-day events across timezone changes
- DST transitions
- floating times
- UTC instants
- events crossing midnight
- recurrence exceptions
- leap years
- month/year boundaries
- source timezone preservation
- date-only deadlines
