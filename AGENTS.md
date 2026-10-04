# Agent Operating Contract

This file applies to automated coding agents and assistants working in Ephemeris.

## Identity

Ephemeris is a dedicated local-first temporal-information application.

Do not reinterpret it as:

- a general productivity suite
- a Taskwarrior clone
- Rivetr renamed
- a cloud calendar frontend
- a Tauri/React application

## Current implementation phase

Read `docs/PROJECT_STATE.md` before making changes.

At bootstrap, implementation has not started. The next required engineering step is the Rivetr calendar inheritance audit, not speculative feature coding.

## Source of truth

Product and architecture documentation is authoritative unless explicitly revised.

Relevant documents:

- `docs/PRODUCT.md`
- `docs/ARCHITECTURE.md`
- `docs/DATA_MODEL.md`
- `docs/TIME_SEMANTICS.md`
- `docs/TARIA_INTEGRATION.md`
- `docs/ROADMAP.md`
- `docs/adr/`

## Rivetr

Rivetr is the implementation ancestor.

Reuse calendar code selectively.

Do not make `TaskDto` the canonical Ephemeris event model merely to simplify migration.

## Technical direction

Default assumptions unless an ADR changes them:

- Rust
- native `eframe`/`egui`
- local-first
- no Tauri
- no React/WebView application shell
- no required network calls for rendering/query
- strong provenance
- transactional imports
- correctness before feature breadth

## Work discipline

For every non-trivial change:

1. identify the product requirement
2. identify affected domain semantics
3. preserve or add tests
4. avoid unrelated cleanup
5. update documentation when behavior/contracts change
6. record architectural decisions durably

## Safety rails

Never:

- silently discard unknown source fields during a migration without policy
- overwrite user annotations during source refresh
- treat source disappearance as cancellation without source-specific semantics
- deduplicate events solely by title/date
- convert all-day dates through UTC in a way that moves civil dates
- block the egui frame loop on network or large imports
- hard-code machine-specific service addresses as canonical behavior
