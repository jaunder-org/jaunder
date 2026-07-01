# ADR-0009: High-Fidelity Retention Policy

- Status: accepted
- Deciders: mdorman, Gemini CLI
- Date: 2026-05-14

## Context and Problem Statement

Content on the decentralized web is frequently edited or deleted. Jaunder needs
a policy for how to handle these changes for followed sources.

## Decision Drivers

- Fidelity: Preserve the history of consumed content.
- User Agency: Allow users to see what has changed.
- Integrity: Prevent silent erasure of information from the user's private
  archive.

## Decision Outcome

Chosen option: **High-Fidelity Retention** via immutable revisions.

### Implementation Details

1.  **Edits**: When an update activity/event is received, the revised item is
    stored as a new immutable revision alongside all prior versions.
2.  **Deletions**: Inbound delete requests may hide content from active views
    but do not purge previously stored data from the database.
3.  The UI shows the latest version by default but retains access to the
    history.

## Consequences

- Good: Resilient against "gaslighting" or accidental loss of information.
- Good: Provides a complete historical record for the user.
- Bad: Increased database growth over time.
