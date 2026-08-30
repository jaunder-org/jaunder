# Issue #215 — prompt for new Post metadata

## Outcome

`jaunder-new-post` gathers a new Post's title, Tags, and publication state
before creating and visiting its Org file. Server-advertised Tags assist entry
without making local authoring depend on server availability;
`C-u jaunder-new-post` retains a prompt-free minimal-template path.

## Load-bearing decisions

- The existing `jaunder-new-post` command gains the richer behavior; no second
  interactive command is introduced. Its non-interactive creation core remains
  prompt-free and directly testable.
- The ordinary command resolves the target blog before gathering metadata, then
  collects every answer before creating the file. Cancelling any prompt creates
  no file.
- Ordinary invocation retains the existing target rule: use the longest matching
  configured blog, prompt for a configured blog when none matches, and use
  `default-directory` only when no blogs are configured.
- Title is optional and defaults to empty.
- Tags are entered one at a time until an answer is empty after trimming. Each
  prompt offers the authenticated AtomPub Service Document's Posts Collection
  `app:categories` as completions while permitting a new free-text Tag label;
  categories belonging to other Collections are ignored.
- A nonempty answer must satisfy the existing Tag-label boundary; invalid input
  re-prompts. Tag order is preserved, while a later answer resolving to the same
  canonical Tag slug is omitted. Accepted labels are serialized as one
  comma-separated `#+KEYWORDS:` value.
- Tag discovery is best effort. A missing configuration, authentication error,
  network error, or malformed Service Document produces one non-blocking message
  and continues with free-text Tag entry.
- The stable AtomPub Service Document is the sole server source. The Emacs
  Protocol Client does not consume the private `/api/tags/list` server function,
  and this issue does not add a user-scoped tag endpoint.
- Status is chosen from exactly `draft`, `published`, and `scheduled`,
  defaulting to `draft`, and is written as canonical lowercase `JAUNDER_STATUS`
  metadata.
- Draft and published Posts retain the existing creation-time `#+DATE:` stamp.
  Scheduled Posts instead prompt through the Org date reader until given a
  parseable future instant. Cancellation still creates no file.
- The resulting template retains its empty `#+DESCRIPTION:` and leaves point at
  the body after visiting the saved file.
- `C-u jaunder-new-post` bypasses metadata collection and tag discovery and
  creates the existing minimal template. It uses the blog containing
  `default-directory`; when configured blogs exist but none matches, it fails
  without creating a file or prompting. Only an entirely unconfigured client
  falls back to `default-directory`.
- Format prompting and `JAUNDER_FORMAT` creation are deferred. V1 remains Org
  only; this intentionally narrows the original issue text rather than
  presenting a one-choice prompt whose answer has no current conversion effect.

## Acceptance

- Ordinary invocation uses the longest matching configured blog, prompts for a
  configured blog when none matches, and falls back to `default-directory` only
  when no blogs are configured. It then prompts for title, repeated Tags, and
  status before a file exists and writes the selected values into the canonical
  Org metadata block.
- Empty title and immediate empty Tag input produce valid empty metadata fields.
- Tag completion reads the Posts Collection's advertised Service Document
  categories, ignores other Collections, accepts a new valid label, re-prompts
  for an invalid label, stops on trimmed empty input, preserves entry order, and
  omits duplicate canonical Tag slugs.
- Unavailable or invalid Service Document data emits one message and still
  permits successful local Post creation with free-text Tags.
- Draft and published selection preserve a creation timestamp. Scheduled
  selection rejects malformed, present, and past instants until a future Org
  date is supplied.
- Cancelling during any metadata or scheduled-date prompt leaves no new file.
- `C-u` creates the pre-existing minimal template without metadata, tag-server,
  or blog-choice prompts. It succeeds through longest-prefix matching, fails
  without a file when `jaunder-blogs` is nonempty and no configured root
  contains `default-directory`, and uses `default-directory` when no blogs are
  configured.
- The created buffer is saved, visited, and positioned in the body in both
  ordinary and `C-u` success paths.

## Boundaries

- No format selector, converter dispatch, or `JAUNDER_FORMAT` write.
- No new tag endpoint, private server-function dependency, tag-catalog caching,
  client-side server tag-limit policy, or user-scoped tag semantics.
- No publishing, server-side draft creation, or other network write during new
  Post creation.
- No CONTEXT.md or ADR change: the feature uses the existing Post, Tag, Protocol
  Client, and AtomPub Service Document vocabulary and boundaries.
