# ADR-0000: Documentation Strategy

- Status: accepted
- Deciders: mdorman, Gemini CLI
- Date: 2026-05-14

## Context and Problem Statement

The Jaunder project's documentation has grown organically, leading to a mix of
permanent living documents, historical milestones, and transient implementation
plans. This lack of clear boundaries makes it difficult to distinguish between
the current state of the system, the reasoning behind decisions, and the future
vision.

## Decision Drivers

- Clarity: Distinction between technical structure, functional behavior, and
  decision reasoning.
- Maintainability: Minimizing "noise" by pruning transient files.
- Discoverability: A consistent structure that makes information easy to find.
- History: Preserving architectural context without cluttering day-to-day work.

## Decision Outcome

We will adopt a four-tiered documentation strategy:

1.  **Architecture Decision Records (ADRs)**: Located in `docs/adr/`. These
    records capture the _reasoning_ (the "Why") behind significant architectural
    and design choices. They follow the MADR template.
2.  **ARCHITECTURE.md**: A living document describing the internal technical
    structure (the "What" and "Where"). It provides a map of the codebase, crate
    responsibilities, and internal data flow, linking to ADRs for detailed
    rationale.
3.  **DESIGN.md**: A living document describing the system's functional behavior
    and operational model (the "What" and "How"). It focuses on interfaces,
    user-facing logic, and protocol interactions.
4.  **ROADMAP.md**: A living document describing the project's strategic vision
    and future milestones (the "When").

### Transient Documentation

Documents created for specific development tasks (milestones, implementation
plans, design specs) are considered transient. They should be committed to git
during development but deleted once the work is complete or captured in living
docs/ADRs. Git history remains the authoritative source for these historical
details.

## Consequences

- Good: Clear separation of concerns between different types of documentation.
- Good: ADRs provide a stable, numbered history of decisions.
- Good: Living docs stay concise and focused on the current state.
- Bad: Requires discipline to update living docs and create ADRs as the system
  evolves.
