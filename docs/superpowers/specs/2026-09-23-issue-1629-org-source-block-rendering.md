# Issue #1629 — Org source-block rendering, first pass

## Outcome

A published Org Post containing a `#+begin_src … #+end_src` block shows its
source as readable, escaped, **visually distinct** preformatted code on its
public permalink, Local, and Home, and as preformatted code in the rendered HTML
carried by its public Syndication Feeds. This first pass addresses the reported
symptom: the code is present but looks like ordinary prose when logged out.
Syntax highlighting is desirable but not required for this pass.

## Load-bearing decisions

- A source block is author content, not an instruction to execute or interpret
  the code. Preserve its line breaks, indentation, and visible characters while
  preventing authored markup from executing in a reader's browser.
- Render the same stored Org Post consistently whether it came from the web
  composer or Emacs synchronization. Do not make rendering depend on the
  authoring client.
- A language identifier may be retained as safe metadata for future
  highlighting, but the first-pass proof does not require colorization or a
  particular highlighting library.
- Preserve the existing safe-HTML boundary: no bypass of `RenderedHtml`
  sanitization or enlargement of its active-markup policy to make blocks appear.
- Establish a reproduction of the missing/incorrect source block through the
  real published-Post path before choosing the fix. If the reported symptom
  cannot be reproduced, document what was checked and seek a representative
  failing source rather than inventing an unrelated renderer change.
- After the first pass, revisit highlighting with the user in light of the
  observed cause and cost; this spec does not silently authorize a highlighting
  implementation or pre-empt that decision.

## Acceptance

- A regression proof demonstrates that a multiline Org source block with
  indentation and markup-like text reaches the published Post as readable
  preformatted code, with no executable markup; the proof fails on the diagnosed
  defect before the fix and passes afterward.
- The same published Post has a visibly distinct code block on its public
  permalink, Local, and Home, including a narrow viewport where long lines
  remain accessible without widening or clipping the page. Capture comparable
  before/after views on the public permalink and Home with the same fixture and
  viewport.
- The same sanitized preformatted code appears in the rendered HTML of that
  Post's public Atom, RSS, and JSON Syndication Feeds; those surfaces need not
  implement separate rendering logic.
- The first-pass behavior holds for both web-created and Emacs-synchronized Org
  Posts; authored source remains round-trippable as Org rather than being
  replaced by rendered HTML.
- Existing Markdown fenced-code and Org non-source content continue to render
  and sanitize as before. Relevant focused tests and the applicable repository
  verification ladder pass.
- Report the reproduced cause and what a later highlighting pass would entail,
  so the user can choose whether to extend #1629 or track a separate change.

## Boundaries

- No code execution, syntax highlighting, new language grammar, client-specific
  display path, or unrelated Org export overhaul in this first pass.
- Do not change Post source semantics, publication authorization, or unrelated
  metadata normalization to address a presentation defect.
