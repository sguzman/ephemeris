# Taria Integration

## Relationship

Taria and Ephemeris solve different problems.

- **Taria** owns knowledge acquisition, source/resource curation, ontology, provenance, projections, and temporal-resource generation.
- **Ephemeris** owns interactive local temporal exploration, visualization, querying, inspection, annotations, and personal calendar behavior.

Ephemeris should consume rich Taria temporal resources without forcing Taria to imitate a desktop calendar database.

## Native interchange goal

The preferred long-term integration is a versioned Taria temporal interchange format.

It should carry enough information to preserve:

- stable event identifiers
- stable source identifiers
- normalized title
- raw/source title
- descriptions
- event type
- domain
- geography
- jurisdiction
- institution
- participants
- lifecycle status
- confidence
- importance/relevance when appropriate
- canonical instant/local time/all-day semantics
- source timezone
- recurrence
- relations
- collections/sequences
- source authority and source kind
- provenance
- snapshot membership
- source record identifiers
- extensible properties

## What must not happen

Do not reduce Taria-native data to:

```text
title
start
end
color
```

before Ephemeris sees it.

ICS may be an excellent external interchange format for conventional calendar clients, but it is not the canonical Taria → Ephemeris data contract.

## Import ownership

Taria-supplied events are normally source-owned or projection-owned.

Ephemeris may attach user-owned annotations without mutating the original Taria assertion.

Examples:

- watched
- personal relevance
- notes
- reminder rules
- local tags
- suppression
- custom display classification

These must survive a Taria refresh.

## Projection identity

Taria projections should provide stable keys so refresh can distinguish:

- unchanged record
- changed record
- moved event
- deleted/superseded source record
- newly added record

Where stable source identity is impossible, the adapter should expose confidence and matching diagnostics rather than pretend identity is certain.

## Snapshot integration

If Taria provides snapshot identity, Ephemeris should preserve it.

That enables:

- as-of views
- diff between snapshots
- source change history
- event lifecycle reconstruction

## Resourcearium/source metadata

Temporal resources may include source-health information such as:

- last successful retrieval
- expected refresh cadence
- endpoint stability
- rollover behavior
- source authority
- native machine-readable vs generated projection

Ephemeris should be designed to surface this metadata rather than flatten it away.

## Rollover

Some temporal sources are continuous; others are year/edition specific.

Taria may own discovery/rebuilding of replacement sources.

Ephemeris should be able to display source state such as:

- current
- stale
- rollover required
- replacement discovered
- projection rebuild required
- failed

## Local handoff

The first integration should prefer a deterministic local handoff over live service coupling.

Potential forms:

- versioned JSON bundle
- SQLite export/import
- structured directory snapshot
- another explicit versioned artifact

The first implementation should prioritize:

- inspectability
- deterministic tests
- offline operation
- stable IDs
- schema versioning

Live IPC/service integration can be added later if it solves a real workflow problem.

## Contract versioning

Every native interchange artifact should declare:

- schema version
- producer version where useful
- generation timestamp
- snapshot identity where applicable

Unknown mandatory fields or incompatible versions should fail loudly rather than silently discard semantics.

## Provenance display

For any Taria event, the inspector should eventually be able to answer:

- Where did this come from?
- Who published it?
- Was the source official?
- Was it a native calendar or generated projection?
- When was it acquired?
- Which snapshot contains it?
- What normalization transformed it?
- What source record supports it?
