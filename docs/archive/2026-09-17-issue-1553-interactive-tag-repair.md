# Issue #1553: Interactive Tag repair in `jaunder-new-post`

## Outcome

`jaunder-new-post` refuses an invalid Tag without abandoning Post creation or
discarding the user's input. The Tag prompt explains the concrete problem and
lets the user repair the rejected text in place.

## Load-bearing decisions

- Tag validation occurs when the user submits each Tag during ordinary
  `jaunder-new-post` metadata collection.
- Invalid input remains exactly as entered in the next Tag prompt rather than
  being cleared. The rejection is carried by that prompt, so beginning the next
  edit cannot overwrite it.
- Validation retains the existing trim semantics: leading and trailing
  whitespace is ignored when deciding validity and storing a valid Tag. When an
  otherwise-invalid value is offered back, its untrimmed text is preserved.
- Invalid input has exactly two error classes:
  - an invalid first non-whitespace character is reported as requiring an ASCII
    letter or digit at the start;
  - an invalid later character is reported by character and one-based position,
    with the reminder that subsequent characters allow only ASCII letters,
    digits, or hyphens.
- The repair prompt places the cursor immediately before the first offending
  character in the preserved, untrimmed text. Cursor placement therefore counts
  any ignored leading whitespace even though validation does not.
- Author casing remains accepted and preserved while Tag identity remains
  case-insensitive.
- A rejected Tag is not accepted, silently dropped, or allowed to advance the
  command to publication-state collection.
- Correcting the text and submitting it continues the existing repeated-Tag
  collection flow.
- Empty input still finishes Tag collection, and quitting still cancels the
  command without creating a local Post.
- Existing completion candidates, canonical duplicate handling, accepted-Tag
  order, and first-entered casing remain unchanged.

## Acceptance

- Interaction tests establish these binary rejection cases:
  - `-topic` reports the invalid-start class, preserves `-topic`, and places the
    cursor before `-` at zero-based offset 0;
  - `two words` reports the internal space as character 4, preserves the input,
    and places the cursor before the space at zero-based offset 3;
  - `tag!` reports `!` as character 4 and places the cursor at offset 3;
  - `café` reports `é` as character 4 and places the cursor at offset 3;
  - `  two words  ` preserves the surrounding whitespace and places the cursor
    before the internal space at offset 5, proving validation-after-trim and
    cursor-in-original-input semantics.
- At least one rejection interaction repairs the preserved input and continues
  through successful Post creation without restarting `jaunder-new-post`.
- The created Post's `#+KEYWORDS` contains the corrected Tag and never the
  rejected form.
- Existing completion, duplicate, cancellation, and valid free-text Tag tests
  remain green.

## Boundaries

- This change applies only to interactive Tag collection in `jaunder-new-post`.
- It does not change Tag grammar, AtomPub, server or web validation, or Tag
  completion discovery.
- It does not add validation for manually edited `#+KEYWORDS` during publish.
- It does not change title, publication-state, scheduling, or prefix-argument
  behavior.
