# ADR 0001: Ephemeris is a dedicated temporal product

Status: Accepted

## Context

Rivetr is the native Rust + egui successor to the broader Rivet productivity application.

Its calendar implementation became sufficiently substantial that a calendar-only direction was briefly considered for Rivetr itself.

That would have required demoting or hiding the broader product surfaces Rivetr was explicitly created to preserve.

## Decision

Create Ephemeris as a separate descendant dedicated to temporal information.

Rivetr remains the broad Rivet successor.

Ephemeris may inherit useful calendar implementation but does not inherit Rivetr's full product obligations.

## Consequences

Positive:

- calendar architecture can be designed around events rather than tasks
- Taria-native temporal semantics can be first-class
- Rivetr's identity remains intact
- non-calendar features do not compete for Ephemeris scope

Cost:

- shared code may diverge
- selective extraction/migration is required
- some calendar behavior may initially exist in two repositories
