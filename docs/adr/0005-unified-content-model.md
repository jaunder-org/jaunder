# ADR-0005: Unified Content Model

* Status: accepted
* Deciders: mdorman, Gemini CLI
* Date: 2026-05-14

## Context and Problem Statement

Jaunder consumes content from multiple protocols (ActivityPub, AT Protocol, RSS/Atom/JSON Feed). It needs a way to present this diverse data consistently to the UI and API while preserving the original source material.

## Decision Drivers

*   Consistency: A single core representation for the UI.
*   Fidelity: Preservation of original protocol data for future re-processing.
*   Performance: Low read-time overhead for normalized data.

## Decision Outcome

Chosen option: Store items in two forms at write time: **Raw payload** and **Processed form**.

### Implementation Details

1.  **Raw payload**: The original protocol data (JSON-LD, XML, etc.) stored without alteration as the "source of truth."
2.  **Processed form**: A normalized unified representation (Core fields + Protocol-specific extension blob).
3.  The API and UI read from the processed form.

## Consequences

*   Good: Ingestion cost is paid once at write time.
*   Good: Logic changes can be applied retrospectively by re-processing raw payloads.
*   Good: Protocol-specific extensions allow advanced clients to access non-unified features.
*   Bad: Higher storage overhead due to duplicate data.
