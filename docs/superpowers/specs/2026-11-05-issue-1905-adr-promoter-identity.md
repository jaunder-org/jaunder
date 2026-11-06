# Proposed spec: ADR promoter identity uses REST user login

## Outcome

The ADR promoter recognizes its own pull request by the exact GitHub App bot
login returned in the REST pull-request `user.login` field. The identity check
continues to fail closed for any other author and for any PR that does not match
the configured promoter owner, branch, base, open state, and marker.

## Approach and tradeoffs

The promoter currently gets occupant metadata from `gh pr list` /
`gh pr view --json author`; their GraphQL-facing `author.login` for the app is
`app/jaunder-adr-promoter`. Live REST `GET /repos/{owner}/{repo}/pulls/{number}`
response for PR #1902 represents the same App as
`user.login: jaunder-adr-promoter[bot]`, the configured exact `BOT_LOGIN`. Use
GitHub REST endpoints directly as the single `PromoterPullRequest` source for
both open-PR discovery (`GET /repos/{owner}/{repo}/pulls` with `state=open`,
`head=<owner>:automation/adr-promoter`, `base=main`) and numbered lookup
(`GET /repos/{owner}/{repo}/pulls/{number}`). Do not call
`gh pr list`/`gh pr view` for promoter PR metadata, make paired
GraphQL-plus-REST identity reads, or add a cross-response consistency protocol.
Keep local occupant filtering and multiple-occupant rejection over REST list
results.

Map REST fields directly: `number`→number; `head.repo.owner.login`→head owner;
`user.login`→author; `head.ref`→head ref; `head.sha`→head SHA; `base.ref`→base
ref; `body`→marker body; `state`→open state; nullable `auto_merge`→auto-merge
armed. If `head.repo` or required identity/occupant values are absent, fail
closed (deleted/malformed head repository cannot qualify as promoter).
`in_merge_queue` remains independently derived as it is today; GraphQL
`snapshot_with` remains solely for checks/queue state and exact-head safety. Do
not accept aliases, prefixes, arbitrary app users, or normalized variants;
compare `user.login` exactly to existing `BOT_LOGIN`.

This replaces GraphQL projection reads for promoter PR metadata with REST
records. It avoids accepting divergent API aliases and avoids a second identity
read, while preserving the exact identity authority. Preserve existing
owner/head branch/base/open/marker guards and all downstream immutable-head and
queue protections.

## Acceptance

- Injectable production-read-path tests cover open-PR discovery and numbered-PR
  lookup parsing from raw REST-shaped responses returned by the specified GitHub
  REST endpoints. They demonstrate exact `user.login: jaunder-adr-promoter[bot]`
  identity recognition; GraphQL `author.login` is not part of the promoter-PR
  metadata source.
- Tests reject missing/malformed REST identity or occupant fields and multiple
  matching open occupants. An open PR on the configured repository’s exact
  promoter head branch/base is first selected as an occupant; if its author is
  foreign or its marker is absent, identity validation must return an error and
  stop the generation path (not silently treat it as no occupant, delete/replace
  its branch, or create a successor PR). A PR whose head repository owner,
  branch, base, or state does not match remains excluded by the existing
  occupant predicate. This preserves current `singleton` behavior: occupant
  filtering is followed by exact author/marker validation that errors on a
  different identity.
- REST parsing preserves nullable `auto_merge` semantics; `in_merge_queue` and
  check evidence continue to come from their existing independent snapshot path.
- Identity approval remains conjunctive with exact repository owner,
  `automation/adr-promoter` head branch, `main` base, open state, and
  `<!-- jaunder-adr-promoter -->` body marker. Tests reject wrong owner, branch,
  base, closed PR, or missing marker.
- Tests for generation/dequeue retain their existing exact-head and
  required-check/queue behavior; this correction does not broaden re-arming or
  mutation authority.
- No test or code path changes, retries, closes, edits, merges, or manual
  promotion of existing PR #1902. Its recovery requires a separate concrete plan
  and explicit authorization.

## Boundaries

Only the promoter PR identity API representation and focused regression coverage
are in scope. Do not change expected `BOT_LOGIN`, add accepted-login lists or
normalization, weaken any occupant/marker/authorship/branch/base/head guard,
change App permissions, alter workflow concurrency, retry failed runs, or modify
PR #1902. No implementation until explicit human approval of this spec. Approval
to implement does not approve merging any PR; per-PR merge approval remains
separate.
