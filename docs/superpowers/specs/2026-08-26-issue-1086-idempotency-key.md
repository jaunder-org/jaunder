# Issue #1086 — Model post idempotency keys as a domain value

## Outcome

AtomPub create requests parse a retry token once into `IdempotencyKey`, and that
domain type remains intact through post creation, duplicate lookup, and database
insertion. Existing client compatibility and replay behavior remain unchanged.

The type removes primitive transposition and stripping risk without changing the
HTTP grammar or database schema.

## Load-bearing decisions

- `IdempotencyKey` is a non-secret string domain type in `common`, following
  ADR-0063's standard string-newtype trailer.
- `FromStr` is the validation and canonicalization chokepoint: trim outer
  whitespace, reject an empty result, and store the trimmed value.
- `IdempotencyKey` itself accepts any otherwise-arbitrary non-empty string.
  There is no character-set, byte-length, scalar-length, UUID, base64url, or RFC
  8941 structured-field restriction.
- AtomPub preserves the existing `HeaderValue::to_str` compatibility boundary:
  - missing header means no idempotency key;
  - whitespace-only readable header text means no idempotency key;
  - bytes rejected by `HeaderValue::to_str` — including non-ASCII UTF-8 and
    invalid UTF-8 — mean no idempotency key;
  - non-empty readable header text becomes an owned `IdempotencyKey`.
- Boundary compatibility is deliberate: empty and unreadable headers are treated
  as absent rather than rejected with HTTP 400.
- Borrowed orchestration and lookup APIs use `Option<&IdempotencyKey>`;
  lifetime-free creation inputs and persisted values use owned
  `Option<IdempotencyKey>`.
- The ordinary non-secret SQLx bridge remains enabled. SQL binds and decodes
  operate on `IdempotencyKey`; decode revalidates the domain invariant.
- No migration is needed. Both backends already store the key as `TEXT NOT NULL`
  with `UNIQUE(user_id, key)` and no length/check constraint.
- Uniqueness remains scoped by `UserId`, not global.
- Existing replay semantics remain authoritative:
  - a fresh keyed create returns 201;
  - reusing the key for the same user returns the original post as 200;
  - payload equality is not checked, so reuse with different content still
    returns the original post;
  - another user may independently use the same key;
  - the key row is inserted atomically with the post, and a uniqueness collision
    rolls the attempted post back;
  - keys remain retained indefinitely.
- The issue's request to remove #697's follow-up note is superseded by the
  repository's frozen-archive rule: the historical plan remains unchanged.
  Closing #1086 resolves the follow-up, while ADR-0063 records `IdempotencyKey`
  as an adopted domain value.

## Acceptance

- `common` exposes `IdempotencyKey` with the standard owned/borrowed/serde/SQLx
  string-newtype interface and a named parse error.
- Type-level tests prove trimming, non-empty enforcement, acceptance of
  otherwise arbitrary UTF-8 text, serde behavior, SQLx compatibility where
  convention requires it, and the standard trailer contract.
- AtomPub parses a present header into an owned `IdempotencyKey` before calling
  post creation.
- No raw `String` or `str` represents an idempotency key in post-service inputs,
  post-storage inputs, duplicate lookup, or database binds.
- Valid keyed create behavior remains 201 on first use and 200 with the original
  post on reuse.
- Whitespace-only, non-ASCII UTF-8, and invalid-UTF-8 header cases exercise the
  real AtomPub request boundary and behave exactly like an absent key.
- Reusing a key with different post content returns the original post without
  creating the attempted post.
- The same key can create one post for each of two different users.
- Existing transactional behavior remains intact: a collision leaves no
  attempted post, audience, media-reference, or idempotency row behind.
- Both SQLite and PostgreSQL execute the storage/AtomPub contract tests
  according to repository backend conventions.
- ADR-0063 and the current architecture projection describe the typed
  idempotency-key contract; frozen #697 artifacts remain historical.

## Boundaries

- No database migration, new index, retention policy, garbage collection,
  expiry, or background cleanup.
- No payload fingerprint, request-body comparison, conflict response, or 409
  behavior.
- No new client-generated key format and no change to the Emacs/client retry
  algorithm.
- No RFC 8941 structured-field parsing and no dependence on the expired IETF
  idempotency-key draft.
- No broad HTTP-header validation cleanup outside `Idempotency-Key`.
- No behavior change to unkeyed post creation, slug retries, tags, audiences,
  media references, or publication state.
