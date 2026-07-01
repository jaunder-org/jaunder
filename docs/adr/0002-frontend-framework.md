# ADR-0002: Frontend Framework Selection

- Status: accepted
- Deciders: mdorman, Gemini CLI
- Date: 2026-05-14

## Context and Problem Statement

Jaunder requires a modern, reactive web interface that supports both Server-Side
Rendering (SSR) for performance/SEO and client-side interactivity.

## Decision Drivers

- Language Consistency: Prefer Rust for both backend and frontend.
- Performance: Fast initial page loads via SSR.
- Developer Experience: Reactive programming model and type safety.

## Considered Options

- Leptos: Full-stack Rust framework with fine-grained reactivity and
  SSR/Hydration.
- Dioxus: Multi-platform Rust framework (good for mobile, but SSR was less
  mature at the time).
- Vanilla JS/TS: Decoupled frontend, but loses language consistency and type
  safety across the boundary.

## Decision Outcome

Chosen option: "Leptos", because it provides a cohesive full-stack Rust
experience, excellent performance through fine-grained signals, and built-in
support for SSR and hydration.

## Consequences

- Good: Shared code between server and client (e.g., types, validation logic).
- Good: Type-safe "Server Functions" for seamless client-server communication.
- Bad: Compiling Rust to WASM can result in large binary sizes.
- Bad: Leptos is a fast-moving framework with occasional breaking changes.
