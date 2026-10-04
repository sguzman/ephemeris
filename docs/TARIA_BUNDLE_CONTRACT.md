# Taria Temporal Bundle Consumer Contract

Status: **accepted Ephemeris consumer contract v1**

Established: 2026-10-04

This document defines the boundary between Taria / Resourcearium as the producer of temporal bundle releases and Ephemeris as a local interactive consumer.

Resourcearium now carries the reciprocal producer-side handoff at:

- `incubator/resourcearium/TEMPORAL_BUNDLE_EPHEMERIS_HANDOFF.md`

The two documents describe the same boundary from opposite sides.

The contract exists so the two projects can evolve independently without collapsing acquisition, canonical event state, bundle packaging, and desktop presentation into one system.

## 1. Ownership boundary

### Taria / Resourcearium owns

Taria owns the upstream temporal-data pipeline:

```text
raw temporal sources
    -> acquisition
    -> source/resource attribution
    -> NormalizedEventSnapshot
    -> ProjectionEventSet
    -> ReconciledProjectionEventSet
    -> CalendarSet
    -> TemporalBundleRelease
```

Taria therefore owns:

- the canonical temporal Resource universe;
- RICS/profile/source acquisition;
- source-family and source-surface lineage;
- normalization;
- event assertion provenance;
- identity/revision reasoning;
- projection definitions;
- reconciliation;
- CalendarSet construction;
- bundle-domain ontology;
- selected-but-uningested accounting;
- bundle coverage/completeness state;
- frozen release packaging;
- rendered ICS / JSCalendar / jCal / CSV derivatives.

### Ephemeris owns

Ephemeris owns the consumer runtime:

- local import and indexing;
- one canonical local event record per upstream event identity;
- local release-adoption state;
- fast search/query;
- saved views;
- grouping;
- sorting;
- color rules;
- overlays and composition;
- local annotations;
- desktop visualization;
- ordinary personal-calendar behavior.

Ephemeris does **not** acquire Taria's individual upstream sources and does not need to understand every RICS profile in order to consume a release.

## 2. Release is the primary handoff

The preferred producer/consumer boundary is a **TemporalBundleRelease manifest plus its referenced frozen artifacts**.

### Transport is filesystem-first

The normal transport is the local filesystem.

Ephemeris is configured with a local Taria/Resourcearium path and reads:

```text
registry/temporal-bundle-releases.yml
    -> selected local channel
    -> immutable local release manifest
    -> local referenced artifacts
```

directly from disk.

No GitHub/API/HTTP download step is part of normal operation.

The configured path may point to:

- the Taria repository root;
- `incubator/resourcearium` directly;
- or the local release-registry file.

Ephemeris normalizes these forms to the Resourcearium root.

See `TARIA_FILESYSTEM_WORKFLOW.md` for the implemented operator workflow.

Canonical upstream files:

- `registry/temporal-bundle-release-schema.yml`
- `registry/temporal-bundle-releases.yml`

Release manifests are immutable.

Release channels are movable pointers.

Older release IDs remain addressable for reproducibility.

Ephemeris may pin a release ID and does not have to advance merely because a channel advances.

## 3. Channel semantics

Taria currently defines:

- `bootstrap`
- `production`

### Bootstrap

Bootstrap releases are valid for integration work and immediate client development.

They may contain specimen shards and partial domain coverage.

They must never be presented as production-complete.

### Production

Production may point to:

- `production-partial`
- `production-complete`

A production-partial release is still consumable.

Incomplete source coverage must remain visible as coverage state rather than being interpreted as an empty world.

## 4. Accepted release statuses

Ephemeris may import:

- `bootstrap-partial`
- `production-partial`
- `production-complete`

A `superseded` release remains reproducible and may be imported explicitly, but Ephemeris should not auto-select it as the newest channel release.

## 5. Canonical bundle slots

The canonical domain slots are:

1. `bundle:temporal/politics-government`
2. `bundle:temporal/economics-public-statistics`
3. `bundle:temporal/finance-markets`
4. `bundle:temporal/business-corporate`
5. `bundle:temporal/sports-competition`
6. `bundle:temporal/culture-media`
7. `bundle:temporal/science-technology-space`
8. `bundle:temporal/public-health`
9. `bundle:temporal/environment-weather`
10. `bundle:temporal/transportation-civic-infrastructure`
11. `bundle:temporal/holidays-observances`
12. `bundle:temporal/education-academia`

The logical aggregate is:

```text
bundle:temporal/everything
```

Bundle-domain membership is non-exclusive.

A canonical event may belong to several domain bundles while remaining one event identity.

## 6. Event payload versus membership metadata

This distinction is mandatory.

### ReconciledProjectionEventSet

The reconciled event set supplies event payload:

- reconciled event identity;
- upstream event identity;
- display fields;
- temporal value;
- lifecycle/renderability state;
- source contexts;
- assertion/source/provenance references;
- field-resolution decisions.

### CalendarSet

CalendarSet supplies logical membership/navigation metadata:

- projected calendar IDs;
- merged/partition calendars;
- event membership by `reconciled_event_ref`;
- groups/hierarchy;
- partition values;
- blocked/undated accounting.

CalendarSet does **not** replace event payload.

Ephemeris therefore consumes both concepts:

```text
ReconciledProjectionEventSet
    -> canonical local events

CalendarSet
    -> local bundle/calendar membership metadata
```

Calendar membership must never create duplicate local events.

## 7. Overlapping bundle import

The twelve canonical domain bundles overlap by design.

Therefore Ephemeris must not do this:

```text
Politics event -> local event A
Economics copy -> local event B
Finance copy -> local event C
```

It must instead do this:

```text
stable upstream event identity
    -> one local TemporalEvent

bundle memberships
    -> politics
    -> economics
    -> finance
```

Deduplication should prefer the strongest upstream identities already retained by Resourcearium, including reconciled-event/event refs and source-record identity.

## 8. Release-v1 packaging variants

Taria currently has two validated release-v1 packaging shapes.

### Bootstrap-partial

Bootstrap manifests use:

```text
shards[]
```

Each shard identifies:

- `shard_id`;
- `bundle_ref`;
- normalized snapshot ref/path/fingerprint;
- CalendarSet ref/path/fingerprint;
- ready-event and source counts;
- data posture;
- and, for some recovered shards, a pinned `CompactReconciledEventIndex` path/hash.

### Production partial/complete

The production packager uses:

```text
bundle_artifacts[]
```

Each artifact identifies:

- `artifact_id`
- `bundle_ref`
- `projection_ref`
- ProjectionEventSet path/hash;
- ReconciledProjectionEventSet path/hash;
- CalendarSet path/hash;
- rendered JSON directory;
- rendered ICS directory;
- ready/blocked/undated counts.

The aggregate identifies the materialized `bundle:temporal/everything` CalendarSet.

Ephemeris should treat both as release-v1 packaging variants, not as different temporal ontologies.

## 8.1. Upstream schema/validator convergence

The current Resourcearium implementation has one release-v1 schema-description mismatch:

- `temporal-bundle-release-schema.yml` globally lists `shards` as required;
- the validator conditionally expects `shards[]` for `bootstrap-partial`;
- the validator conditionally expects `bundle_artifacts[]` for `production-partial` / `production-complete`;
- the production packager emits `bundle_artifacts[]`.

Ephemeris treats the **validated status-discriminated behavior** as the operative v1 contract.

Resourcearium should update its machine-readable release schema so the conditional packaging variants are expressed formally rather than leaving the schema and validator divergent.

Ephemeris should not invent a third packaging shape.

## 9. Required event-payload resolution

For Ephemeris to import a populated release slot, it needs a resolvable rich event payload.

Preferred full payload:

```text
ReconciledProjectionEventSet
```

because that is the rich event shape Ephemeris already ingests.

A release may also expose an explicitly versioned post-reconciliation compact payload when its semantics are pinned.

Current accepted compact bootstrap payload:

```text
CompactReconciledEventIndex
```

This object is acceptable for recovered shards because it is explicitly post-reconciliation and retains stable reconciled-event/event/assertion/provenance identity plus temporal/display fields. Its own semantics state that the full normalized snapshot remains authoritative.

A consumer-ready artifact therefore needs one of:

1. `reconciled_event_set_path` plus integrity hash; or
2. `event_index_path` plus integrity hash when the object kind is the accepted `CompactReconciledEventIndex`; or
3. another explicitly versioned post-reconciliation rich payload whose semantics are sufficient to reconstruct local canonical event state without guessing.

CalendarSet alone is insufficient because it contains event references and membership, not complete event payload.

### Current bootstrap payload posture

Bootstrap r4 is fully payload-resolvable for every populated shard:

- recovered U.S. Politics -> `CompactReconciledEventIndex`;
- recovered U.S. Holidays -> `CompactReconciledEventIndex`;
- 2027 European national elections -> `ReconciledProjectionEventSet`;
- 2026 U.S. pro sports excluding hockey -> `ReconciledProjectionEventSet`.

Ephemeris implements both adapters and can adopt all four populated shards from the local r4 manifest.

Bootstrap integrity values are Resourcearium content fingerprints, not necessarily raw file hashes. Ephemeris validates the embedded `content_fingerprint.value` according to the producer contract.

CalendarSet is still membership metadata, not event payload. ICS remains an unacceptable interchange workaround.

## 10. Partial coverage semantics

Ephemeris must preserve release coverage posture.

These states are not synonyms:

- populated;
- partial;
- pending;
- gap-only.

In particular:

```text
pending != zero real-world events
gap-only != zero real-world events
selected-but-uningested != empty source
```

A partial release is still useful and may be adopted immediately.

The UI should eventually expose:

- release status;
- production-complete flag;
- represented Resource count;
- selected-but-uningested count;
- populated/pending/gap-only bundle slots;
- ready/blocked/undated event counts;
- data posture such as specimen/frozen-rebuild/fresh-observation/mixed.

## 11. Release validation

Before transactional adoption, Ephemeris should validate:

- release schema version;
- release status;
- release ID;
- canonical bundle-slot set;
- referenced artifact existence;
- declared SHA-256 hashes/fingerprints;
- CalendarSet identity/fingerprint;
- event-payload identity where provided;
- aggregate identity;
- coverage/accounting fields.

A failed release validation must not corrupt the currently imported corpus.

## 12. Local release identity

Ephemeris should eventually retain local metadata such as:

- release ID;
- channel used for discovery;
- manifest hash;
- generated-at time;
- local import time;
- release status;
- production-complete flag;
- normalized snapshot ref/fingerprint;
- artifact/shard IDs;
- CalendarSet refs/hashes;
- coverage summary.

A new release is a new frozen upstream state.

It is not merely "the same source fetched again."

## 13. Release adoption semantics

Adopting a newer release should be transactional.

Conceptually:

```text
validate release
    -> resolve rich event payloads
    -> reconcile/upsert canonical local events
    -> import CalendarSet memberships
    -> preserve explicit gaps/coverage
    -> preserve local annotations
    -> record adopted release metadata
    -> commit
```

If adoption fails, the previous local release state remains usable.

Local annotations must survive upstream refresh/adoption.

## 14. Missing records

A record missing from a later release does not automatically mean the real-world event should be deleted.

Ephemeris should preserve the conservative behavior already used by its Taria reconciled-event-set importer:

- explicit upstream cancellation/status changes are authoritative;
- disappearance alone is not equivalent to cancellation;
- upstream identity/history should remain inspectable;
- future snapshot/history support may make supersession explicit.

## 15. Runtime views are consumer-owned

Taria materializes canonical bundles and durable derivatives.

Ephemeris is free to construct runtime views such as:

- US politics excluding hearings over the next 45 days;
- California courts plus federal elections;
- high-importance economic releases this month;
- finance events overlaid with government meetings;
- sports events grouped by league and colored by status.

Those operations do not require Resourcearium to rebuild a bundle.

They are saved/query views over imported canonical event state.

## 16. Rendered formats are not the interchange authority

ICS, JSCalendar, jCal, CSV, Google Calendar, and similar outputs remain downstream projections.

Ephemeris must prefer rich Resourcearium JSON/state over those flattened outputs.

In particular:

```text
ICS file != canonical Taria event corpus
Google calendar container != canonical bundle
Ephemeris saved view != canonical bundle
```

## 17. Current Taria production pipeline

The current first production path is:

```text
429 canonical temporal Resources
    -> pinned RICS acquisition tranche(s)
    -> resilient acquisition
    -> NormalizedEventSnapshot
    -> 13 canonical projection builds
    -> reconciliation
    -> CalendarSets
    -> TemporalBundleRelease
```

The first exact-lineage tranche contains 177 direct single-Resource RICS profiles.

Other composite/family-mediated/surface/local/non-RICS inputs can be layered into later releases without changing the Ephemeris consumer contract.

## 18. Current bootstrap release

The current bootstrap channel points to:

```text
temporal-bundle-release:bootstrap:2026-10-04:r4
```

It is:

- `bootstrap-partial`;
- consumer-safe for integration testing;
- **1,114 ready events**;
- 15 represented canonical Resource identities;
- 2 typed recovered ingestion profiles;
- 196 recovered source surfaces;
- **3** partially populated canonical bundle slots;
- **9** pending canonical bundle slots;
- explicitly not production-complete.

Current populated shards:

- U.S. Politics recovery: 1,038 ready events;
- 2027 European national elections: 11 ready events;
- 2026 U.S. pro sports excluding hockey: 37 ready events;
- U.S. Holidays recovery: 28 ready events.

Current domain totals:

- Politics & Government: 1,049;
- Sports & Competition: 37;
- Holidays & Observances: 28.

The recovered Politics/Holidays shards are downstream-derived recovered state, not fresh upstream publisher observations, and Ephemeris must preserve that data posture.

Ephemeris must preserve the release's partial posture if/when it imports it.

## 19. Current Ephemeris release-consumer implementation

The filesystem-first release consumer is now implemented through the core identity/membership layer.

Implemented:

1. persisted Resourcearium root path and release channel;
2. auto-detection of local/sibling Taria checkouts, including `$HOME/Code/Text/taria`;
3. local release-registry and immutable manifest resolution;
4. path confinement under Resourcearium root;
5. bootstrap content-fingerprint validation;
6. production file-SHA-256 validation;
7. direct `ReconciledProjectionEventSet` import;
8. direct `CompactReconciledEventIndex` import;
9. immutable release metadata persistence;
10. immutable CalendarSet persistence;
11. release-to-CalendarSet association;
12. projected-calendar persistence;
13. CalendarSet event-membership persistence;
14. many source/import-record mappings per canonical event;
15. many upstream event/reconciled identity aliases per canonical event;
16. cross-bundle deduplication for overlapping production projections;
17. bootstrap `shards[]` adoption;
18. production `bundle_artifacts[]` adoption;
19. one-click **Update Taria Sources** UI;
20. persisted last release/update summary.

The overlap invariant is regression-tested: a shared Politics/Finance event remains one canonical local event with two CalendarSet memberships.

The current bootstrap r4 release is fully consumable for all populated shards.

The live production channel remains unset, so production adoption is implemented and tested synthetically but awaits a real production release.

Still required:

- release coverage/pending/gap UI;
- whole-release atomic rollback across multiple payload imports;
- asynchronous/background adoption for large releases;
- release-history/diff inspection;
- query/view predicates over imported bundle/CalendarSet membership.
