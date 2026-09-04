# Site-wide Media Upload Capability Implementation Outline

> Execute with `jaunder-iterate`, delegating through `jaunder-dispatch`. This
> outline exists because the feature changes a shared storage-owned policy seam,
> a public AtomPub discovery/HTTP contract, and coordinated web/admin surfaces.

## Scope

In:

- One typed site-config capability shared by web and AtomPub uploads.
- Entry-snapshot enforcement, typed `403` projection, and no-side-effect proof.
- AtomPub Service Document discovery, `/media`, and `/admin/site` projections.
- SQLite/PostgreSQL, manager, web, AtomPub, and focused browser coverage.
- Operator, domain, architecture, and proposed-ADR documentation.

Out:

- Per-user or per-protocol policy, in-flight cancellation, and new admin routes.
- Changes to byte limits, quotas, media addressing, retrieval, or deletion.
- Process flags, environment variables, legacy-zero compatibility, or migration.

## Task outline

- [x] Task 1: Establish the configuration and authoritative manager policy
  - Contract: the closed registry adds `media.uploads_enabled`.
    `SiteConfigStorage::get_media_uploads_enabled(&self) -> sqlx::Result<bool>`
    reads absence as `true` and malformed physical data as `false`;
    `SiteConfigStorage::set_media_uploads_enabled` takes
    `(&mut WriteTransaction, bool)` and returns `sqlx::Result<()>`, writing the
    independent value. Both manager upload entry points return
    `MediaError::UploadsDisabled` before polling supplied content or reading
    quota/upload storage. The manager reads the value once per attempt;
    already-admitted uploads do not recheck it.
  - Verification: registry validation and accessor behavior cover exact
    booleans, absent/invalid state, and both backends; explicit regressions
    prove zero maximum-file-size and quota values remain invalid/defaulted
    rather than disabling uploads. Manager tests cover enabled and disabled
    stream/byte paths, non-polling and no downstream storage work, and the
    in-flight snapshot. Upload metrics classify the disabled outcome without
    reporting success. The operator/configuration documentation, `CONTEXT.md`,
    architecture projection, and proposed ADR describe this same policy.

- [x] Task 2: Project policy through authoritative web and AtomPub boundaries
  - Depends on: Task 1's accessor and `MediaError::UploadsDisabled` contract.
  - Contract: the web upload boundary and AtomPub error ladder map the typed
    error to client-safe `403 Forbidden` with `media uploads are disabled`. The
    AtomPub Service Document includes the Media Collection only when the typed
    setting is enabled; direct collection POST still reaches manager
    enforcement. Media GET and DELETE behavior does not consult the upload
    capability.
  - Verification: dual-backend web and AtomPub tests cover disabled `403`, exact
    safe message, conditional discovery, direct-URI authority, no temporary
    upload file or durable media mutation, and successful retrieval/deletion
    while disabled; enabled and absent behavior preserve existing upload
    contracts.

- [x] Task 3: Add the independent operator control
  - Depends on: Task 1's typed site-config boundary.
  - Contract: `/admin/site` adds a separately loaded and saved Media Uploads
    card whose checkbox writes only the capability. Its read/write server
    functions require operator authority and use the existing mutation-feedback
    contract; site identity remains a separate payload and action.
  - Verification: operator/member server-function tests prove authorization,
    round-trip behavior, and independent writes. The focused
    `admin-site.spec.ts` flow disables and re-enables uploads and confirms site
    identity is unchanged.

- [x] Task 4: Project read-only state through the media page
  - Depends on: Task 1's typed site-config boundary and Task 2's server
    authority.
  - Contract: `/media` remains available and continues rendering usage and
    existing media. When disabled it renders
    `Media uploads are disabled by the site operator.` and does not render
    upload controls; this client visibility is advisory.
  - Verification: a host-tested presentation decision covers
    loading/error/enabled/disabled states. The focused `media.spec.ts` flow
    proves the read-only notice, absence of upload controls, direct-call
    authority, existing-media retrieval/deletion, and unchanged enabled uploads.

## Risk checks

- The shared manager owns enforcement; transport adapters and CSR components do
  not grow independent policy matches that can drift.
- The setting is read before any upload content/storage work, but only once; no
  cancellation, repeated config reads, or temporary-file cleanup race is added.
- Invalid stored configuration fails closed without swallowing database errors.
- Both public boundaries preserve typed error classification and expose no
  internal source text.
- AtomPub discovery omission never substitutes for direct-request enforcement.
- Existing-media retrieval/deletion remains available while disabled across both
  transports and storage backends.
- Site identity and media capability use independent read/write actions and
  mutation feedback; saving either cannot overwrite the other.
- Production Rust paths preserve owner-qualified free functions/constants; no
  lint suppression is introduced without explicit approval, and commits carry no
  `Co-Authored-By` trailer.
