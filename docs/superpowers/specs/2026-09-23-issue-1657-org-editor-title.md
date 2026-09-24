# Org editor preserves the Post title (#1657)

## Outcome

Editing an existing Org Post in the web interface exposes its representable
saved title as editable `#+TITLE:` lines. Saving without changing those lines
keeps the title; deliberately removing them can leave the Post titleless. The
canonical stored Org body remains free of recognized metadata.

## Load-bearing decisions

- Reconstruct the saved Post title in the **Org editor's editable source**, not
  in the stored body, the read-only permalink, the AtomPub Member, or the
  new-Post composer. One nonblank title line uses one `#+TITLE:` directive;
  titles formed from multiple nonblank lines use repeated directives, preserving
  their existing Org normalization semantics. A titleless Post has no synthetic
  title line.
- Only the title needs reconstruction in the body: the editor already presents
  summary, tags, audience, slug, publication state/time, and format in dedicated
  controls. Their current structured-field precedence and omission semantics
  remain intact.
- On Org save, the source's `#+TITLE` supplies the title through the existing
  server-side normalization. Removing the line expresses the author's choice to
  clear it, subject to existing title derivation from body headings. Editing the
  line changes the Post title.
- Switching away from Org must not turn a synthetic `#+TITLE` into literal
  content or silently submit a stale synthetic title. The selected format and
  authored body remain authoritative.
- Keep the server's recognized-header stripping, metadata-free canonical
  storage, validation, and existing behavior for unknown Org directives. Do not
  silently alter a title that cannot be represented by nonblank `#+TITLE:` lines
  (for example, a title with an internal blank line); the policy decision for
  such titles is tracked in #1662. No persistence/schema or protocol change.

## Acceptance

- A titled Org Post opened for editing shows its saved title in editable
  `#+TITLE:` line(s) and its canonical body beneath them; saving unchanged and
  reopening retains the title, including titles composed of multiple nonblank
  lines, without storing those directives in the canonical body.
- Changing title lines and saving updates the displayed/stored title; deleting
  them and saving a body without a heading removes the title. A titleless Org
  Post opens without an invented title. An unrepresentable title does not
  silently change on save.
- Summary, tags, audience, slug, publication state/time, and format continue to
  load from their structured values and survive an ordinary title-preserving
  edit. Unknown Org directives in the body remain untouched.
- Changing an editor's selected format away from Org does not accidentally save
  a synthesized `#+TITLE:` as Markdown content.
- Host-tested source/dispatch behavior and a browser create → edit → save →
  reopen regression demonstrate the fix; existing SQLite/PostgreSQL integration
  coverage of the server Org normalization remains green.

## Boundaries

- No general synthesis of Org metadata beyond `#+TITLE`, no new title-entry
  control, and no change to how new Posts are authored. Whether titles with
  internal blank lines should be allowed, and their eventual editing contract,
  are deferred to #1662.
- No changes to AtomPub or Emacs Protocol Client header synthesis, legacy stored
  bodies, or title derivation rules.
