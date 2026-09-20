# Issue #1600: Mobile web photo capture

## Outcome

An authenticated User can take a photo from Jaunder's shared Media upload
control on a mobile browser and upload it immediately, while retaining the
existing ability to choose any supported file. Desktop and mobile browsers that
do not honor capture hints still present a usable file picker.

## Load-bearing decisions

- The shared upload control presents two distinct actions: **Take photo** and
  **Choose file**. Home, the full Post composer, and `/media` use that same
  control rather than implementing separate capture flows.
- **Take photo** uses a photo-only file input with `accept="image/*"` and the
  rear-camera hint `capture="environment"`. These attributes are hints to the
  browser and operating system; Jaunder does not own camera selection or
  permission UI.
- **Choose file** remains unrestricted. It continues to admit non-image Media,
  including audio and video, subject to the existing server policy.
- A selected or captured file uploads immediately through the existing multipart
  upload lifecycle. This work adds no staging or confirmation step.
- Closing either picker without a selection is a silent no-op: no request,
  progress state, result, or error is produced. After handling a selection, the
  control permits the same file to be selected again.
- Both actions share upload progress, disabled state, success and error
  callbacks, and indeterminate-commit handling. Neither path creates a second
  upload policy or transport.
- The site-wide Media Upload Capability remains the discovery and presentation
  policy for both actions. The Media manager remains authoritative for direct
  requests, maximum file size, User quota, and capability enforcement.
- The actions have distinct accessible names and remain operable when the
  capture hint is unsupported.
- Manual device acceptance uses a temporary LAN-reachable instance on
  `vetinari.local` with Media uploads enabled and a unique disposable
  non-operator User. Fixed development credentials are not exposed.

## Acceptance

- Home (`/app`), the full Post composer (`/posts/new`), and `/media` expose
  **Take photo** and **Choose file** wherever the existing upload control is
  available.
- The **Take photo** input has exactly `accept="image/*"` and
  `capture="environment"`; the **Choose file** input has no `accept` attribute
  and no `capture` attribute.
- Selecting through either action drives the same uploading state and existing
  outcome callbacks. The actions are disabled consistently while an upload is in
  progress and settle through the existing confirmed, failed, and indeterminate
  outcomes.
- Cancelling either picker sends no upload and shows no upload result or error.
  After each settled outcome, selecting the same file again remains possible.
- Automated browser coverage exercises the shared control through a Post
  composer and `/media`, including successful selection, cancellation, the
  unrestricted general-file path, shared failure and indeterminate behavior, and
  same-file reselection after every settled outcome.
- Desktop browser coverage uses **Take photo** as an ordinary image chooser and
  completes an upload when the capture hint is ignored.
- Existing automated coverage continues to prove that disabled Media uploads
  withhold both controls, direct uploads are rejected, and existing Media stays
  readable and deletable.
- Before manual acceptance, the tested commit passes full `cargo xtask validate`
  or all required CI checks, and a prepared `vetinari.local` instance from that
  same commit is supplied with its exact URL, generated username, and generated
  password.
- Current iOS Safari and Android Chrome are each manually checked for opening
  capture UI, cancelling without an upload, completing a captured-photo upload,
  and retaining a usable **Choose file** path. The rear-facing preference is
  verified where the browser honors it; chooser or fallback behavior is recorded
  rather than treated as a Jaunder guarantee.
- A PR comment records each manual check's timestamp, device model, OS and
  browser versions, tested commit, exact URL without credentials, and pass/fail
  observation for every step.
- Visual proof compares the merge-base with the tested commit on `/app`,
  `/posts/new`, and `/media` at 390×844, plus `/media` at 1440×900. The PR links
  the before/after captures and identifies both revisions.
- After both device results are recorded, the temporary instance, User
  credentials, database, and uploaded Media are removed; teardown confirmation
  is recorded in the PR before merge.

## Boundaries

- No custom `getUserMedia` recorder, camera chooser, or permission interface.
- No image preview, cropping, editing, compression, or transcoding.
- No video or audio capture action and no restriction of the general Media
  picker to images.
- No change to Post body insertion, Media rendering, sanitizer policy, storage
  schema, upload endpoint, quota, maximum size, or site-wide capability
  semantics.
- No public cloud host, new tunnel provider, permanent deployment target, or
  reusable infrastructure is introduced for manual acceptance.
