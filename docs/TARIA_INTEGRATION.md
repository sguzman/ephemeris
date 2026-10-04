# Taria Integration

## Relationship

Taria / Resourcearium and Ephemeris solve different parts of the same temporal-information problem.

- **Taria / Resourcearium** owns source acquisition, source/resource identity, temporal assertions, normalization, reconciliation, projection, CalendarSet construction, coverage accounting, frozen bundle releases, rollover/source health, and rendered export artifacts.
- **Ephemeris** owns local adoption, indexing, interactive querying, inspection, saved views, grouping, sorting, color rules, overlays, annotations, and desktop calendar behavior.

Ephemeris consumes Taria's rich temporal products. It must not reproduce Taria's acquisition pipeline and must not flatten Taria through ICS before ingestion.

The canonical producer/consumer agreement is:

- [TARIA_BUNDLE_CONTRACT.md](TARIA_BUNDLE_CONTRACT.md)

## Upstream lifecycle

The accepted Taria lifecycle is now:

```text
raw temporal sources
    -> acquisition
    -> NormalizedEventSnapshot
    -> ProjectionEventSet
    -> ReconciledProjectionEventSet
    -> CalendarSet
    -> TemporalBundleRelease
    -> Ephemeris adoption
```

Rendered ICS / JSCalendar / jCal / CSV artifacts are downstream projections, not the canonical Taria -> Ephemeris interchange.

## Primary consumer boundary

The primary handoff is now a **TemporalBundleRelease manifest plus its referenced frozen artifacts**.

### Default transport: local filesystem

Ephemeris normally consumes that release directly from the local Taria checkout.

The configured path may point to the Taria repository root or directly to `incubator/resourcearium`.

Normal flow:

```text
local Resourcearium root
    -> registry/temporal-bundle-releases.yml
    -> selected channel
    -> local immutable release manifest
    -> local referenced artifacts
    -> Ephemeris SQLite
```

No download, GitHub API, or HTTP step is required.

The implemented UI exposes **Update Taria Sources** in the toolbar and Sources panel.

Canonical upstream files include:

- `registry/temporal-bundle-release-schema.yml`
- `registry/temporal-bundle-releases.yml`
- `registry/temporal-bundle-projection-ontology.yml`
- `registry/temporal-calendar-set-schema.yml`
- `registry/temporal-projection-reconciliation-schema.yml`
- `registry/temporal-normalized-event-snapshot-schema.yml`

A release is packaging over already-frozen derivative state. It does not redefine canonical event identity.

## Release channels

Taria currently exposes two logical channels:

- `bootstrap`
- `production`

Release manifests are immutable. Channel pointers may advance. Older releases remain addressable.

Ephemeris may pin a specific release ID and must not silently rewrite that release merely because a channel advances.

### Bootstrap

Bootstrap releases are valid for integration work and client development.

They may contain specimen data and partial bundle coverage.

They must remain visibly non-production.

### Production

Production may be:

- `production-partial`
- `production-complete`

A production-partial release is still consumable. Missing coverage is a property of the release, not evidence that the real world contains zero events.

## Current Taria bundle state

At the current 2026-10-04 boundary, Resourcearium has:

- 429 canonical temporal Resources;
- 221 pinned RICS profiles in the broader crosswalk;
- 177 direct single-Resource RICS profiles in the first exact-lineage production tranche;
- resilient batch acquisition tooling;
- normalization into a master `NormalizedEventSnapshot`;
- 13 canonical projection builds from one frozen snapshot;
- reconciliation;
- CalendarSet construction;
- JSON/ICS rendering;
- immutable TemporalBundleRelease packaging and validation.

The current bootstrap release is:

```text
temporal-bundle-release:bootstrap:2026-10-04:r4
```

It is explicitly `bootstrap-partial` and currently exposes:

- 1,114 ready events;
- 15 represented canonical Resource identities;
- 2 typed recovered ingestion profiles;
- 196 recovered source surfaces;
- 3 partially populated canonical domain slots;
- 9 pending canonical domain slots;
- consumer-safe integration posture;
- non-production-complete status.

Populated data currently includes:

- 1,038 recovered 2026 U.S. Politics events;
- 11 2027 European national-election events;
- 37 2026 U.S. pro-sports events excluding hockey;
- 28 recovered U.S. Holidays events.

The recovered U.S. Politics and Holidays shards are explicitly downstream-derived recovery state rather than fresh upstream publisher observations.

The production channel remains independently advanceable as acquisition coverage improves.

## Canonical bundle ontology

The canonical domain slots are:

- Politics & Government
- Economics & Public Statistics
- Finance & Markets
- Business & Corporate
- Sports & Competition
- Culture & Media
- Science, Technology & Space
- Public Health
- Environment & Weather
- Transportation & Civic Infrastructure
- Holidays & Observances
- Education & Academia

The logical root is:

```text
bundle:temporal/everything
```

Domain membership is intentionally non-exclusive.

The same event may belong to several canonical bundles without becoming several event identities.

## Event payload versus CalendarSet membership

This distinction is fundamental.

### ReconciledProjectionEventSet

The reconciled event set supplies rich event payload:

- reconciled event identity;
- upstream event identity;
- assertion/source/provenance refs;
- display fields;
- temporal values;
- lifecycle and renderability;
- source contexts/facets;
- field-resolution decisions.

Ephemeris already has a direct importer for this shape.

### CalendarSet

CalendarSet supplies logical membership/navigation metadata:

- projected calendars;
- merged and partition calendars;
- event membership by reconciled-event reference;
- groups/hierarchy;
- partition values;
- blocked/undated accounting.

CalendarSet references events. It does not replace event payload.

Therefore the consumer mapping is:

```text
ReconciledProjectionEventSet
    -> canonical local TemporalEvent rows

CalendarSet
    -> local bundle/calendar membership metadata
```

## Overlapping bundle import

Canonical domain bundles overlap by design.

Ephemeris must never create one local event per bundle membership.

For example:

```text
one FOMC event
    -> politics-government
    -> economics-public-statistics
    -> finance-markets
```

must remain:

```text
one local TemporalEvent
    + three bundle memberships
```

Stable upstream event/reconciled-event identity is authoritative for deduplication.

## Release-v1 packaging variants

Taria currently validates two release-v1 packaging forms.

### Bootstrap-partial

Bootstrap manifests use:

```text
shards[]
```

A shard identifies a bundle, frozen normalized snapshot, CalendarSet, counts, integrity fingerprints, data posture, and an accepted post-reconciliation payload.

Bootstrap r4 uses two payload classes:

- recovered Politics/Holidays -> `CompactReconciledEventIndex`;
- Elections/Sports -> full `ReconciledProjectionEventSet`.

All four populated r4 shards are therefore directly consumable.

### Production partial/complete

The production packager uses:

```text
bundle_artifacts[]
```

Each artifact carries paths/hashes for:

- ProjectionEventSet;
- ReconciledProjectionEventSet;
- CalendarSet;
- rendered JSON;
- rendered ICS;
- ready/blocked/undated counts.

The aggregate carries the materialized `bundle:temporal/everything` CalendarSet.

Ephemeris treats these as packaging variants of the same release concept.

## Current bootstrap payload posture

Bootstrap r4 closes the prior payload gap.

Current populated shards:

- U.S. Politics recovery -> pinned `CompactReconciledEventIndex`;
- U.S. Holidays recovery -> pinned `CompactReconciledEventIndex`;
- 2027 European national elections -> pinned `ReconciledProjectionEventSet`;
- 2026 U.S. pro sports excluding hockey -> pinned `ReconciledProjectionEventSet`.

Ephemeris now consumes both payload classes through the same local release updater and persists each shard's CalendarSet independently.

CalendarSet remains membership metadata rather than event payload, and ICS is not used as a bridge.

## Partial coverage semantics

Ephemeris must preserve Taria's coverage posture.

These states are distinct:

- populated;
- partial;
- pending;
- gap-only;
- selected-but-uningested.

In particular:

```text
pending != zero events
gap-only != zero events
selected-but-uningested != empty source
```

A partial release is still useful and may be adopted immediately.

The UI should eventually expose release coverage and acquisition posture directly.

## Identity

Taria separates:

- assertion identity;
- event identity;
- occurrence identity;
- series identity;
- event-version identity.

Ephemeris must preserve that distinction.

An internal UUID is a local database key, not a replacement for upstream identity.

Important retained IDs include:

- `event_ref`;
- `reconciled_event_key` / reconciled-event refs;
- assertion refs;
- source refs;
- provenance refs;
- occurrence refs;
- series refs;
- version refs.

A changed time alone does not imply a new event.

Cancellation does not erase event identity.

Unresolved continuity must remain unresolved rather than being forced.

## Temporal precision

Taria distinguishes:

- exact instant;
- local datetime;
- date-only;
- explicit all-day date;
- interval;
- month precision;
- year precision;
- unknown/unresolved.

Critical rules:

- all-day date is not a midnight instant;
- date-only is not automatically all-day;
- timezone-unknown must not be guessed;
- floating-local is not timezone-unknown;
- month/year precision must not invent a day;
- original published values and inference evidence should remain recoverable.

Ephemeris already models these distinctions locally.

## Renderability

Reconciled Taria events can be:

- ready;
- blocked by temporal conflict;
- blocked by operative-status conflict;
- blocked by multiple conflicts;
- undated/unresolved.

Blocked and undated records remain locally inspectable.

They are not silently discarded or assigned fake dates.

## Provenance

Taria provenance includes:

- Resource/source lineage;
- RICS/profile lineage where applicable;
- acquisition/capture state;
- transformations;
- assertion refs;
- snapshot refs;
- reconciliation decisions;
- evidence/authority posture.

Ephemeris should eventually be able to answer not just "what event is this?" but "why does Taria believe this event exists in this state?"

## Release adoption

Release adoption should be transactional:

```text
validate release
    -> verify identities/hashes
    -> resolve rich event payloads
    -> reconcile/upsert canonical local events
    -> import CalendarSet membership
    -> retain explicit coverage gaps
    -> preserve local annotations
    -> record adopted release metadata
    -> commit
```

Failure must leave the previously adopted local state usable.

A newer release is a new frozen upstream state, not merely "the same URL fetched again."

## Missing records

A record disappearing from a later release does not automatically mean the real-world event was cancelled or should be deleted.

Ephemeris keeps its current conservative rule:

- explicit upstream cancellation/status change is meaningful;
- disappearance alone is not cancellation;
- identity/history remains inspectable;
- future snapshot/history support may make supersession explicit.

## Runtime views are consumer-owned

Taria materializes canonical bundles and selected durable derivatives.

Ephemeris owns ad-hoc runtime views such as:

- US politics excluding hearings over the next 45 days;
- California courts plus federal elections;
- high-importance economic releases this month;
- finance events overlaid with government meetings;
- sports grouped by league and colored by lifecycle status.

These do not require Resourcearium to rebuild a bundle.

## Current Ephemeris support

Implemented now:

- direct ReconciledProjectionEventSet import;
- direct CompactReconciledEventIndex import;
- stable source/import-record identity;
- persistent many-to-one upstream event/reconciled identity aliases;
- Taria source/provenance refs;
- blocked/unplaced preservation;
- transactional per-source re-import;
- created/updated/unchanged/retained-missing accounting;
- persisted local Resourcearium root;
- persisted bootstrap/production channel selection;
- auto-detection of common local Taria checkout locations including `$HOME/Code/Text/taria`;
- local channel registry and immutable manifest resolution;
- Resourcearium-root path confinement;
- bootstrap content-fingerprint validation;
- production file-SHA validation;
- immutable release metadata in SQLite;
- immutable CalendarSets plus release associations;
- projected-calendar and event-membership persistence;
- bootstrap `shards[]` adoption;
- production `bundle_artifacts[]` adoption;
- global cross-bundle event deduplication;
- one-click **Update Taria Sources**;
- persisted last release/update summary;
- explicit skipped-artifact reporting.

Current bootstrap r4 behavior:

- recovered Politics -> imported from CompactReconciledEventIndex;
- recovered Holidays -> imported from CompactReconciledEventIndex;
- Elections -> imported from ReconciledProjectionEventSet;
- Sports -> imported from ReconciledProjectionEventSet;
- all four CalendarSets/memberships are retained independently.

Production support is implemented and regression-tested with overlapping domain projections. The live Resourcearium production channel is still unset.

Not implemented yet:

- release coverage UI;
- whole-release transaction/rollback across multiple payloads;
- asynchronous large-release adoption;
- release-history/diff inspection.

Those are now the next Taria-facing integration boundary.

## Rollover and source health

Taria owns upstream rollover and source-health semantics.

Ephemeris should consume and display that state rather than invent a competing source-discovery/acquisition system.
