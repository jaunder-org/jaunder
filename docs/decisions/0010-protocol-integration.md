# ADR-0010: Multi-Protocol Integration Strategy

* Status: accepted
* Deciders: mdorman, Gemini CLI
* Date: 2026-05-14

## Context and Problem Statement

Jaunder aims to be a unified reader for ActivityPub, AT Protocol, and web feeds (RSS/Atom). These protocols have fundamentally different delivery mechanisms.

## Decision Drivers

*   Real-time Delivery: Prioritize push over polling where possible.
*   Resource Efficiency: Avoid excessive polling.
*   Reliability: Ensure content is eventually delivered even without push support.

## Decision Outcome

Chosen option: **Push-First with Adaptive Polling**.

### Implementation Details

1.  **Push (Priority)**: Use AP Inbox delivery, WebSub (RSS/Atom), and AT Jetstream for real-time updates.
2.  **Adaptive Polling (Fallback)**: For sources without push support, estimate a polling schedule based on historical frequency (e.g., between 15 minutes and 24 hours).
3.  All sources are normalized into the **Unified Content Model** (see ADR-0005).

## Consequences

*   Good: Fast updates for modern sources.
*   Good: Minimal server load for infrequently updated feeds.
*   Bad: Complexity in maintaining multiple ingestion pipelines and an adaptive scheduler.
