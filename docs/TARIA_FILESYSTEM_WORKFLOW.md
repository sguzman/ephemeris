# Local Taria Filesystem Workflow

Status: implemented baseline, 2026-10-04.

## Principle

Ephemeris consumes Taria / Resourcearium **directly from the local filesystem**.

The normal workflow does not require:

- copying Taria artifacts into the Ephemeris repository;
- downloading release files from GitHub;
- using the GitHub API;
- HTTP access;
- exporting/importing ICS as an interchange workaround.

The local Taria repository remains the upstream artifact store.

Ephemeris keeps its own SQLite database as the fast interactive local representation.

## Normal layout

A typical checkout may look like:

```text
~/src/
  taria/
    incubator/
      resourcearium/
        registry/
        derived/
        examples/
        ...

  ephemeris/
```

Ephemeris may be pointed at either:

```text
~/src/taria
```

or directly at:

```text
~/src/taria/incubator/resourcearium
```

It normalizes both forms to the Resourcearium root.

Ephemeris also accepts the release-registry file itself:

```text
.../resourcearium/registry/temporal-bundle-releases.yml
```

## Persisted setting

Ephemeris stores one local path:

```text
Taria Resourcearium root
```

The setting lives in Ephemeris UI state. It is machine-local configuration, not canonical temporal data and not a path that should be committed into Taria or Ephemeris source control.

The configured release channel is also persisted.

Current channels:

- `bootstrap`
- `production`

## Auto-detection

On startup, if no path has been configured, Ephemeris attempts local detection.

Detection checks:

1. `TARIA_RESOURCEARIUM_ROOT` environment variable;
2. the current working directory;
3. a sibling `../taria` checkout;
4. a child `./taria` checkout;
5. the parent directory when it is itself the Taria repo;
6. `$HOME/Code/Text/taria`.

If a valid Resourcearium release registry is found, Ephemeris stores the normalized canonical path automatically.

If detection fails, the path can be entered once in the Sources panel.

## One-click update

The primary operator action is:

> **Update Taria Sources**

It is available in:

- the main toolbar;
- the Taria section of the Sources panel.

The update does this:

```text
configured local Resourcearium root
    -> registry/temporal-bundle-releases.yml
    -> selected channel
    -> current immutable release manifest
    -> resolve referenced local artifacts
    -> verify declared integrity metadata
    -> import post-reconciliation payloads
    -> reconcile canonical event identity globally
    -> persist CalendarSets/memberships/releases
    -> Ephemeris SQLite
    -> reload the current view
```

There is no network step.

## Current local release support

The filesystem updater supports both release-v1 packaging paths:

- `bootstrap-partial -> shards[]`
- `production-partial / production-complete -> bundle_artifacts[]`

Accepted shard payloads:

### CompactReconciledEventIndex

If a shard exposes:

- `event_index_path`
- `event_index_content_sha256`

Ephemeris:

1. resolves the path relative to Resourcearium root;
2. rejects paths that escape the Resourcearium tree;
3. verifies the Resourcearium-declared `content_fingerprint.value`;
4. verifies object kind `CompactReconciledEventIndex`;
5. imports its post-reconciliation events;
6. retains stable reconciled/event/assertion/provenance identity;
7. reconciles repeat imports against the existing source.

This consumes recovered and frozen-rebuild bootstrap-r10 shards directly from disk, including Politics, Holidays, Economics, Finance, Business, Culture, and Education payloads.

### ReconciledProjectionEventSet

The updater also recognizes a bootstrap shard that exposes a reconciled-event-set path and declared content fingerprint.

The existing direct Taria reconciled-event-set adapter is reused.

Bootstrap r10 uses this path for:

- 2027 European national elections;
- 2026 U.S. pro sports excluding hockey.

Therefore **all four populated r4 bootstrap shards are now directly consumable**.

## Unsupported populated shards

A populated release shard/artifact that does not expose an accepted post-reconciliation payload is **not silently ignored**.

It is reported as skipped or rejected according to the release contract.

Ephemeris does not:

- reconstruct Resourcearium reconciliation from normalized snapshots;
- invent event payload from CalendarSet membership;
- fall back to ICS;
- pretend a missing payload was successfully adopted.

Bootstrap r10 exposes accepted rich payloads for every populated shard, so the current bootstrap channel has no payload-resolution gap. Explicit gap-only shards are preserved as coverage state.

## Production releases

Production `bundle_artifacts[]` are now supported by the local release resolver.

Production integrity differs from bootstrap:

- reconciled event set -> raw file SHA-256;
- CalendarSet -> raw file SHA-256.

Overlapping domain bundles are safe because Ephemeris now retains many upstream identity aliases per canonical event and many source/import-record mappings per canonical event.

Conceptually:

```text
Politics projection ----\
Economics projection ----> one local TemporalEvent
Finance projection ------/          |
                                    +-> several CalendarSet memberships
```

A regression test covers a shared event appearing in Politics and Finance: it remains one canonical local event with two memberships.

The live Resourcearium `production` channel is currently unset, so production adoption is implemented/tested but not yet exercised against a real production release.

## Integrity

Every payload consumed through the release updater must have declared integrity metadata. Bootstrap and production intentionally use different integrity conventions.

Current behavior:

- missing payload file -> update fails;
- path escape -> update fails;
- missing declared integrity metadata -> update fails;
- bootstrap content-fingerprint mismatch -> update fails;
- production file-SHA mismatch -> update fails;
- unsupported payload kind -> update fails or is reported unsupported according to release position;
- supported populated shard without payload -> shard is explicitly reported skipped.

The importer never needs to trust GitHub transport because it validates the files already on disk.

## Local SQLite remains authoritative for interaction

Ephemeris does **not** query Taria JSON on every calendar interaction.

The ownership boundary is:

```text
Taria filesystem
    immutable/versioned upstream artifacts

        local filesystem adoption

Ephemeris SQLite
    fast local working corpus
    queries
    views
    overlays
    presentation
    annotations
```

This keeps ordinary calendar interaction independent from:

- Git;
- GitHub;
- network state;
- Taria's current working tree operations;
- Resourcearium acquisition jobs.

## Updating Taria

When Taria produces a newer release, the operator workflow is intentionally boring:

1. update/pull/materialize Taria however Taria itself is maintained;
2. open Ephemeris;
3. press **Update Taria Sources**.

Ephemeris follows the local channel pointer and adopts the current local release artifacts it supports.

No manual selection of individual JSON files should be necessary for the normal workflow.

Direct drag/drop remains useful for debugging and one-off artifact testing.

## Persistent status

Ephemeris remembers:

- Resourcearium root;
- selected release channel;
- last successfully processed release ID;
- last update timestamp;
- last update summary.

The Sources panel displays these values plus adopted release status, completeness, generated/adopted timestamps, coverage counts, and per-bundle partial/pending/gap-only posture.

Bundle-membership and projected-calendar predicates also use release-backed selectors populated from the adopted SQLite state, so normal query editing remains offline after adoption.

## Network behavior

Network-based release discovery may be added later as a convenience.

It is not required by the architecture.

Priority order is:

1. local filesystem Resourcearium root;
2. explicit local release/artifact testing;
3. optional future network/GitHub discovery.

A network outage must not prevent Ephemeris from reading already-present Taria artifacts or using its existing SQLite corpus.

## Next integration steps

The filesystem transport and core release-adoption semantics are now implemented.

Whole-release adoption is now atomic:

- release metadata is written inside the same outer SQLite transaction as event payloads;
- payload adapters and CalendarSet import join that transaction;
- upstream identity aliases and memberships join it too;
- a failure in any later artifact rolls back every earlier mutation from the release.

The next work is:

1. make the explicit update operation asynchronous so large releases never stall the frame loop;
2. add release-to-release diff/history inspection.
