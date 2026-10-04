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
5. the parent directory when it is itself the Taria repo.

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
    -> verify declared hashes
    -> import supported post-reconciliation payloads
    -> reconcile into Ephemeris SQLite
    -> reload the current view
```

There is no network step.

## Current local release support

The filesystem updater currently supports the `bootstrap-partial` `shards[]` packaging path.

Accepted shard payloads:

### CompactReconciledEventIndex

If a shard exposes:

- `event_index_path`
- `event_index_content_sha256`

Ephemeris:

1. resolves the path relative to Resourcearium root;
2. rejects paths that escape the Resourcearium tree;
3. verifies SHA-256;
4. verifies object kind `CompactReconciledEventIndex`;
5. imports its post-reconciliation events;
6. retains stable reconciled/event/assertion/provenance identity;
7. reconciles repeat imports against the existing source.

This currently makes the recovered U.S. Politics and U.S. Holidays bootstrap-r3 shards directly consumable from disk.

### ReconciledProjectionEventSet

The updater also recognizes a bootstrap shard that exposes a reconciled-event-set path and declared hash.

The existing direct Taria reconciled-event-set adapter is reused.

## Unsupported populated shards

A populated release shard that does not expose an accepted post-reconciliation payload is **not silently ignored**.

It is reported as skipped.

Ephemeris does not:

- reconstruct Resourcearium reconciliation from normalized snapshots;
- invent event payload from CalendarSet membership;
- fall back to ICS;
- pretend a skipped shard was successfully adopted.

For bootstrap r3, the Elections and Sports specimen shards are currently reported this way until Resourcearium exposes their rich reconciled payloads.

## Production releases

Production `bundle_artifacts[]` are recognized by the local release resolver.

They are deliberately not imported by this first filesystem-update slice yet.

Reason:

canonical domain bundles overlap. Importing each production projection independently through the current source-scoped adapter could duplicate a canonical event across several bundles.

Production adoption therefore waits for the release-level cross-bundle identity/membership layer.

The button reports this limitation instead of creating duplicate data.

## Integrity

Every payload consumed through the release updater must have a declared hash.

Current behavior:

- missing payload file -> update fails;
- path escape -> update fails;
- missing declared hash -> update fails;
- hash mismatch -> update fails;
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

The Sources panel displays these values.

## Network behavior

Network-based release discovery may be added later as a convenience.

It is not required by the architecture.

Priority order is:

1. local filesystem Resourcearium root;
2. explicit local release/artifact testing;
3. optional future network/GitHub discovery.

A network outage must not prevent Ephemeris from reading already-present Taria artifacts or using its existing SQLite corpus.

## Next integration steps

The filesystem transport problem is now solved at the baseline level.

The next release-adoption work is semantic rather than transport-related:

1. persist release metadata canonically in SQLite;
2. persist CalendarSet membership independently from event identity;
3. implement cross-bundle event deduplication for production `bundle_artifacts[]`;
4. adopt production partial/complete releases through the same button;
5. expose release coverage/pending/gap posture in the UI;
6. eventually make the explicit update operation asynchronous so large releases never stall the frame loop.
