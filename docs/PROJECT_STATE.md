# Project State

Last major state change: project documentation bootstrap.

## Current status

Ephemeris is a newly created repository with a formalized product and architecture, but no application implementation yet.

That is intentional.

The project is starting from an explicit temporal model rather than copying Rivetr wholesale and discovering later that task semantics became permanent architecture.

## Immediate ancestor

Repository:

- `sguzman/rivetr`

Relevant Rivetr calendar implementation includes:

- `crates/rivet_app/src/calendar.rs`
- `crates/rivet_app/src/app/calendar_ui.rs`
- `crates/rivet_app/src/persistence.rs`
- calendar-related portions of `services.rs`, `types.rs`, `runtime.rs`, and shell/keyboard code

Known useful behavior includes:

- year/quarter/month/week/day views
- navigation
- source color display
- local ICS import
- re-import reconciliation
- JSON temporal bundle import
- persisted calendar UI state
- keyboard behavior

## Known inheritance hazard

Rivetr currently represents calendar entries through task-compatible records.

That is useful compatibility scaffolding but should not become Ephemeris's canonical temporal ontology.

Before copying service/data code, classify each piece as:

- pure/reusable
- adaptable
- compatibility-only
- reject

## Next implementation task

Perform a focused Rivetr calendar inheritance audit and produce an explicit extraction map.

The audit should answer:

1. Which calendar/date functions are pure and directly reusable?
2. Which egui views can be ported without task coupling?
3. Which ICS parsing/reconciliation tests are reusable?
4. Which structures assume `TaskDto`?
5. Which persistence assumptions should be replaced?
6. Which configuration is genuinely calendar-specific?
7. Which Rivetr behavior should be preserved for user-visible continuity?

Only after that audit should the new Rust workspace be bootstrapped.

## Not started

- Rust workspace
- storage engine
- temporal schema
- Taria interchange
- source manager
- native UI
- migration/import pipeline

These are roadmap items, not missing undocumented work.
