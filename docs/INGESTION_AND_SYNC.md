# Ingestion, Refresh, Synchronization, and Export

## Principle

Import should be promiscuous; canonical storage should be normalized.

Source formats are adapters, not the ontology.

For Taria specifically, Ephemeris is a **release consumer**, not a source-acquisition engine.

## Input classes

Long-term inputs may include:

- Taria TemporalBundleRelease packages
- direct Taria ReconciledProjectionEventSet artifacts
- ICS
- webcal/webcals
- CalDAV
- JSCalendar
- jCal
- JSON
- CSV
- APIs
- manual local events

## Two ingestion classes

Ephemeris distinguishes two broad classes.

### 1. Upstream-normalized Taria products

Taria has already performed acquisition, normalization, identity reasoning, projection, and reconciliation.

Ephemeris should not repeat that work.

Preferred flow:

```text
TemporalBundleRelease
    -> validate manifest + hashes
    -> resolve ReconciledProjectionEventSet or accepted CompactReconciledEventIndex payloads
    -> reconcile/upsert local canonical events
    -> import CalendarSet memberships
    -> import coverage/release metadata
    -> transact
    -> index
```

The existing direct reconciled-event-set importer and CompactReconciledEventIndex importer are low-level payload adapters underneath the implemented local release updater.

### 2. Non-Taria source formats

For ICS, CalDAV, CSV, generic JSON, APIs, and locally authored events, Ephemeris owns more of the adapter pipeline:

```text
acquire/read
    -> parse
    -> validate
    -> normalize
    -> identify/match
    -> reconcile
    -> transact
    -> index
    -> record provenance/snapshot
```

Local ICS is the first implemented source in this class. The current adapter parses a strict supported VCALENDAR/VEVENT subset, projects UID groups into canonical events, creates a first-class `SourceKind::Ics` source, and imports through `TemporalStore::import_batch`.

For local files, the canonicalized filesystem path is the stable source identity and VEVENT UID is the stable source-record identity. Refresh is transactional: unchanged records remain unchanged, changed UIDs update in place, new UIDs are created, and UIDs absent from a later file snapshot are retained unless stronger source semantics justify deletion or cancellation. Invalid calendars fail before source creation.

ICS import is available from `ephemeris-import` and GUI drag/drop. The canonical RFC exporter is implemented; the remaining user-facing export work is selecting stored events/sources and writing the resulting VCALENDAR payload to disk.

Each adapter stage should provide diagnostics.

## Taria release adoption

The authoritative contract is:

- [TARIA_BUNDLE_CONTRACT.md](TARIA_BUNDLE_CONTRACT.md)
- [TARIA_FILESYSTEM_WORKFLOW.md](TARIA_FILESYSTEM_WORKFLOW.md)

A Taria release adoption is not equivalent to individually refreshing 429 upstream sources.

Taria owns those upstream sources.

Ephemeris adopts a frozen release directly from the local Resourcearium filesystem.

### Required release behavior

Ephemeris now does most of this baseline contract. It must continue to:

- validate release schema/version;
- validate release ID and status;
- validate canonical bundle slots;
- verify referenced hashes/fingerprints;
- reject missing required artifacts;
- preserve partial/pending/gap-only posture;
- resolve rich event payloads;
- deduplicate overlapping bundle membership by stable upstream event identity;
- persist CalendarSet membership separately from event identity;
- preserve local annotations;
- record release metadata;
- commit the entire multi-artifact release atomically.

Whole-release atomicity is implemented. Release metadata, source/event imports, upstream identity aliases, release/source links, canonical event snapshots, CalendarSets, projected calendars, and memberships participate in one adoption transaction. A failure in any later artifact rolls back all mutations from that release.

### Accepted release states

Ephemeris may consume:

- `bootstrap-partial`
- `production-partial`
- `production-complete`

A superseded release may remain manually reproducible but should not be preferred automatically.

### Partial releases

A partial release is useful.

Do not interpret:

- pending
- gap-only
- selected-but-uningested

as "there are no events."

Coverage state is data and must survive import.

## Event identity during bundle adoption

Canonical domain bundles overlap.

Ephemeris must not duplicate events merely because several Taria bundles include the same reconciled event.

The intended mapping is:

```text
upstream reconciled/event identity
    -> one local TemporalEvent

CalendarSet / bundle membership
    -> separate local membership rows/metadata
```

Identity matching should prefer:

- stable Taria reconciled/event identity;
- source-record identity;
- source-native UID where retained;
- occurrence/series identity when available;
- canonical publisher identity.

Weak heuristics should produce diagnostics rather than irreversible silent merges.

## Current Taria payload boundary

The current direct importer accepts ReconciledProjectionEventSet JSON.

That payload contains the rich resolved event state Ephemeris needs.

Bootstrap r4 pins CompactReconciledEventIndex objects for recovered Politics/Holidays and full ReconciledProjectionEventSets for Elections/Sports. Both payload classes are implemented adapters.

CalendarSet alone is not a complete event payload; it references reconciled events and carries membership/navigation metadata.

The release importer therefore needs both:

- event payload;
- CalendarSet membership.

## Current release-v1 variants

Taria currently validates:

### Bootstrap

`shards[]`

Bootstrap r4 is fully payload-resolvable:

- recovered Politics/Holidays shards provide CompactReconciledEventIndex paths/content fingerprints;
- European Elections/Sports shards provide ReconciledProjectionEventSet paths/content fingerprints.

The one-click updater can therefore consume all populated bootstrap-r4 shards from the local filesystem.

### Production

`bundle_artifacts[]`

Production artifacts expose:

- ProjectionEventSet path/hash;
- ReconciledProjectionEventSet path/hash;
- CalendarSet path/hash;
- rendered JSON directory;
- rendered ICS directory.

This directly matches the rich payload adapter Ephemeris already has. Production bundle adoption is implemented with global upstream identity aliases so overlapping projections converge on one canonical event while their CalendarSet memberships remain separate.

## Source definitions

For non-Taria external sources, a source definition should describe:

- identity
- locator(s)
- source kind
- format
- publisher/authority
- refresh policy
- authentication reference if required
- read-only/write ownership
- expected freshness
- rollover behavior
- adapter configuration

For Taria, the release manifest/channel is the upstream boundary; Ephemeris does not need one local source definition per Resourcearium Resource.

## Refresh semantics

A refresh/adoption should be repeatable and identity-aware.

It should report:

- records/events observed
- events created
- events updated
- events unchanged
- events moved/rescheduled
- events explicitly cancelled
- retained records missing from the newer payload
- conflicts
- parse/validation failures
- identity ambiguities
- bundle-slot coverage changes
- release identity/status

## Deletion semantics

A source record disappearing does not always mean the real-world event should be erased.

Current conservative policy:

- disappeared upstream record -> retain canonical event unless stronger semantics say otherwise
- explicit cancellation -> update lifecycle to cancelled
- explicit supersession -> preserve history/identity and mark supersession when supported
- projection rebuild -> adjust membership without cloning/deleting canonical identity

Adapters/source classes may require different policy.

## Release history

A newer Taria release is a new frozen upstream state.

Ephemeris now retains immutable release identity, channel, manifest hash/path, generated-at time, import/adoption time, production-complete flag, raw manifest, and coverage JSON.

CalendarSets are stored as immutable objects with separate release associations.

Implemented release history now includes:

- release-to-source projection associations;
- immutable per-release canonical event snapshots;
- source/bundle/projected-calendar/member-event add/remove diffs;
- canonical event add/remove, rename, temporal-move, lifecycle-status, and newly-cancelled diffs;
- expandable before/after canonical event change details.

Still future:

- broader normalized provenance/artifact-lineage tables beyond the current Taria release snapshot model;
- explicit persisted prior/next release links.

## Raw source retention

Depending on format and size, retain:

- raw payload
- content hash
- source-record excerpt
- raw field map
- or a stable reference to an external frozen artifact.

For Taria releases, immutable upstream artifact refs/hashes may be sufficient without copying every raw acquisition payload into Ephemeris.

## Filesystem-first refresh

Ephemeris must remain local-first.

The implemented default path is:

```text
local Taria checkout
    -> local Resourcearium release registry
    -> local release manifest
    -> local payload files
    -> Ephemeris SQLite
```

Normal updates require no download/fetch step.

The application persists:

- local Resourcearium root;
- selected release channel;
- last processed release ID;
- last update timestamp;
- last update summary.

The toolbar/Sources-panel **Update Taria Sources** action follows the local channel pointer and processes supported artifacts.

Future network-assisted release discovery may be added as a convenience only. It must never be required for ordinary operation.

Rendering/querying never requires live Taria/GitHub access.

The filesystem update action runs on a background worker with its own SQLite connection. Release adoption remains atomic on that worker connection, while egui polls completion without blocking the frame loop.

## Source health

Schema v13 persists generic refresh-attempt records independently from release adoption so failed attempts survive release rollback. Records retain refresh kind/target, start/completion timestamps, success/failure, adopted release ID when available, summary, and error. An app interruption can therefore leave an explicit incomplete attempt.

The Taria UI distinguishes:

- never refreshed;
- running;
- healthy;
- stale;
- failed;
- interrupted;
- unknown/malformed history.

Current local Taria policy marks the refresh posture stale after **7 days without a successful refresh**. The threshold is displayed in the UI rather than hidden in implementation details. An upgraded pre-v13 database that already has an adopted release but no attempt history is treated as unknown/legacy history, not falsely as never refreshed.

This health state describes Ephemeris's local release-consumption freshness. It does not replace Resourcearium/Taria's own upstream acquisition diagnostics or bundle coverage posture.

For future directly managed non-Taria sources, the same generic attempt-history foundation can be reused, with source-class-specific freshness/HTTP/cache policy where appropriate.

## External synchronization

External calendar services are optional integration surfaces.

If CalDAV/Google-style synchronization is added, define:

- source authority
- local authority
- read-only/read-write mode
- conflict policy
- UID mapping
- sync token/ETag behavior
- deletion policy
- attendee/invitation ownership

The external service must not become the canonical representation of Taria's richer event model.

## Export

Export is a projection.

Examples:

- export a saved view to ICS
- export query results to CSV
- export selected events to JSON
- generate a mobile-compatible calendar feed

Export does not require moving events into a dedicated storage container.

## Personal events

Locally authored personal events may have Ephemeris as their authoritative source.

That ownership should be explicit and separate from read-only external or Taria-owned events.
