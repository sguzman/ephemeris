# Personal Scheduling in Ephemeris

This guide describes the currently implemented local scheduling workflow. Ephemeris is still a general temporal-information system; personal scheduling is one consumer of its canonical event model.

## Create and edit an event

Use the application's **New event** control to open quick-create. Enter a title and a date/time or choose an all-day event. Optional fields include description, event type, domain, lifecycle status, and **Busy** / **Free** availability.

- **Busy** means the event can occupy time in free/busy calculations.
- **Free** means the event remains on the calendar but does not block availability.
- **All day** is a definite full-day commitment when Busy.
- **Date only** records a civil date without asserting that the entire day is blocked. It is not interchangeable with all-day.

Select a local/writable event to edit its canonical details, time, structured location, participants, or recurrence through their separate inspector sections. Source-backed read-only events remain source-owned; local annotations, topology, and reminder rules may still attach without rewriting the upstream assertion.

The time editor supports non-recurring exact instants, floating wall-clock times, all-day intervals, and date-only values. For exact zoned times it preserves the source clock context and rejects ambiguous or nonexistent local times rather than inventing an instant. Recurring master-time editing and bounded-uncertainty editing are deliberately not offered by this time editor yet.

## Check availability and find a slot

Open **Availability** for the current calendar range. The panel calculates blocking Busy/tentative intervals from the **currently visible query and source selection**, shows free intervals, and lists events skipped because they lack a concrete schedulable interval.

Under **Find a slot**, choose a duration, step, working-hour window, and enabled weekdays. These preferences persist. Use **Use slot** to prefill quick-create from a candidate interval, or **New event here** from a raw free interval.

The result depends on which events and sources are visible. A filtered view can therefore report a free slot that would not be free in the full canonical corpus. The panel deliberately reports skipped unsupported temporal shapes; it does not fabricate intervals for date-only or coarse/unresolved values.

## Save-time conflict confirmation

For concrete non-recurring blocking commitments, Ephemeris checks stored events for overlapping time intervals at save time. This check is broader than the current view filter.

The following workflows use conflict confirmation:

- Creating a local Busy event.
- Changing the time of a writable non-recurring Busy event.
- Changing details so a previously nonblocking event becomes a blocking commitment, such as Free → Busy or Cancelled → Scheduled.

If a collision is found, the first save presents the conflicting titles. **Save again without changing the scheduling fields** to accept that collision. Editing those fields invalidates the previous confirmation. A pure title/description edit, or Scheduled → Confirmed on an already-blocking event, does not introduce a new commitment and does not require this extra confirmation.

For a conflicting **timed** new event or time edit, Ephemeris also offers up to four later alternatives within the next 15 days, respecting the current work-hour/weekdays/step preferences. These proposals use the full stored canonical corpus, not the filtered calendar view. Selecting an alternative updates only the draft's date/time/duration, preserves title and metadata, and clears the previous conflict confirmation. Saving the revised time runs the conflict check again. Unrepresentable source-clock or DST-ambiguous choices are rejected. Date-only/all-day and uncertain/recurring candidate edits do not yet have automatic alternatives.

This is a conflict advisory, not a universal guarantee: events with unsupported temporal shapes can be skipped. The Availability panel reports skipped events and reasons separately. Bounded start-placement uncertainty must never be mistaken for a definitely occupied interval merely because a representative start is recorded. Recurring candidate-master edits and uncertainty-aware rescheduling are still outside this confirmation path. An event with no definite duration cannot be meaningfully checked as an interval.

## Reminders

Ephemeris stores before-start reminder rules locally in SQLite. A rule can target a canonical event or a SavedView. The evaluator understands supported recurrence occurrences and deduplicates deliveries by stable identity.

Open **Reminders** to see **Due**, **Snoozed**, and **Upcoming** entries. Due reminders can be dismissed or snoozed by 10 minutes, 30 minutes, one hour, or a custom interval from 1 to 10,080 minutes.

Snoozed reminders show their wake deadline. **Wake now** returns a reminder to Due immediately; **Dismiss** closes it. Snooze and dismissal state survive application restart. Once the snooze deadline arrives, a still-undismissed reminder becomes due again.

Reminders are an **in-app** workflow. This does not promise system notifications or reminders while Ephemeris is closed. The application refreshes its due-state evaluation while running.

Before-start reminders currently require exact or floating DATE-TIME events. All-day and date-only reminders need an explicit notification-clock model before they can be scheduled without guessing.

## Current boundaries

Ephemeris does not yet implement an attendee/invitation exchange workflow or CalDAV scheduling. It does not silently invite participants merely because they are listed in canonical event metadata.

Recurring master-time changes, unsupported uncertain placements, and external source-owned fields remain protected behind their existing semantic boundaries.

For data semantics see [TIME_SEMANTICS.md](TIME_SEMANTICS.md). For persistence and source ownership see [DATA_MODEL.md](DATA_MODEL.md) and [INGESTION_AND_SYNC.md](INGESTION_AND_SYNC.md). Current verification and priorities live in [PROJECT_STATE.md](PROJECT_STATE.md) and [ROADMAP.md](ROADMAP.md).
