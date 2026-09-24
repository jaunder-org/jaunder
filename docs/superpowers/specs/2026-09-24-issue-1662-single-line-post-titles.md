# #1662 — Single-line authored Post titles

## Outcome

A Post title is optional, but every present authored title is one logical source
line. Newline-bearing titles fail validation rather than becoming untitled,
flattened, or partly saved. Enforce the rule now across web, Org, AtomPub, and
the Emacs Protocol Client in the same issue.

## Load-bearing decisions

- A present Post Title is non-blank and contains none of CR, LF, vertical tab,
  form feed, Unicode next-line, line separator, or paragraph separator, anywhere
  in its source (including the edges). Check before the usual outer whitespace
  trim. Surrounding non-line-breaking whitespace is still trimmed; internal
  non-line-breaking whitespace, case, and the absence of a length bound remain.
- An absent title stays absent. An AtomPub title containing only
  non-line-breaking whitespace retains its absent-title meaning. Any line
  separator, even as the entire title, instead rejects the request; do not
  convert a failed title parse into absence. Other AtomPub fields retain their
  current lenient policy.
- The server applies one domain invariant to every accepted Post create/update.
  Repeated Org `#+TITLE:` lines compose a newline and therefore reject, even if
  another structured title is present. A single valid header and no header
  retain the precedence and titleless behavior from ADR-0155. Markdown's
  existing first-heading derivation and Org's first-heading fallback stay
  source-line-based; an invalid derived heading rejects instead of falling back
  to an untitled Post. No source-content rewriting sidesteps validation.
- The Emacs Protocol Client rejects invalid local titles before Service Document
  discovery, Post requests, link/Media uploads, or local write-back. A malformed
  remote title must not corrupt an existing local Org file on pull.
- A failed create or update leaves the stored Post, metadata, and body
  unchanged; a failed Emacs publish/pull leaves local authored files and durable
  create state intact. Web invalid submissions surface a validation error;
  AtomPub invalid submissions return a bad request rather than silently saving
  without a title.
- This is an authored-source rule, not a rendered-line or CSS rule. Long titles
  may wrap; inline source markup such as HTML `<br>` remains permitted and is
  rendered under ADR-0204's unchanged inline sanitization policy.
- The owner confirms no affected production titles. No backfill, schema change,
  legacy decode path, new length limit, or replacement-title syntax is needed.
  Record the policy in `CONTEXT.md` and a draft ADR projected into the
  architecture view. The issue includes implementation and tests, not deferred
  enforcement tickets.

## Acceptance

- Domain-value tests reject each separator at the beginning, middle, and end,
  including line-separator-only input; still accept ordinary one-line titles,
  trim surrounding non-line-breaking whitespace, preserve internal whitespace
  and case, and retain optional/blank semantics at the relevant boundary.
- Web and AtomPub create/update tests prove invalid explicit titles and repeated
  Org title headers cannot save a Post or partially change an existing one. A
  valid single Org title, titleless Post, and blank AtomPub title still work;
  structured-title precedence does not excuse an invalid Org header.
- Focused create/update tests for Markdown and Org first-heading title
  candidates containing non-CR/LF separators prove they reject rather than
  become untitled and leave previously stored content unchanged; normal derived
  titles still work.
- A live browser test exercises an Org Post with repeated title lines and checks
  visible validation and absence of a saved Post. If its presentation changes,
  capture comparable before/after evidence for that composer state.
- Table-driven Emacs pure tests cover CR, LF, VT, FF, NEL, line separator, and
  paragraph separator with representative beginning, middle, and end placement.
  Live tests prove repeated and edge-break TITLE metadata fails locally before
  any HTTP request or Media upload, without changing buffer, saved file, or
  create checkpoint. Pulling a malformed remote title does not replace a matched
  local Post. Normal single-line publish/pull round-trips.
- The applicable focused suites and repository gates pass on the final branch;
  backend parity and the existing rendered-title behavior remain intact.

## Boundaries

No production-title migration or automatic cleanup; no change to titleless
Posts, summary/body grammar, slugs apart from rejecting invalid title input,
rendered `<br>` policy, visual wrapping, or unrelated metadata tolerance.
