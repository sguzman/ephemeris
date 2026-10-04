# Project Lineage

## Genealogy

```text
Rivet
  broad personal-productivity application
  Rust task core + Tauri backend + React frontend
        |
        v
Rivetr
  native Rust + eframe/egui successor
  intended to preserve the broad Rivet product
        |
        +----> Ephemeris
                 dedicated temporal-information application
```

## Rivet

Rivet began as a Taskwarrior-like system with a Rust task engine and grew into a broader desktop productivity application.

Its implemented or planned desktop surfaces included:

- Tasks
- Kanban
- Calendar
- Contacts
- Dictionary
- Map

Its GUI used a Tauri backend and React/TypeScript frontend. The calendar subsystem became substantial, including multiple calendar views and external calendar ingestion.

Rivet remains the historical source of several domain ideas and compatibility behaviors, but it is not the architecture Ephemeris intends to revive.

## Rivetr

Rivetr was created as the native Rust + `eframe`/`egui` successor to Rivet.

It retained the useful Rust task engine and reimplemented desktop surfaces natively.

At the point Ephemeris was created, Rivetr already contained substantial calendar work:

- year view
- quarter view
- month view
- week view
- day view
- task/event filtering
- source colors
- local ICS import
- re-import reconciliation
- JSON temporal bundle import
- persisted UI state
- keyboard interaction
- dense calendar rendering work

Rivetr also retained or began rebuilding the broader Rivet product: tasks, Kanban, dictionary, contacts, and map.

## Why Ephemeris was split out

A brief attempt was made to redefine Rivetr itself as a calendar-only application.

That direction was rejected.

The reason was conceptual rather than technical: Rivetr has a legitimate identity as the broad native successor to Rivet. Turning it into a dedicated calendar would erase or demote the very product scope it was created to preserve.

The calendar problem had become sufficiently deep to deserve its own product instead.

Ephemeris therefore exists as a **descendant/spinoff**, not as a rename or hostile takeover of Rivetr.

## Inheritance policy

Ephemeris may reuse or adapt proven Rivetr calendar code when doing so saves work and preserves correct behavior.

However, inheritance is selective.

Ephemeris does not automatically inherit:

- Rivetr's `TaskDto` as canonical event storage
- task-centric semantics
- standalone task UI
- Kanban
- contacts workspace
- dictionary workspace
- map workspace
- product obligations inherited from Rivet

Any copied code must be re-evaluated against the Ephemeris product contract.

## Identity rule

The project roles are:

- **Rivet**: historical broad Tauri productivity application
- **Rivetr**: native broad successor to Rivet
- **Ephemeris**: dedicated temporal-information system descended from Rivetr's calendar work
- **Taria**: source/knowledge ecosystem that can provide rich temporal resources to Ephemeris

This distinction should remain visible in documentation and project history.
