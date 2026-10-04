# Rivetr Calendar Implementation Audit

Date: 2026-10-04

This audit closes Ephemeris Phase 0 and records the implementation boundary before code extraction.

## Summary

Rivetr contains enough proven calendar behavior to accelerate Ephemeris, but its calendar domain is not reusable as-is.

The clean split is:

- **reuse directly:** pure date-navigation/calendar math
- **adapt:** egui rendering/navigation patterns and calendar UI state
- **port behavior, not storage:** ICS parsing and refresh/reconciliation tests
- **reject as canonical Ephemeris architecture:** task-backed events, task statuses, task tags as ontology, task-file persistence

## calendar.rs

### Directly reusable semantics

The following functions are independent of the task model or can be trivially made so:

- shift_focus
- month shifting
- month_grid_start
- month_days
- quarter_months
- week_days
- year_months
- calendar_title

These functions are small, deterministic, and already have useful tests.

### Adapt, not copy

- entries_for_day
- entries_for_month
- period_entries

Their date-window semantics are useful, but they currently operate on CalendarEntry, whose payload contains TaskDto.

Ephemeris should instead query canonical temporal events for an explicit half-open time/date window.

### Reject

- visible_calendar_entries
- task_to_entry
- task-status visibility
- board/tag-derived marker semantics

These encode Rivetr's productivity-suite model into calendar data.

## app/calendar_ui.rs

The UI is substantial and validates that native egui is sufficient for the product.

Proven behavior includes:

- year / quarter / month / week / day switching
- prev / today / next navigation
- source panel
- details panel
- period statistics
- marker rendering
- period drill-down
- dense cell behavior
- imported calendar controls

The layout/rendering approach is reusable, but the file is tightly coupled to RivetApp, task-backed CalendarEntry, Kanban metadata, and Rivetr-specific import state.

Decision: reimplement the Ephemeris calendar surface using these interaction patterns rather than copying the module wholesale.

## services.rs

### Valuable behavior

Rivetr's local ICS path demonstrates:

- UID-based source-event identity
- first import creates records
- reimport updates same-UID records
- missing source records are reconciled
- large calendar fixtures are viable

The existing tests are valuable specifications.

### Architectural contamination

The imported event becomes a task:

- title -> task description
- calendar source -> tag
- event UID -> tag
- color -> tag
- calendar source name -> task project
- datetime -> task due
- disappearance -> task deleted state

Ephemeris must not preserve this mapping.

Decision: later port the parser/reconciliation tests onto TemporalSource, TemporalEvent, source records, and lifecycle/history semantics.

## Time parsing findings

Rivetr handles:

- RFC3339
- Taskwarrior UTC datetime
- iCalendar UTC datetime
- date-only DTSTART
- local DTSTART with TZID fallback

But date-only ICS values are converted to local midnight and then UTC.

That is not acceptable as Ephemeris canonical semantics because an all-day civil date must remain a date rather than an instant.

Decision: Ephemeris starts with a first-class TimeSpec that distinguishes at minimum:

- all-day civil date
- exact instant/zoned event
- floating local datetime

## Persistence

Rivetr correctly moved UI state out of browser local storage.

Useful persisted concepts:

- active calendar view
- focus date
- panel visibility
- filter state

Do not preserve the single-state-object assumption for all product data.

Ephemeris will separate:

- canonical temporal database
- source definitions
- UI state/preferences
- saved views
- caches

## Keyboard behavior

Rivetr's keyboard-first direction is retained.

The task-specific shortcuts are not.

## Final extraction decision

Phase 1 should begin with:

1. native Rust + egui application
2. direct temporal domain types
3. embedded transactional temporal store
4. extracted pure calendar math
5. standard calendar views rendering canonical events
6. source and event inspection foundations

ICS and Taria ingestion follow on top of that store.

No task compatibility layer will be inserted between ingestion and the calendar.
