# User Experience Contract

## Goal

Ephemeris must make a large temporal database feel like a calendar rather than a database administration tool.

Power should be progressively exposed.

## Primary surfaces

Implemented date ranges:

- year
- quarter
- month
- week
- day

Implemented layouts:

- Grid
- Agenda
- Compact Agenda
- chronological Stream
- proportional Timeline
- Density / heatmap
- Summary / pivot foundation
- dense Table

Long-term additional layout variants may include:

- multi-day specialized surfaces
- richer continuous/vertical timelines
- interval/Gantt-like views where appropriate

Date range and layout are independent. All layouts consume the same canonical query result rather than defining separate event containers.

Agenda, Compact Agenda, and Table honor configured grouping/sort presentation. Stream intentionally does not: Stream means canonical temporal sequence. Timeline likewise owns temporal placement across the active date window. Both preserve saved grouping/sort settings for later reuse.

Density is aggregate: only events that genuinely occur on a day contribute to that day's intensity, while coarse-precision records remain explicitly unassigned to fake days. Summary is also aggregate and reuses GroupBy as its pivot dimension. Density/Summary preserve but do not apply per-event color rules.

## Dense-day behavior

A day with 100 events is valid.

Views should support:

- compact event rows
- collapsed groups
- event counts
- aggregation markers
- overflow indicators
- zoom-dependent detail
- density modes
- drill-down
- table/agenda fallback

Never allow a dense day to make navigation unusable.

## Local quick-create

Quick-create exposes common event fields directly: title, description, classification, lifecycle, Busy/Free behavior, timed or multi-day all-day placement, and optional participant names, venue, address, and virtual meeting URL.

Participant names are local structured event metadata, not an invitation or outbound message. Richer participant roles and entity resolution remain available in the inspector after creation. Choosing a suggested alternate slot must not erase participant or location fields.

The advanced possible-start window is distinct from event duration and from the representative start. All-day windows accept civil dates; exact-time windows accept source-local date-times, rejecting DST-ambiguous or nonexistent bounds. The representative start must lie within a nonzero window. An active Busy event with bounded uncertainty cannot be certified conflict-free from the representative time and requires explicit second-Save acknowledgment; editing the bounds invalidates that acknowledgment. Free or nonblocking lifecycle states do not manufacture Busy commitments.

## Event inspector

Selecting an event should eventually expose:

- normalized title
- raw title
- time
- original timezone
- display timezone
- all-day/floating semantics
- lifecycle status
- event type
- domain/categories
- geography/jurisdiction
- institution
- participants
- location
- source
- authority/source kind
- canonical URL
- acquisition time
- confidence
- identifiers
- recurrence information
- relations
- collections
- snapshot/provenance
- custom properties
- user annotations
- transformation history where available

## Source inspector

A source should expose:

- publisher
- authority classification
- native/generated distinction
- format
- locators
- refresh state
- freshness expectations
- failure state
- snapshot history
- rollover state

## Filtering

Ordinary filters should be discoverable through facets.

Advanced filters can use a query builder or expression language.

Changing filters should be immediate and should not modify canonical organization.

## Saved views

Implemented actions:

- create from current state
- apply
- update from current state
- delete
- return to later

Implemented saved state includes query, source visibility, date range, layout, grouping, sorting, color rules, overlays, timezone, and week-start behavior.

Still useful future actions include:

- rename
- clone
- compare
- export
- inheritance/composition

The product must make clear that a saved view is not a duplicated event container.

## Color configuration

Changing color logic is a view-level operation.

Implemented semantic fallback strategies:

- by source
- by domain
- by jurisdiction
- by institution
- by event type
- by status
- no semantic fallback

Implemented custom color rules:

- full recursive query condition
- explicit RGB color
- first-enabled-match precedence
- add/edit/delete
- enable/disable
- reorder
- persistence in saved views
- overlay-specific rule sets

Color remains presentation; it does not imply event ownership or membership.

## Overlays

Overlays are now an implemented view-level surface.

An overlay can:

- define its own query;
- contribute matching canonical events to the visible result;
- carry its own semantic fallback coloring;
- carry its own ordered color rules;
- be enabled/disabled;
- be renamed;
- be reordered for styling precedence;
- be deleted.

Overlay union does not copy events.

Calendar algebra is implemented separately from overlays: ordered composition layers support union, intersection, and subtraction over embedded queries or cycle-safe SavedView references. Overlays remain the independently styled union surface applied afterward.

## Keyboard interaction

Implemented direct shortcuts avoid text fields and currently include:

- Left / Right: previous / next period
- T: today
- Y / Q / M / W / D: Year / Quarter / Month / Week / Day
- G / A / C / S / L / H / P: Grid / Agenda / Compact Agenda / Stream / Timeline / Density / Summary

Target operations:

- command palette
- fuzzy search
- switch view
- date jump
- previous/next period
- today
- move selection
- open inspector
- close inspector
- toggle facets
- save current view
- select all/current group
- bulk actions

Shortcuts must avoid interfering with text editing.

## Bulk operations

Users should eventually be able to select large result sets and:

- add/remove local tags
- change local classification
- annotate
- suppress from a view
- export
- compare
- inspect sources
- add/remove watch/reminder rules

Source-owned canonical assertions should require explicit ownership-aware operations.

## Status and background work

Never hide significant work.

The UI should show:

- loading
- importing
- parsing
- reconciling
- indexing
- refreshing
- stale
- failed
- completed

Terminal/log diagnostics are useful during development, but critical status must also be visible in-app.

## Customization

Long-term view customization may include:

- event height
- row density
- inline fields
- font size
- time format
- weekend visibility
- week start
- working hours
- separators
- group headers
- color rules
- iconography
- opacity
- cancelled-event treatment
- custom label templates

## Accessibility and legibility

Dense does not mean illegible.

Avoid making color the only carrier of meaning for lifecycle/status.

Keyboard focus should remain visible.

Text truncation should offer inspection rather than silently hiding important information.
