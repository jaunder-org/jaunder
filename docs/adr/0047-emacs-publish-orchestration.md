# ADR-0047: The Emacs Publish Orchestration — Multi-Blog Config via Dynamic Specials, ID-First Safe-to-Resume Write-Back

- Status: proposed
- Deciders: mdorman, Claude
- Date: 2026-07-03

## Context and Problem Statement

C4 (#162) is the final Unit-C sub-issue: it wires the C1 transport, C2 org→atom
mapping, and C3 media upload into the end-to-end `jaunder-new-post` /
`jaunder-publish` / `jaunder-save-draft` lifecycle. Two cross-cutting shape
decisions govern the orchestration and outlive this issue — how a buffer's
target blog is chosen and threaded to the already-shipped transport, and how the
publish sequence stays safe to retry. This ADR records them; the command
surface, field mapping, and server contract live in the issue spec
(`docs/superpowers/specs/2026-07-03-issue-162-emacs-publish-flow.md`).

## Decision Drivers

- The client must support **more than one blog** (one Emacs, several Jaunder
  instances/accounts) selected by which directory a post lives in.
- C1/C2/C3 already ship and are tested against the `jaunder-base-url` /
  `jaunder-username` special variables; adding multi-blog should **not** re-open
  or re-test the transport layer.
- A single-blog user who only sets the two globals must keep working.
- Publishing mutates both the server and the on-disk file; a failure at any step
  (including a `412` stale-ETag) must be recoverable by a plain re-publish,
  never leaving a torn state.

## Decision Outcome

### D1 — Multi-blog config is a directory→blog alist, resolved by longest-prefix match

A `jaunder-blogs` `defcustom` maps `(DIRECTORY . PLIST)` with `:base-url`,
`:username`, and optional `:format`. The active blog for a buffer is the entry
whose `DIRECTORY` is the **longest prefix** of the buffer file's expanded
directory, so nested blog roots resolve to the most specific. Resolution falls
back, in order, to (1) a matching `jaunder-blogs` entry, (2) the single-blog
globals synthesized into a plist, (3) a loud error naming the unconfigured
directory. This keeps the config declarative and the single-blog path
zero-config-migration.

### D2 — Thread the active blog by dynamically `let`-binding the transport specials

The commands resolve the blog and `let`-bind the **existing** `jaunder-base-url`
/ `jaunder-username` around the publish flow. The C1/C2/C3 helpers are **not**
refactored to take a blog parameter — they keep reading the specials and simply
observe the active blog's values within the dynamic extent. (The blog's
`:format` is accepted for forward-compatibility but **not** threaded in v1: org
is the only converter, so a bound format special would be inert dead config.)
This is the least-invasive way to add multi-blog: no change to the
transport/auth/media surface, no re-test of shipped code, and the dynamic
binding is naturally buffer-scoped (each command re-resolves from its buffer).
The alternative — plumbing a `blog` argument through `jaunder--http-request`,
`jaunder--auth-secret`, `jaunder--upload-media`, `jaunder--localize-media` —
would touch every shipped C1/C3 function for no behavioral gain and is rejected.

### D3 — `JAUNDER_ID`-first, safe-to-resume send/write-back ordering

The publish sequence performs **all network mutation before any destructive
local change**: validate → media upload (idempotent, sha256-dedup) → entry send
(`POST` create, or `PUT`+`If-Match` when `JAUNDER_ID` is present) → **only
then** write back and rename. A pre-response failure (incl. `412`) leaves the
on-disk file pristine to retry. On success the write-back persists `JAUNDER_ID`
**first** (the numeric tail of the `Location` header), before `JAUNDER_SLUG` /
`JAUNDER_SYNCED` / the publish time and before the temp→`<slug>.org` rename — so
a failure after the server committed (e.g. a rename collision) degrades to a
self-healing `PUT` on the next publish rather than a duplicate `POST`.

## Consequences

- Good: multi-blog support with zero churn to the shipped transport/media layer;
  the change is additive and the single-blog config still works untouched.
- Good: publish is idempotent-to-retry at every step; the only residual
  duplicate window is create-`POST`-response-lost, a known limitation (server
  idempotency key is follow-on #79).
- Neutral: correctness now depends on the commands establishing the dynamic
  binding — a future caller that invokes a transport helper **outside** a
  `jaunder--with-blog`-style binding would hit the globals/error path. The
  binding is centralized in the three commands to contain this.
- Bad/So-what: the dynamic-specials approach is less explicit than argument
  passing; mitigated by keeping the binding in one wrapper and documenting it
  here.

## Verification

- Pure ERT: `jaunder--resolve-blog` (longest-prefix, globals fallback,
  unconfigured error); create-vs-update decision and `JAUNDER_ID` extraction
  from a `Location` URL; write-back property ordering.
- Live ERT (`jaunder-test--with-live-server`): publish creates then re-publish
  updates (not duplicates); a stale `If-Match` → `412` surfaced with the file
  left pristine; `JAUNDER_ID`/`JAUNDER_SLUG`/`JAUNDER_SYNCED` written back.
