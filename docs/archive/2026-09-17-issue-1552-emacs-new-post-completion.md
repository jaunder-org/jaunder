# Issue #1552 — Idiomatic Emacs New-Post Completion

## Outcome

A Post editing buffer opened by `jaunder-new-post` behaves like an ordinary
Emacs transient input buffer: `C-c C-c` completes the operation, while `C-c C-k`
abandons it.

## Load-bearing decisions

- `C-c C-c` publishes the Post using the existing `jaunder-publish` behavior.
- After a successful publish, the editing buffer closes.
- A failed publish leaves the buffer open with its content intact so the user
  can correct or retry it.
- `C-c C-k` abandons the new Post without making any server request, deletes its
  local draft file, and closes the editing buffer.
- Abandonment deletes the local draft even when the user previously saved edits
  with ordinary Emacs save commands.
- The completion and abandonment bindings are buffer-local behavior established
  only for buffers opened by `jaunder-new-post`.
- Existing Post files, pulled Posts, and unrelated Org buffers retain their
  normal key bindings.
- Ordinary file saving and direct invocation of `jaunder-publish` remain
  available independently.

## Acceptance

- Both ordinary and prefix invocations of `jaunder-new-post` install the
  new-Post editing behavior in the visited draft buffer.
- In such a buffer, `C-c C-c` publishes and closes the buffer only after a
  successful response.
- A publish error from `C-c C-c` preserves the live buffer, its content, and its
  local file.
- In such a buffer, `C-c C-k` makes no transport request, removes the draft file
  even after an explicit save, and closes the buffer.
- After both ordinary and prefix `jaunder-new-post` creation paths have been
  exercised, ordinary Org buffers and existing Jaunder Post buffers retain their
  original `C-c C-c` and `C-c C-k` bindings.
- Focused ERT coverage proves the completion, failed-completion, cancellation,
  and binding-scope behavior.

## Boundaries

- This work does not change AtomPub request or write-back semantics.
- It does not add completion or cancellation bindings to arbitrary existing Post
  files.
- It does not change Org mode's global keymap or redefine ordinary save
  commands.
- It does not introduce a general-purpose Post editing mode beyond the new-Post
  input lifecycle.
