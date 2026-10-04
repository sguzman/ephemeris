# Product Contract

## Definition

Ephemeris is a **local-first temporal information system** for dense, heterogeneous, provenance-bearing event data.

Its product definition is:

> **Events are canonical data. Calendars are programmable views.**

Ephemeris is calendar-only in product scope, but "calendar" is interpreted broadly enough to include institutional schedules, public events, historical and projected temporal data, recurring events, deadlines, personal appointments, temporal sequences, and arbitrary saved views over the same corpus.

## Problem statement

Most calendar applications optimize for a sparse personal schedule. They assume:

- a small number of events per day
- calendar containers are the primary organizational unit
- container membership controls visibility
- container membership often controls color
- events are mostly title, start, end, location, and description
- month/week/day views are sufficient
- source provenance is unimportant
- change history is disposable
- a cloud service may be treated as canonical state

Ephemeris explicitly rejects those assumptions.

The primary target corpus may contain large numbers of events from politics, legislatures, courts, election authorities, statistical agencies, central banks, finance, business, sports, holidays, academic or media schedules, research data, and personal scheduling.

## Core principles

### One canonical event corpus

An event should exist canonically once even when it belongs to many conceptual sets.

"US Politics", "Congress", "California", "Elections", "Economic Releases", and "Sports" should normally be metadata, queries, collections, or views rather than duplicate physical calendars.

### Organization is not visibility

An event's ontology or source membership must not determine whether it can be shown.

Visibility is a query concern.

### Color is not ownership

Color is a presentation rule. A view may color the same events by source, domain, jurisdiction, institution, event type, lifecycle state, confidence, relevance, or any custom rule.

### Filtering, grouping, sorting, coloring, and layout are independent

A view may:

- filter to US federal politics
- group by day
- sort by importance
- color by branch
- render as a compact agenda

Another view may use the same events while grouping by state and coloring by election type.

These operations must never be coupled by the data model.

### Saved views are first-class

A saved view is a durable query plus presentation configuration, not a copied calendar.

A saved view may eventually contain:

- query/filter expression
- date-range behavior
- layout
- grouping
- sort rules
- color rules
- density
- visible fields
- timezone
- overlays
- inheritance/composition metadata

### Provenance is product data

The distinction between:

- official native ICS
- official API
- official webpage converted to structured events
- official PDF/schedule extraction
- aggregator feed
- manual event
- derived/projected event

must survive ingestion and remain inspectable.

### Temporal uncertainty is real

Events may be:

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

These states should not be flattened to present/absent.

### History matters

For source-backed calendars, "what changed?" is often as important as "what is scheduled?"

Ephemeris should eventually support:

- added events
- removed events
- moved events
- changed fields
- cancellations
- snapshot comparison
- as-of inspection

### Dense data is normal

Dozens or hundreds of events on a day are valid data, not an error condition.

The UI must degrade through aggregation and density controls rather than become unusable.

### Local first

Existing data, search, filtering, saved views, inspection, annotations, and history must work with no network connection.

Network access is for acquisition and synchronization.

### Keyboard first

Core navigation and interrogation should be efficient without a mouse.

### Progressive complexity

A month view should remain understandable immediately. Advanced query, provenance, history, ontology, and projection machinery should appear when needed.

## Product boundaries

### In scope

- temporal event storage and inspection
- calendar and temporal visualizations
- querying and saved views
- source/provenance modeling
- recurrence and timezone semantics
- ingestion and refresh
- source snapshots/history
- annotations
- bulk temporal operations
- export/projection
- personal scheduling
- deadlines and tasks only as temporal projections or related object types
- source health and rollover where temporal feeds require it

### Out of scope by default

- general task management
- Kanban
- contact management as a standalone product
- dictionary/reference workspaces
- generic map application
- broad note-taking
- generic productivity dashboards
- Tauri/React/WebView shell
- Taskwarrior compatibility as a product goal

Some out-of-scope domains may later participate as integrations. That does not make them Ephemeris product surfaces.

## Calendar anti-spec

Ephemeris should resist these design assumptions:

- calendar container = ontology
- calendar container = visibility
- calendar container = color
- event = title/time/description only
- month/week/day = complete visualization vocabulary
- sparse schedules = normal density
- source/provenance = irrelevant
- history = disposable
- external cloud copy = canonical
- importing a source means adopting its ontology
- exporting a view means reorganizing canonical storage

## Ordinary personal calendar behavior

Rich temporal-data support does not remove ordinary scheduling requirements.

Ephemeris should eventually support:

- create/edit/move events
- recurring appointments
- attendees
- structured and virtual locations
- reminders
- availability/free-busy
- tentative status
- invitations where useful
- personal annotations
- personal and public events in the same query/view system

The richer model should subsume ordinary calendars without being constrained by them.

## Decision tests

Before implementing a feature:

1. Does it improve local interrogation, understanding, or visualization of temporal data?
2. Does it preserve canonical event identity?
3. Does it retain provenance rather than flatten it?
4. Does it work for dense data?
5. Does it avoid coupling organization to presentation?
6. Can it remain responsive without a network round trip?

If a feature fails these tests, it requires explicit justification.
