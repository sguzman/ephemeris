# Ephemeris Documentation

This directory is the design and project-memory surface for Ephemeris.

## Start here

1. [PRODUCT.md](PRODUCT.md)
2. [PROJECT_STATE.md](PROJECT_STATE.md)
3. [ROADMAP.md](ROADMAP.md)
4. [ARCHITECTURE.md](ARCHITECTURE.md)

## Domain and architecture

- [ARCHITECTURE.md](ARCHITECTURE.md) - system boundaries and layering
- [DATA_MODEL.md](DATA_MODEL.md) - canonical temporal entities and identity
- [TIME_SEMANTICS.md](TIME_SEMANTICS.md) - recurrence, timezones, all-day, floating time, lifecycle
- [QUERY_AND_VIEWS.md](QUERY_AND_VIEWS.md) - saved views, queries, overlays, color, grouping, sorting
- [TARIA_INTEGRATION.md](TARIA_INTEGRATION.md) - native Taria temporal handoff
- [TARIA_BUNDLE_CONTRACT.md](TARIA_BUNDLE_CONTRACT.md) - exact Resourcearium/Ephemeris release contract
- [TARIA_FILESYSTEM_WORKFLOW.md](TARIA_FILESYSTEM_WORKFLOW.md) - local Resourcearium path and one-click update workflow
- [INGESTION_AND_SYNC.md](INGESTION_AND_SYNC.md) - adapters, refresh, reconciliation, sync, export
- [RIVETR_INHERITANCE.md](RIVETR_INHERITANCE.md) - what to reuse from the immediate ancestor
- [LINEAGE.md](LINEAGE.md) - Rivet → Rivetr → Ephemeris genealogy

## Product behavior

- [UX.md](UX.md) - interaction and visualization contract
- [PERFORMANCE.md](PERFORMANCE.md) - scale and latency expectations
- [QUALITY.md](QUALITY.md) - correctness, tests, migrations, observability
- [FUTURE_CAPABILITIES.md](FUTURE_CAPABILITIES.md) - full long-horizon feature envelope
- [GLOSSARY.md](GLOSSARY.md) - project terminology
- [IMPLEMENTATION_HISTORY.md](IMPLEMENTATION_HISTORY.md) - archived detailed milestone/test checkpoint ledger

## Decisions

Architectural Decision Records live under [adr/](adr/).

Current ADRs:

- [0001-dedicated-calendar-product.md](adr/0001-dedicated-calendar-product.md)
- [0002-events-canonical-views-programmable.md](adr/0002-events-canonical-views-programmable.md)
- [0003-native-local-first-rust-egui.md](adr/0003-native-local-first-rust-egui.md)

## Maintenance rule

When a design or implementation decision changes project behavior, update the relevant canonical document and add an ADR when the change affects durable architecture.

Keep the root README and PROJECT_STATE concise. Historical implementation checkpoints belong in IMPLEMENTATION_HISTORY rather than being appended indefinitely to landing/current-state documents.

Do not rely on chat history as the only record of project decisions.
