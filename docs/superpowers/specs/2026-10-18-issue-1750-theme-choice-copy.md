# #1750: theme choices, Copy and automatic names — approved addendum

**APPROVED for implementation — decision JAU-1750-COPY-NAMES-001.** The approved
installed-preview contract remains valid; this addendum supersedes only its
Copy/naming exclusions. BuiltinCSS001 remains withdrawn.

**Approval provenance:** In response to the Lead's request, “May I implement the
reviewed Copy/numbering plan? Copy saved content current-library/private
unused/permitted images else whole Copy reject; allocate Name 2/3 without
overwrite/shorten only generated names as needed. A yes approves implementation
and plan—not merging,” the user answered **“yes”**. Lead acknowledgement:
`51e03446-a8b8-4315-a0ce-daa671184145`. Approval covers implementation of this
spec and linked outline, not PR merge. The reviewed source documents were 4609
bytes / SHA256
`afcb1c71584f8bbf25a10aac9e51e81fadd335633629a60f8dcd19e31f1880fd` (spec) and
6411 bytes / SHA256
`747a038c1f75c396cebe89bd4b65047fe984001162e839b8ace1d516c965e437` (outline).

## Write contract

Add
`copy_theme(scope: OwnershipScope, source: PublicThemeSelection) -> WebResult<MutationOutcome<CatalogEntry>>`
in `web/src/themes/api.rs`. `#[macros::server(skip_all)]` derives
`web::themes::CopyTheme` and `POST /api/themes/copy_theme`, following the
vertical's verb-led naming convention.

- Use `owner(scope)`, private/no-store and one operation permit. Author means
  the authenticated User's library; Site requires operator status for source and
  destination. No name, arbitrary owner, URI or digest input.
- Custom source is one coherent snapshot of its latest saved editable package
  and presentation: manifest, authored CSS, assets, logo/header bindings, header
  pool and shuffle seed—not unsaved buffers or the published revision.
  Missing/foreign sources remain masked not-found.
- Preserve Media ownership/provenance and existing reuse authorization. Site
  access does not grant another operator's private Media. If any required
  binding cannot be reused, reject the entire Copy; never silently omit it.
- Built-ins use canonical original package bytes from the verified startup
  system inventory, checked against installed revision/source identity. Compiled
  CSS and preview HTML are not copy sources. Missing/mismatched inventory or
  installed data is an honest error, not fallback.
- Create a new private, unpublished, unselected ID; leave source and selection
  untouched. Follow site theme is inheritance, not a source. No cross-library
  copy is inferred.

## Automatic naming

UI uploads omit `name`; the validated manifest name supplies their base. Copy
uses the source's current catalog name, or canonical built-in name. Catalog
labels remain independent of manifest names.

Trim the base; reject blank or over-100-Unicode-scalar input. Reserved built-in
bases are allowed only as automatic seeds, never final reserved labels. Choose
the base if available and valid; otherwise append ` 2`, ` 3`, etc., using the
lowest available integer under existing owner-local lowercase keys and
reserved-name checks. Do not reinterpret a numeric ending.

For a suffixed automatic candidate, shorten only its stem to 100 minus suffix
scalar length, trim its end and validate the result. Explicit-name APIs never
truncate or reinterpret: invalid/reserved names and exact collisions retain
their current rejection behavior. Strict create and rename semantics otherwise
remain unchanged.

Allocation, admission and insertion share a database serialization boundary with
strict create and rename; the unique index remains the final guard. Return the
actual allocated ID/name, never the requested base.

## Atomicity and interface behavior

Validate/revalidate the source snapshot and Media references, then commit
creation and all copied bindings through one WriteScope transaction under
production quotas and existing Media locks. Separate binding operations are not
an atomic Copy. Preserve existing filesystem/staging guarantees without claiming
cross-store atomicity. CommitIndeterminate remains uncertain: reconcile once,
never automatically replay Copy, publish or select.

Whole-tile Use remains explicit activation; a separate Edit icon opens without
activation. Opt-in editing is the Lead's recommendation, not a human mandate.
Highlight the entire In-use surface with accessible pressed/name semantics;
remove the visible badge and “No saved themes.” Upload and Copy are labelled
peer action tiles; Copy chooses its source before writing. Avoid nested buttons.
Current custom Use must still apply saved edits.

No thumbnail generation, renderer/package/schema expansion, new Media grant or
Save-autoapply. Implementation is authorized under JAU-1750-COPY-NAMES-001;
current-source proof and genuine visual acceptance remain separate.
