# ADR 0002: Events are canonical; calendars are programmable views

Status: Accepted

## Context

Container-centric calendar applications commonly bind event ownership, visibility, organization, and color to physical calendar membership.

That model is poorly suited to a dense corpus whose events simultaneously belong to many conceptual categories.

## Decision

Ephemeris stores canonical events independently of saved calendar views.

Saved views are queries plus presentation configuration.

Filtering, grouping, sorting, coloring, and layout remain independent.

## Consequences

Positive:

- no duplication is required for cross-cutting conceptual calendars
- arbitrary views can be created over one corpus
- color and visibility can change without reorganizing storage
- calendar algebra and overlays become natural extensions

Cost:

- query and view persistence become core infrastructure
- interoperability with container-centric services requires projections
