# Performance and Scale

## Principle

Near-zero interaction latency is a product requirement, not polish.

Ephemeris exists partly because a native local application should be able to interrogate temporal data without cloud round trips or WebView overhead.

## Target corpus

Design for growth from:

- thousands of events
- tens of thousands
- hundreds of thousands

The architecture should not assume only a personal calendar-sized dataset.

## Interaction targets

These are initial engineering budgets rather than promises.

On a representative desktop and warm local data:

- simple filter/facet change: perceptually immediate, target < 50 ms computation
- saved-view switch: target < 100 ms before useful content appears
- date navigation: target < 50 ms query/update for ordinary windows
- event inspector open: target < 50 ms when data is local
- text search feedback: incremental, target < 100 ms for indexed queries
- UI frame work: avoid blocking operations that cause visible multi-frame stalls

Large imports/refreshes are allowed to take longer but must run outside the rendering path.

## Cold start

Cold start should avoid:

- network dependency
- scanning entire raw corpora when indexed state exists
- synchronous database migrations without visible status
- eager recurrence expansion over unbounded horizons

## Query indexes

Indexes should reflect actual common predicates:

- time ranges
- event/source IDs
- source
- status
- domain
- jurisdiction
- institution
- event type
- tags
- snapshot
- full-text fields

Do not index every custom field blindly; measure query patterns.

## Recurrence

Expand recurrence only for required horizons.

Cache/materialize selectively if measurements justify it.

## Rendering

Calendar rendering should degrade gracefully with density.

Use:

- aggregation
- clipping
- virtualization where appropriate
- bounded visible ranges
- cached layout data where useful

Do not construct thousands of expensive widgets when a compact aggregate would answer the view.

## Background work

Never perform in the egui frame loop:

- network retrieval
- large file parsing
- large database imports
- expensive duplicate detection
- index rebuilds
- snapshot diffs over large corpora

Use worker/task boundaries and send compact state changes to the UI.

## Measurement

Performance work requires instrumentation.

Track at least:

- query duration
- render/update duration where practical
- import parse time
- reconciliation time
- index time
- source refresh duration
- event counts returned/rendered

Avoid premature micro-optimization without measurements, but treat user-visible latency regressions as defects.

## Memory

Prefer indexed local storage and bounded query result sets over loading every event and raw payload into memory.

Caches must have explicit invalidation and bounded growth.

## Network

Rendering/querying existing local data must not wait on network availability.

Refresh failures should leave the previous local snapshot usable.
