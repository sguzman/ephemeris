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

Full recurrence semantics are a long-term requirement:

- RRULE
- RDATE
- EXDATE
- recurrence exceptions
- moved occurrences
- cancelled occurrences
- DTSTART semantics
- all-day recurrence
- floating recurrence
- timezone-bound recurrence

The system must distinguish recurrence definitions from materialized occurrences.

## Expansion strategy

Do not eagerly materialize infinite recurrence.

A recurrence engine should expand within query/view horizons and cache/materialize only when justified.

Identity for each occurrence must remain stable enough for:

- annotations
- exceptions
- moved instances
- source refresh
- diffing

## Rescheduling

A rescheduled event should preferably preserve lineage.

Depending on source evidence, this may be represented as:

- same event with changed schedule plus history
- occurrence override
- explicit `rescheduled_from` relation

Do not silently turn every move into an unrelated new event.

## Cancellation

Cancellation is a lifecycle state, not necessarily deletion.

Cancelled events may still be valuable historical facts and should remain queryable.

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
