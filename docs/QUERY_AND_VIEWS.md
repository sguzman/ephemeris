# Query and View System

## Principle

A calendar in Ephemeris is primarily a **view over canonical temporal data**.

Saved views replace the conventional assumption that every conceptual calendar must be a separate physical container.

The implemented system already separates:

- query/filtering;
- source visibility;
- date range;
- layout;
- grouping;
- sorting;
- color rules;
- overlays;
- timezone/week-start behavior.

Changing one dimension must not rewrite event storage.

## Current query model

Ephemeris has two query surfaces that compile into the same runtime filtering path.

### Simple facets

Implemented convenience controls:

- free-text search;
- domain;
- jurisdiction;
- lifecycle status;
- source visibility.

Simple facets are ANDed with the advanced expression tree.

### Advanced expression tree

Implemented recursive operators:

- AND
- OR
- NOT

Implemented predicates include:

- typed text comparison;
- text set membership;
- lifecycle-status sets;
- importance/personal-relevance integer comparisons;
- exists / missing checks;
- temporal-kind membership;
- explicit civil-date overlap;
- relative civil-date windows.

Expressions may nest arbitrarily.

The GUI exposes a recursive editor for constructing these trees without writing a query language manually.

## Temporal query context

Temporal predicates are evaluated with an explicit query context containing:

- display timezone;
- current civil day anchor.

This prevents relative queries from silently depending on UTC or machine-local assumptions.

Implemented relative windows include presets such as:

- Today;
- Next 7 days;
- Next 30 days;
- Previous 7 days.

Exact instants are converted to civil dates using the view timezone.

Month/year precision is only included when the predicate explicitly allows imprecise-span matching; Ephemeris does not invent a day.

## SavedView

A saved view is a durable product object stored in SQLite.

Current saved-view state includes:

```text
id
name
query
hidden_source_ids
calendar_view
calendar_layout
group_by
sort_rules[]
color_by
color_rules[]
overlays[]
display_timezone
week_start_monday
```

Saved views can currently be:

- created from current state;
- applied;
- updated from current state;
- deleted.

They do not copy event membership.

Legacy saved-view data previously held in UI state is migrated into SQLite.

## Layout and date range are independent

Date ranges:

- Year
- Quarter
- Month
- Week
- Day

Layouts:

- Grid
- Agenda
- Table

Examples:

```text
Month + Grid
Month + Agenda
Year + Table
Week + Agenda
```

A layout does not define the underlying query.

## Grouping

Implemented grouping dimensions:

- none;
- date;
- week;
- month;
- source;
- domain;
- jurisdiction;
- institution;
- event type;
- lifecycle status.

Agenda and Table use true group partitioning.

Grouping is independent from sort order; a sort key does not accidentally change group membership.

## Sorting

Multiple stable sort keys are implemented.

Current sort dimensions include:

- time;
- title;
- importance;
- personal relevance;
- source;
- domain;
- jurisdiction;
- institution;
- event type;
- lifecycle status.

Each key has ascending/descending direction.

Examples:

```text
importance desc -> time asc
jurisdiction asc -> time asc
source asc -> title asc
```

## Color rules

Color is implemented as a rule engine rather than calendar ownership.

A saved view has:

1. an ordered list of explicit color rules;
2. a semantic fallback `ColorBy` strategy.

A color rule contains:

- stable rule ID;
- name;
- enabled state;
- full recursive `QueryExpr` condition;
- explicit RGB color.

Semantics:

```text
first enabled matching rule wins
    else
semantic fallback ColorBy
```

Rules can be:

- added;
- edited;
- enabled/disabled;
- deleted;
- reordered.

The same color engine is applied across:

- Grid;
- Agenda;
- Table;
- unplaced/blocked records;
- overlays.

Fallback semantic color dimensions include:

- none;
- source;
- domain;
- jurisdiction;
- institution;
- event type;
- lifecycle status.

Color never changes event identity or bundle membership.

## Overlays

The first overlay system is implemented.

An overlay contains:

- stable overlay ID;
- name;
- enabled state;
- independent `EventQuery`;
- fallback semantic color strategy;
- ordered color rules.

Enabled overlay queries are unioned with the base saved-view query for visibility.

Overlay order is styling precedence.

An event included by an overlay remains the same canonical local event; overlays do not materialize copied membership.

Current overlay behavior is intentionally the ergonomic simultaneous-union surface.

## Calendar algebra

The next composition layer should generalize beyond overlay union to explicit logical composition:

- union;
- intersection;
- subtraction.

Examples:

```text
US Politics - Congress
California ∩ Elections
Government ∪ Economics
Everything - Sports
```

This must compile to logical query composition over canonical events.

The first implementation should avoid saved-view reference cycles; direct embedded operands are safer than arbitrary recursive saved-view references.

## Taria bundle membership versus saved views

Taria `CalendarSet` membership is imported upstream organizational metadata.

Ephemeris saved views are consumer-owned runtime queries/presentation.

They are related but not interchangeable:

```text
Taria bundle/CalendarSet membership
    = frozen upstream projection metadata

Ephemeris SavedView
    = local programmable runtime interpretation
```

A saved view may filter by imported bundle membership in the future without changing that membership.

See:

- `TARIA_BUNDLE_CONTRACT.md`
- `TARIA_INTEGRATION.md`

## Derived properties

Useful future derived fields include:

- days until event;
- days since event;
- week number;
- quarter;
- weekend;
- duration;
- overlap;
- gap from previous related event;
- relative position in a collection/sequence.

Derived fields should be queryable without requiring redundant storage on every event.

## Table

Table is now a first-class dense layout.

It currently:

- renders the same query result corpus as other layouts;
- respects grouping;
- respects stable sorting;
- respects color rules/overlay styling;
- exposes Taria-oriented core fields;
- selects directly into the existing event inspector.

Next Table work:

- user-defined columns;
- column ordering;
- widths;
- hidden/visible column state;
- saved column configuration per SavedView.

## Materialized views

Ordinary saved views remain logical.

A future materialized snapshot is a distinct object used when the exact result set at a point in time must be frozen.

Do not confuse:

```text
saved query
!=
frozen snapshot
```

## Search

Current free-text search spans the major retained event fields and extensible properties.

Long-term search should deepen into:

- people/entities;
- normalized aliases;
- organizations;
- structured locations;
- richer identifiers;
- indexed provenance.

Search results should remain convertible into filters/saved views.

## Performance requirement

Common filtering, grouping, view switching, and color-rule evaluation should feel immediate.

The query representation must map efficiently to local indexes and must never require a network round trip.
