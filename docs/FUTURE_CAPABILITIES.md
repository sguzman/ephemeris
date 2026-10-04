# Long-Horizon Capability Ledger

This document preserves the full intended capability envelope of Ephemeris.

Items here are not all immediate roadmap commitments. They are recorded so short-term implementation does not accidentally make the long-term model impossible.

## Canonical corpus and programmable views

- one canonical event corpus
- unlimited saved views over that corpus
- no required event duplication for conceptual calendars
- visibility independent from organization
- color independent from ownership/source membership
- independent filtering, grouping, sorting, coloring, and layout
- view cloning
- view composition/inheritance
- calendar algebra: union, intersection, subtraction

## Querying and search

- arbitrary boolean queries
- ranges
- sets
- null checks
- text predicates
- nested expressions
- relative dates
- derived fields
- facets for ordinary users
- full-text search
- alias-aware/entity-aware search
- search results convertible to saved views

## Overlays and comparison

- multiple simultaneous independently configured overlays
- current vs historical snapshot
- tentative vs confirmed
- projected vs observed
- personal vs institutional/public data

## Visualizations

- year
- quarter
- month
- week
- multi-day
- day
- agenda
- compact agenda
- chronological stream
- continuous timeline
- vertical timeline
- table
- grouped list
- density/heatmap
- interval/Gantt-like visualization where useful
- pivot/summary views
- charts/histograms built over query results

## Dense-data behavior

- tiny/compact rows
- collapsed groups
- event counts
- aggregation markers
- overflow handling
- zoom-dependent detail
- configurable density
- high-count day drill-down

## Event inspection

- normalized title
- raw source title
- event type
- categories/domain
- geography
- jurisdiction
- institution
- participants
- source
- canonical/source URLs
- acquisition date
- confidence/status
- tags
- recurrence
- identifiers
- source and display timezone
- provenance chain
- snapshot membership
- transformation history
- custom properties
- user annotations

## Provenance and sources

- native official calendar vs generated projection distinction
- source authority
- source kind
- source convenience vs authority vs completeness distinction
- raw/source record retention
- refresh history
- source health
- endpoint/cache metadata
- staleness
- rollover status

## Event lifecycle and uncertainty

- announced
- tentative
- scheduled
- confirmed
- rescheduled
- postponed
- cancelled
- completed
- observed
- superseded
- estimated
- projected
- disputed
- lineage across reschedules/supersession
- confidence and uncertainty

## History and diffs

- snapshot history
- as-of views
- events added
- events removed
- date/time changes
- cancellation changes
- source-record changes
- diff between arbitrary snapshots

## Hierarchy, ontology, and tags

- CalendarSet-like hierarchy for navigation
- lateral queries across hierarchy
- stable typed fields
- ad hoc tags
- extensible custom properties
- schema evolution without forcing every concept into migrations

## Recurrence

- RRULE
- RDATE
- EXDATE
- exceptions
- moved individual occurrences
- cancelled occurrences
- recurring-series editing
- all-day recurrence
- floating-time recurrence
- DST correctness
- stable occurrence identity

## Timezones

- source timezone preservation
- canonical instant where applicable
- display timezone
- view-specific timezone override
- all-day date correctness
- floating times
- DST transitions
- cross-midnight correctness

## Import formats

- Taria native interchange
- ICS
- webcal/webcals
- CalDAV
- JSCalendar
- jCal
- CSV
- JSON
- APIs
- generated projections
- manual local entry

## Export and projection

- saved view -> ICS
- saved view -> CSV
- saved view -> JSON
- selected events -> projection
- external/mobile feed generation
- export must never require canonical event reorganization

## External synchronization

- optional CalDAV/cloud sync
- explicit ownership rules
- read-only vs read-write distinction
- conflict policy
- UID mapping
- ETags
- sync tokens
- no external service as mandatory canonical state

## Derived temporal properties

- days until
- days since
- week number
- quarter
- electoral cycle
- duration
- overlaps
- gaps
- this weekend
- next N days
- position in sequence/collection
- derived fields usable in queries and color rules

## Relations and sequences

- event-to-event relations
- announcement -> deadline -> primary -> runoff -> general election
- hearing -> markup -> floor vote
- earnings release -> shareholder meeting
- season -> playoff -> final
- arbitrary collections/sequences independent from recurrence

## Personal annotation

- notes
- ratings
- relevance flags
- watched state
- reminders
- user tags
- local classifications
- source refresh must preserve annotations

## Bulk operations

- select hundreds/thousands of events
- tag
- annotate
- classify
- suppress from views
- export
- compare
- inspect sources
- apply watch/reminder rules

## Keyboard-first operation

- command palette
- fuzzy search
- date jump
- view switching
- selection
- filter control
- inspector control
- bulk actions
- minimal mouse dependency

## Local-first behavior

- browsing without internet
- search without internet
- filtering without internet
- saved views without internet
- annotations without internet
- history without internet
- network only for acquisition/synchronization

## Source rollover

- continuous feed detection
- edition/year-specific feed detection
- automatic rollover where possible
- new-edition discovery state
- stale projection state
- rebuild requirements
- visible source dependency state

## Reminders and notifications

- per-event alarms
- query/view-based reminder rules
- "one day before any federal election"
- "30 minutes before personal events"
- notification rules independent from source event mutation

## Importance and relevance

- public/objective importance
- personal relevance
- distinct fields
- independent effects on prominence, sorting, opacity, or notification

## Structured locations

- venue
- address
- jurisdiction
- locality/region/country
- coordinates
- virtual location
- conferencing information
- future geographic filtering/calendar-map integration

## Duplicate/entity resolution

- exact source identity
- canonical identity
- likely duplicate detection
- aliases
- source precedence
- intentional co-representation
- no simplistic title/date-only merging

## Table mode

- user-selected columns
- reordering
- sorting
- filtering
- rich event audit table
- columns such as date/title/jurisdiction/institution/type/source/status

## Pivot and quantitative exploration

- count by month
- count by state
- count by institution
- count by event type
- histograms/charts over current query
- summary views using the same query engine

## Stable identifiers and deep links

- stable event IDs
- stable source IDs
- stable view IDs
- stable collection IDs
- stable snapshot IDs
- deep links returning to exact object/view state

## Undo, redo, and audit

- undo/redo for user operations
- especially bulk operations
- source-driven changes distinguishable from user changes
- audit history sufficient to understand change origin

## Custom fields

- strongly typed core fields
- extensible namespaced properties
- schema versioning
- custom fields queryable and exportable

## Personal scheduling completeness

- create/edit/move
- drag-and-drop
- recurrence
- attendees
- locations
- reminders
- availability/free-busy
- tentative status
- invitations where justified
- personal appointments coexisting with public temporal data

## Tasks and deadlines

- tasks are not events by definition
- deadlines/start/due/completion can be projected into temporal views
- retain separate semantics
- do not reintroduce task-centric canonical storage

## Progressive disclosure

- default month view remains understandable
- ontology/query/provenance/history appear on demand
- powerful must not mean permanently overwhelming

## Visual customization

- event height
- label composition
- visible inline metadata
- font size
- time format
- weekend visibility
- week start
- working hours
- density
- separators
- group headers
- color-rule sets
- iconography
- opacity
- cancelled-event rendering
- custom event templates

## Performance

- immediate local filtering
- immediate recoloring
- immediate view switching
- immediate inspector opening
- indexed local search
- network-independent rendering
- scalable to large temporal corpora
