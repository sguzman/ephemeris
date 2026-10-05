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
- daily BYDAY weekday filtering over active interval dates;
- weekly multi-day BYDAY weekday selection with explicit WKST recurrence-week anchoring;
- monthly plain BYDAY weekday expansion for all matching weekdays in an active month;
- yearly plain BYDAY weekday expansion across the active recurrence year, optionally restricted by BYMONTH or intersected by BYYEARDAY;
- signed yearly BYWEEKNO selection for `-53..=-1` and `1..=53`, using WKST-aware week-number years and optional plain BYDAY weekdays;
- signed monthly BYMONTHDAY selection for `-31..=-1` and `1..=31`;
- monthly ordinal BYDAY selection using ordinal weekdays `-5..=-1` or `1..=5`;
- yearly ordinal BYDAY selection using `-53..=-1` or `1..=53` when BYMONTH is absent, while BYMONTH retains month-scoped `±1..±5` semantics;
- positive yearly BYMONTH selection for months 1-12;
- yearly BYMONTH + signed BYMONTHDAY composition;
- yearly BYMONTH + ordinal BYDAY composition, including BYMONTHDAY intersection when both selector families are present;
- signed yearly BYYEARDAY selection for `-366..=-1` and `1..=366`;
- signed BYSETPOS selection for `-366..=-1` and `1..=366`, requiring at least one other supported BY selector;
- RDATE additions;
- EXDATE exclusions;
- per-occurrence moved and cancelled overrides;
- date-only, all-day, floating, and exact/source-timezone base events.

Month/year/unresolved precision cannot be a recurrence base and is rejected before persistence. Plain BYDAY is supported for daily, weekly, monthly, and yearly recurrence; duplicate weekdays are rejected. In yearly recurrence without BYWEEKNO it expands selected weekdays across the recurrence year, with BYMONTH restricting the month set and BYYEARDAY filtering the resulting civil dates. Custom WKST is accepted for weekly BYDAY and yearly BYWEEKNO contexts. BYWEEKNO is yearly-only, accepts signed values `-53..=-1` and `1..=53`, rejects zero/duplicate/out-of-range values, and cannot be combined with ordinal BYDAY. BYMONTHDAY accepts signed values `-31..=-1` and `1..=31`, rejects zero/duplicate/out-of-range values, and keeps previously persisted positive selector values backward-compatible. It is supported for daily, monthly, and yearly recurrence and remains invalid for WEEKLY. In DAILY it limits the active interval date by that date's own civil month; in MONTHLY it resolves candidate month-days inside the active month; in YEARLY it expands across every month when BYMONTH is absent and across only the selected months when BYMONTH is present. Ordinal BYDAY uses the persisted ordinal-weekday selector with context-sensitive bounds: monthly rules and yearly rules with BYMONTH use `-5..=-1` or `1..=5`, while yearly rules without BYMONTH use `-53..=-1` or `1..=53` against the whole recurrence year. Zero/out-of-range ordinals and duplicate ordinal+weekday selectors are rejected. Numeric BYDAY remains invalid with YEARLY+BYWEEKNO. Where BYMONTHDAY and ordinal BYDAY coexist, their resolved civil-date sets are intersected. BYMONTH is supported for daily, weekly, monthly, and yearly recurrence, accepts positive month numbers 1-12, and rejects duplicate or out-of-range values. For DAILY it filters civil dates reached by FREQ/INTERVAL; for WEEKLY it filters civil dates generated inside each active recurrence week; for MONTHLY it limits active recurrence months after the monthly interval cadence is chosen; for YEARLY it expands the active month set. BYYEARDAY is supported only for yearly recurrence, accepts signed values `-366..=-1` and `1..=366`, and rejects zero/duplicate/out-of-range values. BYSETPOS accepts signed positions `-366..=-1` and `1..=366`, rejects zero/duplicate/out-of-range values, and requires at least one other supported BY selector. Exception timestamps must use the same temporal kind as the series, duplicate overrides are rejected, and one original occurrence slot cannot simultaneously be EXDATE-excluded and overridden. An override's original slot must also resolve to a real RRULE-generated or RDATE-added occurrence; detached overrides cannot manufacture phantom occurrences.

Exact recurrence with a retained source timezone advances in source-local wall-clock time, then resolves each occurrence back to UTC. This preserves a series such as 09:00 America/New_York across DST rather than preserving a fixed UTC hour. Nonexistent local times are skipped; ambiguous local times resolve deterministically to the earlier instant.

Daily recurrence first advances by its FREQ/INTERVAL cadence and then applies supported limiting selectors. Plain BYDAY keeps only selected weekdays; BYMONTH keeps only dates in selected months; signed BYMONTHDAY keeps only dates matching the selected positive or month-end-relative civil day; when multiple daily selectors are present they intersect. Skipped dates do not consume COUNT, and WKST is not semantically active for DAILY. BYDAY-only reachability uses the weekday cycle; DAILY BYMONTH or BYMONTHDAY uses the finite 400-year Gregorian date cycle (146,097 days), so selector/interval combinations that can never generate an RRULE slot terminate instead of causing unbounded expansion or override-target validation loops. Exact/source-timezone series retain their source wall clock across DST, and the resulting original slots use the same EXDATE/override identity path as other selectors.

Weekly recurrence uses the persisted WKST value as its recurrence-week anchor when BYDAY is present; WKST defaults to Monday for backward-compatible/default rules. Selected weekdays are generated in WKST-relative chronological order regardless of storage/input order. In the first active recurrence week, selected weekdays earlier than DTSTART are omitted; DTSTART itself is an RRULE occurrence only when its weekday is selected. `interval = N` therefore means every Nth WKST-anchored recurrence week. BYMONTH, when present, filters generated weekly civil dates before BYSETPOS, including weeks that straddle month boundaries; without BYDAY, DTSTART's weekday is preserved and then filtered by month. A finite 400-year / 20,871-week reachability pass accounts for BYMONTH, BYDAY, and BYSETPOS candidate cardinality so permanently empty weekly rules terminate cleanly. This recurrence anchor is separate from the user's view/display week-start preference.

Yearly plain BYDAY without BYWEEKNO expands every occurrence of each selected weekday inside the active recurrence year. Numeric/ordinal BYDAY shares the same logical BYDAY union: when BYMONTH is absent, an ordinal such as `1MO` or `-1FR` means the first Monday or last Friday of the whole year; when BYMONTH is present, the ordinal is resolved separately inside each selected month. If BYYEARDAY is present, its resolved dates are filtered by that same plain+ordinal BYDAY union. Candidate dates are sorted and deduplicated, first-year candidates before DTSTART are omitted, and BYSETPOS operates on the final chronological yearly candidate set. WKST is not semantically active for these yearly BYDAY forms unless BYWEEKNO is present, in which case numeric BYDAY is rejected.

Yearly BYWEEKNO uses the same persisted WKST to define numbered seven-day weeks. Week 1 is the WKST-anchored week containing January 4, equivalently the first week containing at least four days of the numbered year. Positive selectors count forward from week 1 and negative selectors count backward from the final numbered week. A requested week 53 contributes no candidates in a 52-week week-number year. Plain BYDAY selects weekdays inside the chosen numbered weeks; when BYDAY is absent, the DTSTART weekday is preserved inside each selected week. BYMONTH and BYYEARDAY filter those candidates, and BYMONTHDAY can further filter them by each candidate date's own civil month. Ordinal BYDAY is rejected with BYWEEKNO. Candidate dates are sorted/deduplicated before BYSETPOS and COUNT, and exact/source-timezone events retain source wall-clock time.

Monthly recurrence first selects the active month from DTSTART plus INTERVAL. BYMONTH, when present, then limits that cadence to selected calendar months. Monthly plain BYDAY expands every occurrence of each selected weekday inside a surviving active month. Plain and ordinal BYDAY selectors are combined as one civil-date union and deduplicated. When BYMONTHDAY is present, its resolved date set filters that BYDAY union rather than adding unrelated dates. First-month dates before DTSTART are omitted. WKST does not alter monthly BYDAY semantics, so custom WKST remains reserved for weekly BYDAY and yearly BYWEEKNO contexts. BYSETPOS then operates on the chronological filtered monthly candidate set, enabling rules such as the last weekday of each month. A finite 400-year / 4,800-month reachability pass evaluates BYMONTH, BYDAY, ordinal BYDAY, BYMONTHDAY, and BYSETPOS candidate cardinality before expansion; permanently empty monthly rules therefore terminate rather than looping indefinitely.

Monthly BYMONTHDAY recurrence resolves positive selectors from the start of the month and negative selectors backward from month-end (`-1` is the last day, `-2` the penultimate day, and so on). Resolved civil dates are sorted chronologically and deduplicated before occurrence counting, so different selectors that resolve to the same date produce one occurrence. In the first active month, resolved dates earlier than DTSTART are omitted; DTSTART itself is an RRULE occurrence only when a selector resolves to it. `interval = N` means every Nth month measured from DTSTART's month. A selector that cannot resolve inside a particular month is skipped rather than coerced.

Monthly ordinal BYDAY recurrence resolves selectors such as `1 Monday` (first Monday), `-1 Friday` (last Friday), and `5 Monday` (fifth Monday). Positive ordinals count forward from month start; negative ordinals count backward from month end. Resolved dates are sorted and deduplicated before occurrence counting. A missing fifth weekday simply contributes no candidate for that month. In the first active month, resolved dates before DTSTART are omitted.

When monthly BYMONTHDAY is combined with BYDAY, Ephemeris resolves the complete plain+ordinal BYDAY union for the active month and then keeps only dates also selected by BYMONTHDAY. That filtered set is sorted/deduplicated before BYSETPOS and COUNT are consumed and before EXDATE or occurrence overrides are applied. This preserves the same original-slot identity and override-target validation used by single-selector monthly series.

BYSETPOS is applied after the candidate dates/times for all currently supported BY selectors are resolved inside one recurrence interval, but before COUNT is consumed and before EXDATE/occurrence overrides are applied. Daily, weekly, monthly, and yearly reachability guards account for BYSETPOS candidate cardinality, so impossible positions such as requesting position 2 from a permanently one-candidate interval do not create unbounded searches. Positive positions count from the start of that chronological candidate set and negative positions count backward from its end (`-1` is the final candidate). A position outside the interval's candidate-set size selects nothing. Different positions that resolve to the same candidate are deduplicated. RDATE additions are not part of the BYSETPOS candidate set.

Yearly BYYEARDAY resolves positive selectors from January 1 and negative selectors backward from the end of the active recurrence year (`-1` is December 31). Day 366 is valid only in leap years; nonexistent year-day candidates are skipped and do not consume COUNT. BYYEARDAY candidates are sorted and deduplicated, and first-year candidates before DTSTART are omitted. When BYMONTH is also present it filters BYYEARDAY candidates by month. BYMONTHDAY further filters resolved BYYEARDAY or BYWEEKNO candidates by each candidate's own month, whether or not BYMONTH is explicit; ordinal BYDAY filters either month-scoped dates with BYMONTH or whole-year ordinal dates without BYMONTH.

Yearly selector reachability is checked over the finite 400-year Gregorian cycle before unbounded expansion. INTERVAL is reduced modulo 400 for this reachability pass, and every distinct calendar state is tested using the existing yearly candidate generator with BYSETPOS applied. The pass evaluates future-cycle periods rather than only period zero, so first-year DTSTART filtering cannot falsely classify a rule such as a January selector beginning on December 31 as permanently empty. Impossible combinations such as February 30, a never-leap BYYEARDAY cadence, or oversized BYSETPOS therefore terminate cleanly.

Yearly BYMONTH recurrence defines the active month set inside each recurrence year. With no additional yearly month-scoped selector, each selected month preserves DTSTART's civil day and local/source-wall-clock time. With signed BYMONTHDAY, selected month-days are resolved independently inside each selected month. With ordinal BYDAY, selectors such as first Monday or last Friday are likewise resolved inside each selected month; without BYMONTH those same ordinal selectors instead resolve against the whole year. If both BYMONTHDAY and month-scoped ordinal BYDAY are present, only civil dates produced by both selector families survive inside each selected month. Resolved dates are sorted and deduplicated before COUNT is consumed. In the first active year, candidates before DTSTART are omitted. Impossible dates and missing fifth weekdays are skipped rather than coerced. Yearly BYMONTHDAY does not require BYMONTH: without it, signed month-day selectors are resolved independently in every month of the recurrence year. Yearly ordinal BYDAY likewise does not require BYMONTH, but changes scope: without BYMONTH its ordinal is measured across the whole year.

Monthly/yearly recurrence without explicit monthly/yearly selectors preserves the original calendar month/day. An invalid target date is skipped rather than coerced to month-end. Count applies to valid RRULE-generated occurrences across selected weekly BYDAY, monthly plain/ordinal BYDAY and BYMONTHDAY candidates, and yearly selector candidates; RDATE additions do not consume the RRULE count, and EXDATE is applied after candidate generation.

Conceptually, the visible occurrence set is built from RRULE candidates plus RDATE additions, with EXDATE removing matching original slots and occurrence overrides transforming matching slots. A moved override changes the rendered time without changing the identity of the original slot. A cancelled override remains materialized with cancelled lifecycle status; cancellation is therefore not equivalent to EXDATE disappearance.

Still required for fuller interoperable recurrence semantics:

- broader RRULE dimensions beyond the current frequency/interval/count/until/daily+weekly BYDAY/all-frequency BYMONTH/daily+monthly+yearly BYMONTHDAY/WKST/monthly plain+ordinal BYDAY/yearly plain+ordinal BYDAY/BYWEEKNO/BYYEARDAY/BYSETPOS, including additional RFC selector families and combinations;
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

Week start for rendering/grouping is a view/user preference, not canonical event data. RRULE week calculation is separate: weekly BYDAY and yearly BYWEEKNO recurrence use the persisted WKST value, defaulting to Monday.

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
