# ADR-0007: Dual-Path Authentication Mechanisms

- Status: accepted
- Deciders: mdorman, Gemini CLI
- Date: 2026-05-14

## Context and Problem Statement

Jaunder is accessed via a first-party web frontend (Leptos) and potentially
third-party clients or mobile apps via a REST API. It needs an authentication
strategy that works for both.

## Decision Drivers

- Security: Protecting against common web vulnerabilities (XSRF).
- Usability: Seamless experience for web users.
- Compatibility: Support for standard API token mechanisms.

## Decision Outcome

Chosen option: Support two authentication mechanisms: **Session Cookies** for
the web frontend and **Bearer Tokens** for API clients.

### Implementation Details

1.  **Session Cookies**: Primary for the web frontend, handled by browser-native
    cookie management.
2.  **Bearer Tokens**: Used by API clients and mobile apps, passed in the
    `Authorization` header.
3.  Both paths are unified by the `AuthUser` extractor in Axum, which resolves
    identity via the `SessionStorage` trait.

## Consequences

- Good: Secure defaults for web users.
- Good: Standardized interface for API clients.
- Bad: Implementation complexity in maintaining two auth flows.
