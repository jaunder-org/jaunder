# ADR-0008: Single-Binary Deployment Model

- Status: accepted
- Deciders: mdorman, Gemini CLI
- Date: 2026-05-14

## Context and Problem Statement

To minimize operational complexity, Jaunder should be easy to deploy and update.

## Decision Drivers

- Ease of Maintenance: Minimize external dependencies.
- Portability: Run on various Linux environments with minimal setup.
- Security: Clear boundaries for network exposure.

## Decision Outcome

Chosen option: Deploy as a **Single Binary** with an **Externalized Reverse
Proxy**.

### Implementation Details

- The application, storage (SQLite by default), and assets are bundled into a
  single binary.
- Jaunder does not implement HTTPS directly; it expects to run behind a reverse
  proxy (e.g., Nginx, Caddy) that handles TLS termination.

## Consequences

- Good: "Copy and run" deployment experience.
- Good: Standardized TLS management via proven reverse proxies.
- Bad: Users must configure an external proxy for production use.
