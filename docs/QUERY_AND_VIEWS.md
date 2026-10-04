# Query and View System

## Principle

A calendar in Ephemeris is primarily a **view over canonical temporal data**.

Saved views replace the conventional assumption that every conceptual calendar must be a separate physical container.

## Query capabilities

The long-term query system should support:

- AND
- OR
- NOT
- equality/inequality
- ranges
- set membership
- null checks
- text predicates
- date ranges
- relative dates
- nested expressions
- derived properties
- relation/collection predicates where useful

Examples:

```text
domain = politics
AND country = US
AND (branch = judicial OR event_type = election)
AND jurisdiction_level != local
```

```text
event_type = election
AND days_until BETWEEN 0 AND 30
```

```text
source.authority = official
AND status NOT IN (cancelled, superseded)
```

## Simple and advanced surfaces

Users should not need to write query expressions for common filtering.

The UI should expose facets for common fields:

- domain
- source
- geography
- jurisdiction
- institution
- event type
- status
- tags
- date range
- relevance
- importance

Advanced expressions can operate on the same underlying query representation.

## SavedView

A saved view should eventually store:

```text
id
name
description?
query
date_behavior
layout
grouping
sort_rules
color_rule_set
density
visible_fields
timezone
overlays[]
base_view_id?
ui_state{}
```

Views should be cloneable.

Inheritance/composition is desirable when a shared base rule should propagate.

## Calendar algebra

Views should eventually support composition such as:

- union
- intersection
- subtraction

Examples:

```text
US Politics - Congress
California ∩ Elections
Government ∪ Economics
Everything - Sports
```

This should compile to the canonical query model rather than copy event membership.

## Overlays

An overlay is an independently defined view rendered together with another view.

Examples:

- elections + economic releases + personal commitments
- tentative + confirmed with different visual treatments
- current snapshot vs historical snapshot
- projected vs observed

Overlay identity and styling should remain independent.

## Derived properties

Useful derived fields may include:

- days until event
- days since event
- week number
- quarter
- weekend
- this week
- this weekend
- next N days
- duration
- overlaps
- gap from previous related event
- relative position in a collection/sequence

Derived fields should be queryable without requiring them to be stored redundantly on every event.

## Grouping

Possible grouping dimensions:

- day
- week
- month
- source
- domain
- jurisdiction
- institution
- event type
- status
- collection

Grouping should be independent from sort and color.

## Sorting

Multiple stable sort keys should be possible.

Examples:

- chronological
- importance then time
- jurisdiction then time
- source then time
- personal relevance then time

## Color rules

Color is a rule engine.

Potential rule fields:

- domain
- jurisdiction
- branch
- institution
- event type
- source
- status
- confidence
- importance
- personal relevance
- tags
- custom properties

Rules require:

- precedence
- fallback
- deterministic application
- saved rule sets

A view may recolor events without changing event storage.

## Materialized views

Ordinary saved views should remain logical queries.

A materialized snapshot is a different object used when the exact result set at a point in time must be frozen.

Do not confuse "saved query" with "frozen snapshot."

## Search

Search should span:

- normalized title
- raw title
- description
- people
- organizations
- location
- tags
- source metadata
- identifiers
- jurisdiction

Search results should be convertible into filters/saved views.

Alias and normalized-entity support should be added as the ontology matures.

## Performance requirement

Common filtering and view switching should feel immediate.

The query representation must map efficiently to indexes and must not require network access.
