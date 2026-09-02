# Issue #1214: Replace conflicted promoter attempts from fresh main

## Outcome

A generation event may retire one armed immutable ADR promoter attempt only when
both GitHub and local Git positively prove that its exact head conflicts after
`main` advanced. The controller may separately abort an unarmed, incompletely
published generated PR if `main` moved during its create/arm transaction. It
never rebases, force-updates, or edits either PR's head, diff, or content in
place.

## Trust and event boundary

- Only `Generate` events—`push` to `main` and `workflow_dispatch`—may replace.
  They already share the non-canceling `adr-promoter-generate` concurrency
  group. A `dequeued` event remains exact-head re-arm recovery only and never
  closes, deletes, or replaces.
- The stable branch is controller-owned. Repository administrators and other
  principals with Contents or Pull Requests write permission are trusted not to
  mutate it during a controller run. No marker, author string, comment, or
  commit trailer alone authorizes later adoption or deletion.
- App permissions and workflow triggers remain unchanged. Pull Requests write
  covers close/create/arm; Contents write covers authenticated Git ref mutation.

## Positive replacement evidence

The open PR must match the durable promoter identity: exact
`jaunder-adr-promoter[bot]` author, repository owner, stable branch, `main`
base, body marker, PR number, and head SHA.

Replacement requires one captured tuple
`(PR number, head H, generated parent B, current main M)` and all of:

1. GitHub reports `mergeable = CONFLICTING` and `mergeStateStatus = DIRTY`;
   `UNKNOWN`, `BLOCKED`, pending/running checks, queue delay, and failed checks
   do not authorize replacement.
2. The promoter head has exactly one parent `B`.
3. Git proves `B` is a strict ancestor of `M`.
4. A clean local merge-tree operation over the exact fetched objects `M` and `H`
   independently reproduces a content conflict. GitHub's asynchronously computed
   mergeability is a policy signal, not the sole conflict proof.

Immediately before retirement, fetch/re-read and require the same PR/head
identity, remote branch `H`, current main `M`, strict ancestry, and local
conflict. Then append an immutable machine-readable retirement-intent comment
containing the exact evidence tuple. Re-read it and require both the
`jaunder-adr-promoter[bot]` author and
`performed_via_github_app.client_id == ADR_PROMOTER_APP_CLIENT_ID`, then
revalidate the tuple once more. The controller never edits or deletes this audit
comment. Any changed or ambiguous evidence fails before ref mutation.

## Exact retirement and branch recreation

Retirement linearizes at the stable ref:

1. Require the exact App-authored retirement-intent comment. It is durable
   authorization to resume the already-proved transition, not a substitute for
   rechecking its exact Git objects.
2. Delete `automation/adr-promoter` first through Git receive-pack with an
   explicit SHA lease:
   `--force-with-lease=refs/heads/automation/adr-promoter:<H>` and a deletion
   refspec. This is a server-side compare-and-delete; a changed head fails
   without mutation.
3. Verify the stable ref is absent and the PR still reports head `H`, then close
   that exact PR number. Verify it is closed at `H` before proceeding.
4. Recreate the absent stable branch only with the existing non-force push and
   verify the resulting remote SHA.

The lease is used only for exact deletion; the controller never force-updates a
branch. REST/GraphQL ref deletion is forbidden because it has no expected-SHA
precondition. GitHub PR close has no expected-head CAS. Within the supported
controller concurrency model, deletion-first plus generation serialization
prevents a stale controller from deleting or closing a successor: no other
controller can recreate the ref before close. A privileged external write during
that critical section violates the controller-owned-branch invariant; the
postcondition detects it but GitHub cannot undo a completed close. The
controller never recreates the branch until close is verified. A deterministic
local-bare-repository test proves exact leased deletion, changed-head refusal,
and non-force recreation.

## Generated commit provenance

Every generated promoter commit carries canonical parseable trailers:

- `Jaunder-Promoter-Version: 1`
- `Jaunder-Promoter-Base: <fresh-main-sha>`
- replacement commits additionally carry
  `Jaunder-Promoter-Replaces: <pr-number>@<stale-head-sha>`

The base trailer must equal the commit's sole parent. The stable branch SHA is
the generated commit identity; no self-referential SHA is stored.
`Jaunder-Promoter-Version` selects immutable generator/verifier semantics, not
merely a parser shape. A generator behavior change must bump the version and
retain verification for any version that can still exist in remote PR/ref state;
otherwise a legitimate interrupted commit could become unverifiable.

Trailers are recovery coordinates, not authorization. Before adopting or
deleting an orphan generated commit, the controller checks out its exact trailer
base in a clean detached state, reruns deterministic promotion, and requires the
resulting tree to equal the candidate commit tree. It also requires canonical
parent count, message, and trailers plus an exact closed marked App-authored PR
and matching App-authored retirement intent. A structurally valid trailer on a
different tree fails closed. Author and committer strings inside Git commits are
not trusted as authentication.

## Durable state and resumption

The controller reads the latest exact promoter attempt across open, merged, and
closed states and the stable branch. The following cases are ordered; the first
matching case wins:

- **Latest open with exact retirement or publication-abort intent:** reproduce
  the intent's recorded exact-object proof. With branch still at `H`, continue
  leased deletion; with branch absent, continue exact close. Recovery does not
  depend on GitHub recomputing mergeability after the durable intent was
  verified.
- **Latest open with new positive conflict evidence:** regardless of armed,
  queued, unarmed, or required-check state, create and verify retirement intent,
  then perform guarded deletion/close and generate a successor.
- **Latest open, armed or queued, without positive conflict:** return
  `Existing`; healthy, pending, delayed, unknown, and failed-check attempts are
  not replaced.
- **Latest open, unarmed generated successor without positive conflict:**
  validate its exact PR/branch/provenance/tree and required-check state. If its
  base is current, arm and verify it only when required checks are pending or
  green. If its base is stale while checks are pending or green, record an exact
  App-authored publication-abort intent, lease-delete, close, and regenerate;
  this completes a create/arm transaction that never published an armed attempt
  and does not require conflict evidence. Failed required checks remain visible
  and unarmed.
- **Latest closed with exact publication-abort intent:** validate the App
  identity, deterministic candidate provenance/tree, and the intent's recorded
  base-staleness proof. If the branch remains at the exact candidate head, retry
  only leased deletion; if absent, regenerate. No conflict proof is required
  because the candidate never reached the attempt linearization point.
- **Latest closed at stale head:** require matching App-authored retirement
  intent and reproduce its recorded strict-ancestry and exact-object local
  conflict proof. If the branch remains at that stale head, retry only leased
  deletion; if absent, regenerate. A merely closed healthy PR—or a close without
  controller intent—does not authorize deletion or replacement.
- **Stable branch at an orphan generated successor:** require validated
  provenance and deterministic tree equality tied to the exact closed stale PR
  and its App-authored retirement intent. If its base is still current, create
  and arm the exact candidate; if `main` advanced, lease-delete only that
  validated orphan and regenerate.
- **Latest merged:** an absent branch permits ordinary generation; a branch
  still at the merged head may be lease-deleted as completed-attempt cleanup;
  any other branch state fails closed.
- **No attempt and no branch:** ordinary generation. Any unmarked, malformed,
  mismatched, ambiguous, or differently-owned PR/branch state fails closed.

Intent/comment, delete, close, generate, push, create, and arm failures remain
visible in PR/ref/timeline state and command output. A later authorized Generate
run re-reads and resumes only one of the states above. Ambiguous transport
failures are classified by exact postcondition reads before retry.

## Fresh successor publication

After retirement, the controller fetches and detaches fresh `main`, promotes
every pending tracked draft through existing deterministic slug-ordered
`adr::run_promote`, formats, and commits with provenance.

Immediately before the first remote write, it fetches `main` again and requires
the candidate's sole parent/base trailer to equal the observed tip. If main
changed, it discards the unpublished candidate and regenerates, with a small
fixed attempt bound before a visible failure. After an ambiguous or successful
branch push and before PR creation, it re-reads both main and the stable ref; a
newly stale but fully validated orphan is lease-deleted and regenerated rather
than published.

Creation discovers a successor only by exact body marker, App author, owner,
base, branch, and generated head SHA. After creating or rediscovering that exact
unarmed PR and before arming it, the controller re-reads `main` and the stable
ref. If the candidate base is no longer current, it records and verifies an
App-authored publication-abort intent, performs exact leased deletion followed
by exact close, and regenerates within the fixed attempt bound. Otherwise that
post-creation read is the publication linearization point: it arms auto-merge,
then verifies unchanged head plus auto-merge/queue state. A main advance after
that point is a new Generate event. For this state machine, the generated PR
becomes a promoter attempt only at that linearization point. Before it is
observed current and armed, it is an incomplete publication artifact; aborting
it is not replacement of a behind attempt.

Success reports stale and successor PR identities; interrupted resumption is
separately observable.

Duplicate generation events converge because their workflow group is serialized;
a later run observes the verified successor as `Existing`. Duplicate dequeue
events cannot enter replacement. A successor with deterministic check failures
remains visible and immutable; it is replaced only after a later main advance
produces new positive conflict evidence.

## Interfaces and implementation boundary

Deepen `xtask/src/pr/promoter.rs` around typed generation state rather than
exposing raw API steps to orchestration:

- typed latest-attempt state, authenticated App identity, mergeability,
  parent/current-main relation, local conflict proof, and required-check state;
- parsed retirement intent and generated-commit provenance plus
  deterministic-tree verification;
- explicit append-intent, close-PR, and leased-delete capabilities;
- one exact publication/arm path reused by fresh creation, replacement, and
  interrupted-run resumption.

Extend GitHub JSON parsing only at its existing boundary. Keep raw API values
out of controller decisions.

## Decision records and guidance

- Add a tracked proposed ADR draft recording that an immutable promoter PR is
  one replaceable attempt, the dual conflict proof, App-authored retirement
  intent, leased retirement, generated-tree-verified provenance, and
  generation-only serialization.
- Add a short past-tense history pointer to ADR-0152; do not rewrite its
  Decision. Project both decisions into `docs/ARCHITECTURE.md` with the
  draft-path citation.
- Update `CONTRIBUTING.md` and authoritative `jaunder-adr`/`jaunder-ship` skill
  sources: operators diagnose visible failures and rerun the controller; they
  never close/delete/rebase/promote manually. The controller alone performs
  guarded replacement.
- `CONTEXT.md` and generated `docs/README.md` are unchanged. This introduces no
  domain vocabulary; the promoter owns ADR index generation after merge.

## Verification

- Controller tests cover dual positive conflict/base-advance proof; exact App
  identity and retirement/publication-abort intent validation; no replacement
  for unknown, blocked, pending, queue delay, deterministic check failure,
  externally closed healthy PR, or spoofed intent; exact identity rereads;
  duplicate convergence; pre- and post-create main advancement; current and
  stale unarmed-successor resume; latest merged states; every failure boundary
  after intent/retirement; and recovery from stale, absent, validated generated,
  and foreign branch states.
- Existing real promotion tests continue to prove current-main numbering,
  complete pending-draft inclusion, deterministic slug order, status/citation
  rewrite, and index regeneration.
- Temp Git repositories prove exact-object merge conflict classification,
  deterministic reconstructed-tree equality, leased deletion success/refusal,
  and non-force recreation.
- Focused xtask tests, `cargo xtask check --no-test`, dual standards/spec plus
  security review, commit/pre-push gates, and PR CI pass.
- Authoritative skill changes are committed on the current agent-configuration
  branch without push, then `refresh-agent-config jaunder` and `--check jaunder`
  report every worktree current.

## Non-goals

- Replacing a merely failing, pending, queued, unknown, behind, or externally
  closed healthy armed promoter PR. An unarmed generated candidate found stale
  before the create/arm linearization point is aborted as incomplete
  publication.
- Retrying deterministic CI failures unattended.
- Changing ADR allocation/promotion mechanics, merge-queue policy, App
  permissions, or workflow triggers.
- General-purpose PR transaction storage or recovery.
