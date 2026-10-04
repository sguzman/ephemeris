# Rivetr Inheritance Map

## Purpose

Ephemeris is not starting from zero.

Rivetr already contains substantial native calendar implementation. The goal is to inherit proven behavior without inheriting Rivetr's broader product scope or task-centric canonical model.

This document is the initial extraction map. It should be updated as code is audited line by line.

## Immediate ancestor

Repository:

`sguzman/rivetr`

The important application crate is:

`crates/rivet_app`

## Known calendar-relevant files

### `src/calendar.rs`

Known responsibilities include calendar/date calculations and event projection helpers.

Likely inheritance class:

**high-value reusable/adaptable**

Audit for:

- functions that depend only on dates/time and calendar configuration
- functions that depend on `TaskDto`
- marker/source assumptions
- period statistics
- visibility rules

Pure date calculations should be strong candidates for direct extraction with tests.

### `src/app/calendar_ui.rs`

Substantial egui calendar UI.

Known behavior:

- year
- quarter
- month
- week
- day
- navigation
- side panels
- imported-calendar controls
- marker rendering
- statistics
- filtering

Likely inheritance class:

**high-value UI adaptation**

Do not copy blindly because it currently consumes Rivetr's task-backed event projection.

Goal:

retain proven layout/navigation/rendering ideas while changing the data boundary to Ephemeris query/view results.

### `src/services.rs`

Known behavior:

- task CRUD
- local ICS import
- ICS parse
- source/event tags
- re-import reconciliation
- JSON bundle import
- create/update/delete counting
- event-to-task conversion

Likely inheritance class:

**mixed**

Potentially reusable:

- ICS parsing logic
- fixture-based import tests
- reconciliation concepts
- source UID matching behavior

Must be replaced/adapted:

- canonical storage through task files
- conversion of temporal events into tasks
- task lifecycle semantics standing in for event lifecycle
- tag-only source metadata

### `src/persistence.rs`

Known behavior:

- UI state persistence
- calendar focus date
- active calendar view
- side-panel visibility
- imported calendar list
- calendar tag filters
- Kanban/task UI state

Likely inheritance class:

**partial adaptation**

Ephemeris should retain the useful idea of native persisted UI state but separate:

- UI preferences
- saved views
- source definitions
- canonical temporal data
- credentials
- caches

### `src/types.rs`

Known behavior:

- `CalendarView`
- `ImportedCalendarSource`
- `CalendarConfig`
- `CalendarEntry`
- `CalendarMarkerKind`
- `TaskDto`
- runtime config DTOs

Likely inheritance class:

**selective**

Good candidates:

- calendar view enum concepts
- some config concepts
- marker/view presentation concepts

Do not inherit as canonical architecture:

- `TaskDto` as event
- task status as complete event lifecycle
- imported source model limited to a local file path
- calendar entry as only a task projection

### `src/runtime.rs`

Known behavior:

- runtime config loading
- calendar timezone
- week start
- visibility defaults
- day view bounds
- display toggles
- dictionary config

Likely inheritance class:

**calendar-specific extraction only**

### `src/app/keyboard.rs`

Known behavior:

- native keyboard shortcut resolution
- task selection/navigation
- workspace switching

Likely inheritance class:

**interaction patterns reusable**

Ephemeris should derive its own shortcut vocabulary around:

- date navigation
- view switching
- query/filtering
- event selection
- inspector
- bulk temporal operations

### `src/app/shell.rs`

Known behavior:

- top bar
- workspace switching
- status/error messages
- theme
- shortcuts dialog
- data/config display

Likely inheritance class:

**structural ideas only**

Ephemeris does not need Rivetr's multi-workspace navigation.

Useful pieces:

- native status/error surfaces
- theme handling
- visible operation state
- shortcut help

### `tests/compat_fixture.rs` and fixtures

Likely inheritance class:

**inspect for reusable temporal fixtures**

Task compatibility itself is not an Ephemeris goal.

## Known behavior worth preserving

### Calendar navigation

Rivetr already proved the standard views and navigation patterns.

Preserve user-visible continuity where it does not conflict with the richer model.

### ICS import

Rivetr can import local ICS and reconcile subsequent imports.

This is valuable prior art.

Ephemeris should preserve tests/behavior while moving the output to a canonical temporal store.

### Large calendar import

Rivetr contains a test for a large bundled sports calendar.

This should be recovered as an early scale/fixture test.

### Native UI state

The move away from WebView/browser-local storage is correct and should be preserved.

### Keyboard interaction

Rivetr already began keyboard-first native interaction. Ephemeris should extend that direction.

## Known behavior not to inherit

### Task-backed canonical events

The central architectural break.

Rivetr's calendar currently converts source events into task-compatible records.

Ephemeris needs direct temporal entities.

### Task status as event lifecycle

Task states:

- pending
- waiting
- completed
- deleted

are not sufficient for:

- tentative
- scheduled
- confirmed
- rescheduled
- postponed
- cancelled
- observed
- projected
- superseded
- disputed

### Tag encoding as primary ontology

Tags are useful but cannot be the only structure for source identity, jurisdiction, event type, relations, provenance, and lifecycle.

### Local-file-only source model

Rivetr's `ImportedCalendarSource` is oriented around imported local ICS files.

Ephemeris source modeling must cover Taria snapshots, remote feeds, APIs, generated projections, CalDAV, manual sources, and other forms.

### Machine-specific dictionary workaround

Rivetr contains a migration-era PostgreSQL host override in dictionary code.

Ephemeris should not copy unrelated or machine-specific code.

### Multi-workspace shell

Tasks, Kanban, Dictionary, Contacts, and Map belong to Rivetr's product identity, not Ephemeris.

## Extraction sequence

When implementation starts:

1. copy or reimplement pure date/calendar math with tests
2. bootstrap Ephemeris-native temporal types/store
3. adapt ICS parsing into normalized candidate events
4. port reconciliation fixtures to canonical event/source identities
5. adapt calendar rendering to query results rather than tasks
6. recover keyboard/navigation patterns
7. recover density/large-calendar behavior
8. leave unrelated Rivetr work untouched

## Rule

No Rivetr file should be copied merely because it already exists.

Every inherited component must answer:

- What semantic behavior are we preserving?
- Which Rivetr assumptions are being discarded?
- Which tests prove the behavior?
- Does this component depend on task identity or task persistence?
- Does it preserve Ephemeris provenance and time semantics?
