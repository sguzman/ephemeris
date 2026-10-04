# Contributing

## Project contract first

Before changing architecture, read:

- `README.md`
- `docs/PRODUCT.md`
- `docs/ARCHITECTURE.md`
- `docs/DATA_MODEL.md`
- relevant ADRs

Ephemeris is a dedicated temporal-information application. Do not casually broaden it back into a general productivity suite.

## Development priorities

Prefer work that improves:

- temporal correctness
- source/provenance integrity
- local queryability
- dense calendar usability
- Taria integration
- interaction latency
- inspectability

## Rivetr code reuse

Rivetr is an ancestor and source of proven implementation ideas.

Copying code is not automatically correct.

When importing Rivetr code:

1. identify task-model coupling
2. preserve useful tests
3. rename concepts to temporal semantics where appropriate
4. remove machine-specific hacks
5. document important behavior differences

## Architectural changes

Changes to these areas should receive an ADR:

- canonical storage engine
- event identity model
- Taria interchange format
- recurrence engine
- query language/representation
- source ownership/conflict policy
- major async/runtime model changes

## Quality gates

Once code exists, the repository should maintain at least:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets
cargo test --workspace
```

Additional fixture/performance checks should be added as subsystems appear.

## Directness

Keep implementation commits scoped and descriptive.

Do not introduce broad framework abstractions without a concrete current need.

Do not add network services to interactive render/query paths.

## Documentation

Behavioral or architectural changes must update the corresponding documentation in the same development tranche.
