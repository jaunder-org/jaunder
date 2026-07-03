# Emacs Publish Flow (C4 / #162) Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the shipped C1/C2/C3 seams into the Emacs authoring lifecycle —
`jaunder-new-post` / `jaunder-publish` / `jaunder-save-draft` with multi-blog
config, safe-to-resume send, `JAUNDER_ID`-first write-back, and
temp→`<slug>.org` rename.

**Architecture:** Pure buffer/parse/config helpers added to `elisp/jaunder.el`
and unit-tested in `elisp/test/jaunder-test.el`; the three interactive commands
orchestrate them, resolving the target blog from the buffer's directory and
dynamically `let`-binding the existing `jaunder-base-url`/`jaunder-username`
transport specials (ADR: `docs/adr/0046-emacs-publish-orchestration.md`, held
uncommitted until ship — see Task 1). End-to-end behavior is covered by live ERT
against a real server (`jaunder-test--with-live-server`, ADR-0035).

**Tech Stack:** Emacs Lisp (floor 29.1), `org`/`dom`/`libxml`, `plz` transport
(already present); ERT (pure `*-test.el`, live `*-integration.el`);
`cargo xtask` gate.

## Global Constraints

- **Emacs floor `29.1`** (`Package-Requires`, ADR-0042). Use only built-ins +
  `plz` (already a dep); add no new dependency.
- **The authoring buffer's body is never rewritten** by publish (media
  substitution is sent-body-only — C3 invariant).
- **Server contract (verified):** create `POST /atompub/{user}/posts` → `201` +
  `Location: {base}/atompub/{user}/posts/{post_id}` (numeric) + `ETag`; update
  `PUT /atompub/{user}/posts/{post_id}` honors `If-Match` (stale → `412`) →
  `200` + `ETag`; `<j:slug>` on every entry
  (`xmlns:j="https://jaunder.org/ns/atompub"`); `<published>` only when live.
- **Request Content-Type for entries:** `application/atom+xml;type=entry`.
- **ADR-0023:** the client never emits `j:slug` (read-only server value); it
  only reads it back.
- **No `Co-Authored-By` trailer.** One clean commit per task. Pre-commit runs
  the full `cargo xtask check`.
- **Pure tests** live in `elisp/test/jaunder-test.el` (globbed by `*-test.el`);
  **live tests** in `elisp/test/jaunder-publish-integration.el` (globbed by
  `*-integration.el`). No gate wiring needed.
- **Fast inner loop:** `emacs --batch -Q -l elisp/scripts/run-tests.el` (pure
  suite). **Commit gate:** `cargo xtask check`. **Live suite:**
  `cargo xtask elisp-integration`.

---

### Task 1: Design docs landed + follow-on issues filed

**Files:**

- Working-tree only (held uncommitted until ship):
  `docs/adr/0046-emacs-publish-orchestration.md`, `docs/README.md` (generated
  table row)
- Commit now: the spec
  (`docs/superpowers/specs/2026-07-03-issue-162-emacs-publish-flow.md`) and this
  plan. **Not** the ADR — see Step 2.

**Separable concerns → GitHub issues (file before coding; do not fold into
C4):**

- [x] **Step 1: File the three follow-on issues** (`jaunder-issues` conventions,
      milestone _Emacs blogging front-end_):
  1. **Interactive `jaunder-new-post` variant** — prompt for
     title/tags/format/status and pre-fill the template (Unit C UX enhancement;
     blocked by #162). → **#215** (Feature)
  2. **Vanilla-Jaunder `format-media-type` warning** — warn at publish when the
     server's service document does not advertise the `format-media-type`
     feature (per-entry `text/org` may be ignored). Needs a service-doc
     fetch/cache. (Epic-spec edge case; blocked by #162.) → **#216** (Task)
  3. **Multi-machine timezone-mismatch warning** — warn at publish when the
     machine's current zone differs from a recorded `JAUNDER_DATE_TZ` (epic-spec
     "Publish time" multi-machine guard; blocked by #162). → **#217** (Task)

- [x] **Step 2: Assign the ADR number in the working tree, held uncommitted**

`cargo xtask adr renumber` only acts on a **committed** ADR addition (it diffs
`merge-base(origin/main, HEAD)..HEAD`), so it cannot renumber an uncommitted
draft. To keep the ADR out of history until land — its final number is likely to
move again if more ADRs merge first — the renumber is replicated by hand and
left uncommitted: `git mv 0000-…` → `0046-…` (max+1), rewrite the `# ADR-0000:`
heading → `# ADR-0046:`, and `cargo xtask adr sync-readme` to fold in the README
table row. Then unstage the ADR + `docs/README.md` (`git reset`). They persist
as working-tree changes through Tasks 2–10; the **authoritative** `adr renumber`
(and the ADR/README commit) happen at ship, once the ADR is committed and the
real max is known.

The `0046` ADR (unique number) + its README row keep the working-tree gate
(`adr-format` + `adr-readme-parity`, both working-tree-based) green on every
subsequent commit, without the ADR entering any commit.

- [x] **Step 3: Commit the spec + plan only** (one clean commit; ADR/README stay
      uncommitted)

```bash
git add docs/superpowers/specs/2026-07-03-issue-162-emacs-publish-flow.md \
        docs/superpowers/plans/2026-07-03-issue-162-emacs-publish-flow.md
cargo xtask check   # working-tree ADR (0046) + README row must keep the gate green
git commit -m "docs(issue-162): C4 publish-flow spec + plan"
```

---

### Task 2: Buffer read/write helpers

**Files:**

- Modify: `elisp/jaunder.el` (new pure helpers, after the media section, before
  `jaunder--atom->org`)
- Test: `elisp/test/jaunder-test.el`

**Interfaces:**

- Consumes: `jaunder--header-keyword-re`, `jaunder--collect-properties`
  (existing).
- Produces: `jaunder--set-property (key value)`,
  `jaunder--set-keyword (keyword value)`,
  `jaunder--buffer-property (key) → string|nil`,
  `jaunder--buffer-keyword (key) → string|nil`. Used by Tasks 8/10.

- [x] **Step 1: Write the failing tests**

```elisp
(ert-deftest jaunder-set-property-replaces-existing ()
  (with-temp-buffer
    (org-mode)
    (insert "#+TITLE: T\n#+PROPERTY: JAUNDER_ID 7\n\nBody.\n")
    (jaunder--set-property "JAUNDER_ID" "42")
    (should (equal (jaunder--buffer-property "JAUNDER_ID") "42"))
    (should (string-match-p "Body\\." (buffer-string)))
    (should-not (string-match-p "JAUNDER_ID 7" (buffer-string)))))

(ert-deftest jaunder-set-property-inserts-into-header-block ()
  (with-temp-buffer
    (org-mode)
    (insert "#+TITLE: T\n\nBody.\n")
    (jaunder--set-property "JAUNDER_SLUG" "my-post")
    (should (equal (jaunder--buffer-property "JAUNDER_SLUG") "my-post"))
    ;; Inserted in the header block, body untouched.
    (should (string-match-p "\\`#\\+TITLE: T\n#\\+PROPERTY: JAUNDER_SLUG my-post\n\nBody\\."
                            (buffer-string)))))

(ert-deftest jaunder-set-keyword-replaces-and-inserts ()
  (with-temp-buffer
    (org-mode)
    (insert "#+TITLE: T\n\nBody.\n")
    (jaunder--set-keyword "DATE" "[2026-07-01 Wed 09:00]")
    (should (equal (jaunder--buffer-keyword "DATE") "[2026-07-01 Wed 09:00]"))
    (jaunder--set-keyword "DATE" "[2027-01-01 Fri 00:00]")
    (should (equal (jaunder--buffer-keyword "DATE") "[2027-01-01 Fri 00:00]"))))
```

- [x] **Step 2: Run the tests, verify they fail**

Run: `emacs --batch -Q -l elisp/scripts/run-tests.el` Expected: FAIL —
`jaunder--set-property` / `jaunder--set-keyword` / `jaunder--buffer-property` /
`jaunder--buffer-keyword` undefined.

- [x] **Step 3: Write the minimal implementation**

```elisp
(defun jaunder--set-keyword-line (line-re new-line)
  "Replace the first LINE-RE match in the leading header block with NEW-LINE.
When absent, insert NEW-LINE after the last contiguous header-keyword line
\(before any blank line or the body).  Header block only; the body is never
touched."
  (save-excursion
    (goto-char (point-min))
    (let ((case-fold-search t)
          (limit (jaunder--body-start)))
      (if (re-search-forward line-re limit t)
          (progn (beginning-of-line)
                 (delete-region (point) (line-end-position))
                 (insert new-line))
        (goto-char (point-min))
        (let ((insert-at (point-min)))
          (while (looking-at-p jaunder--header-keyword-re)
            (forward-line 1)
            (setq insert-at (point)))
          (goto-char insert-at)
          (insert new-line "\n"))))))

(defun jaunder--set-property (key value)
  "Set the file-level #+PROPERTY: KEY to VALUE (idempotent replace or insert)."
  (jaunder--set-keyword-line
   (format "^[ \t]*#\\+PROPERTY:[ \t]+%s\\(?:[ \t].*\\)?$" (regexp-quote key))
   (format "#+PROPERTY: %s %s" key value)))

(defun jaunder--set-keyword (keyword value)
  "Set the file-level #+KEYWORD: to VALUE (idempotent replace or insert)."
  (jaunder--set-keyword-line
   (format "^[ \t]*#\\+%s:.*$" (regexp-quote keyword))
   (format "#+%s: %s" keyword value)))

(defun jaunder--buffer-property (key)
  "Return the #+PROPERTY: KEY value in the current buffer, or nil."
  (cdr (assoc key (jaunder--collect-properties
                   (org-collect-keywords '("PROPERTY"))))))

(defun jaunder--buffer-keyword (key)
  "Return the #+KEY: value in the current buffer, or nil."
  (cadr (assoc key (org-collect-keywords (list key)))))
```

- [x] **Step 4: Run the tests, verify they pass**

Run: `emacs --batch -Q -l elisp/scripts/run-tests.el` Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add elisp/jaunder.el elisp/test/jaunder-test.el
cargo xtask check
git commit -m "feat(emacs): header-block property/keyword set + read helpers"
```

---

### Task 3: Harvest `slug` + `published` from an entry

**Files:**

- Modify: `elisp/jaunder.el` (extend `jaunder--atom-entry-fields`)
- Test: `elisp/test/jaunder-test.el`

**Interfaces:**

- Produces: `jaunder--atom-entry-fields (xml)` now also returns `(slug . …)` and
  `(published . …)` alongside the existing `content-src`/`content-type`. Used by
  Task 8.

- [x] **Step 1: Write the failing test** (pins the `<j:slug>` namespace-prefix
      parse on server-shaped XML)

```elisp
(ert-deftest jaunder-atom-entry-fields-harvests-slug-and-published ()
  (let ((xml (concat
              "<entry xmlns=\"http://www.w3.org/2005/Atom\""
              " xmlns:j=\"https://jaunder.org/ns/atompub\">"
              "<content type=\"text/org\">Body</content>"
              "<published>2026-07-01T13:00:00+00:00</published>"
              "<j:slug>my-post</j:slug></entry>")))
    (let ((fields (jaunder--atom-entry-fields xml)))
      (should (equal (cdr (assq 'slug fields)) "my-post"))
      (should (equal (cdr (assq 'published fields)) "2026-07-01T13:00:00+00:00"))
      (should (equal (cdr (assq 'content-type fields)) "text/org")))))
```

- [x] **Step 2: Run the test, verify it fails**

Run: `emacs --batch -Q -l elisp/scripts/run-tests.el` Expected: FAIL —
`slug`/`published` keys absent.

- [x] **Step 3: Extend the implementation**

```elisp
(defun jaunder--atom-entry-fields (xml)
  "Parse AtomPub entry XML into an alist of harvested fields.
Returns `content-src'/`content-type' from `<content>', `slug' from `<j:slug>',
and `published' from `<published>'.  The shared entry-parse primitive: C3 uses
the content subset, C4 the slug/published subset, Unit D extends it further.
`libxml-parse-xml-region' folds the default namespace, so `<content>' and
`<published>' are `content'/`published'; the `j:'-prefixed slug is matched by
local name via `dom-by-tag' on the `slug' symbol."
  (let* ((dom (with-temp-buffer
                (insert xml)
                (libxml-parse-xml-region (point-min) (point-max))))
         (content (car (dom-by-tag dom 'content)))
         (slug (car (dom-by-tag dom 'slug)))
         (published (car (dom-by-tag dom 'published))))
    (list (cons 'content-src (dom-attr content 'src))
          (cons 'content-type (dom-attr content 'type))
          (cons 'slug (and slug (dom-text slug)))
          (cons 'published (and published (dom-text published))))))
```

> `libxml-parse-xml-region` strips the `j:` prefix, so `<j:slug>` is the `slug`
> symbol (verified) — `(dom-by-tag dom 'slug)` is correct; the test pins it.

- [x] **Step 4: Run the test, verify it passes** (and the C3 `content-src` tests
      still pass)

Run: `emacs --batch -Q -l elisp/scripts/run-tests.el` Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add elisp/jaunder.el elisp/test/jaunder-test.el
cargo xtask check
git commit -m "feat(emacs): harvest j:slug and published from entry XML"
```

---

### Task 4: `jaunder--utc->org-date` + machine-zone capture

**Files:**

- Modify: `elisp/jaunder.el`
- Test: `elisp/test/jaunder-test.el`

**Interfaces:**

- Consumes: `jaunder--resolve-zone`, `jaunder--buffer-property`,
  `jaunder--set-property` (T2).
- Produces: `jaunder--utc->org-date (utc tz) → org timestamp string`;
  `jaunder--current-zone-name () → string`; `jaunder--ensure-date-tz ()`
  (captures the machine zone into `JAUNDER_DATE_TZ` when unset, returns the
  effective zone). Used by Tasks 8/10.

- [x] **Step 1: Write the failing tests**

```elisp
(ert-deftest jaunder-utc->org-date-renders-in-zone ()
  ;; 13:00Z in America/New_York (EDT, -04:00) is 09:00 local.
  (should (equal (jaunder--utc->org-date "2026-07-01T13:00:00Z" "America/New_York")
                 "[2026-07-01 Wed 09:00]"))
  ;; Round-trips through the existing forward mapping.
  (should (equal (jaunder--org-date->utc
                  (jaunder--utc->org-date "2026-07-01T13:00:00Z" "America/New_York")
                  "America/New_York")
                 "2026-07-01T13:00:00Z")))

(ert-deftest jaunder-current-zone-name-is-nonempty ()
  (let ((z (jaunder--current-zone-name)))
    (should (stringp z))
    (should (> (length z) 0))))

(ert-deftest jaunder-ensure-date-tz-captures-when-unset-and-preserves ()
  (with-temp-buffer
    (org-mode)
    (insert "#+TITLE: T\n\nBody.\n")
    ;; Unset → captured to something non-empty.
    (jaunder--ensure-date-tz)
    (let ((captured (jaunder--buffer-property "JAUNDER_DATE_TZ")))
      (should (stringp captured))
      (should (> (length captured) 0))
      ;; Already set → preserved verbatim (idempotent, no re-capture).
      (jaunder--set-property "JAUNDER_DATE_TZ" "Europe/Paris")
      (jaunder--ensure-date-tz)
      (should (equal (jaunder--buffer-property "JAUNDER_DATE_TZ") "Europe/Paris")))))
```

- [x] **Step 2: Run, verify fail**

Run: `emacs --batch -Q -l elisp/scripts/run-tests.el` Expected: FAIL — helpers
undefined.

- [x] **Step 3: Implement**

```elisp
(defun jaunder--utc->org-date (utc tz)
  "Render an org inactive timestamp for UTC interpreted in zone TZ.
UTC is an RFC-3339 UTC string (e.g. \"2026-07-01T13:00:00Z\"); TZ a
JAUNDER_DATE_TZ string.  Inverse of `jaunder--org-date->utc'."
  (format-time-string "[%Y-%m-%d %a %H:%M]"
                      (date-to-time utc)
                      (jaunder--resolve-zone tz)))

(defun jaunder--current-zone-name ()
  "Return the machine's current IANA zone name, else a numeric offset string.
Prefers a `TZ' IANA name, then the /etc/localtime symlink target; falls back to
the current numeric UTC offset (epic spec: IANA preferred, offset caveat)."
  (or (let ((tz (getenv "TZ")))
        (and tz (not (string-empty-p tz)) (not (string-prefix-p ":" tz)) tz))
      (let ((link (ignore-errors (file-symlink-p "/etc/localtime"))))
        (and link (string-match "zoneinfo/\\(.+\\)\\'" link)
             (match-string 1 link)))
      (format-time-string "%z")))

(defun jaunder--ensure-date-tz ()
  "Ensure the buffer records a JAUNDER_DATE_TZ; return the effective zone string.
When unset, captures the machine's current zone (`jaunder--current-zone-name')
so #+DATE: is interpreted in a recorded zone, not one silently re-inferred on a
later machine.  Idempotent: an existing value is preserved verbatim."
  (or (jaunder--buffer-property "JAUNDER_DATE_TZ")
      (let ((zone (jaunder--current-zone-name)))
        (jaunder--set-property "JAUNDER_DATE_TZ" zone)
        zone)))
```

- [x] **Step 4: Run, verify pass**

Run: `emacs --batch -Q -l elisp/scripts/run-tests.el` Expected: PASS. (The zone
test needs a zone database — the hermetic `ert-check` provides `TZDIR`; this
matches ADR-0042's `encode-time` note.)

- [x] **Step 5: Commit**

```bash
git add elisp/jaunder.el elisp/test/jaunder-test.el
cargo xtask check
git commit -m "feat(emacs): render org #+DATE: from UTC + capture machine zone"
```

---

### Task 5: Multi-blog config + resolution + `jaunder--with-blog`

**Files:**

- Modify: `elisp/jaunder.el` (new `defcustom` near `jaunder-username`;
  resolver + macro in the seams area)
- Test: `elisp/test/jaunder-test.el`

**Interfaces:**

- Consumes: the existing `jaunder-base-url`/`jaunder-username` specials
  (fallback path).
- Produces: `jaunder-blogs` (defcustom),
  `jaunder--resolve-blog (file-or-dir) → (:base-url … :username …)`,
  `jaunder--with-blog (file &rest body)` macro. Used by Tasks 9/10.

- [x] **Step 1: Write the failing tests**

```elisp
(ert-deftest jaunder-resolve-blog-longest-prefix ()
  (let ((jaunder-blogs '(("/home/me/blog/" :base-url "https://a" :username "a")
                         ("/home/me/blog/work/" :base-url "https://b" :username "b")))
        (jaunder-base-url nil) (jaunder-username nil))
    (should (equal (plist-get (jaunder--resolve-blog "/home/me/blog/post.org") :username) "a"))
    (should (equal (plist-get (jaunder--resolve-blog "/home/me/blog/work/x.org") :username) "b"))))

(ert-deftest jaunder-resolve-blog-falls-back-to-globals ()
  (let ((jaunder-blogs nil)
        (jaunder-base-url "https://g") (jaunder-username "g"))
    (should (equal (plist-get (jaunder--resolve-blog "/tmp/x.org") :base-url) "https://g"))))

(ert-deftest jaunder-resolve-blog-errors-when-unconfigured ()
  (let ((jaunder-blogs nil) (jaunder-base-url nil) (jaunder-username nil))
    (should-error (jaunder--resolve-blog "/tmp/x.org"))))

(ert-deftest jaunder-with-blog-binds-transport-specials ()
  (let ((jaunder-blogs '(("/home/me/blog/" :base-url "https://a" :username "a")))
        (jaunder-base-url nil) (jaunder-username nil))
    (jaunder--with-blog "/home/me/blog/post.org"
      (should (equal jaunder-base-url "https://a"))
      (should (equal jaunder-username "a")))))
```

- [x] **Step 2: Run, verify fail**

Run: `emacs --batch -Q -l elisp/scripts/run-tests.el` Expected: FAIL —
`jaunder-blogs`/`jaunder--resolve-blog`/`jaunder--with-blog` undefined.

- [x] **Step 3: Implement**

```elisp
(defcustom jaunder-blogs nil
  "Alist mapping a local directory to a Jaunder blog.
Each element is (DIRECTORY . PLIST), where PLIST carries :base-url and
:username (strings) and an optional :format (accepted for forward
compatibility but not used in v1 — org is the only converter)."
  :type '(alist :key-type directory
                :value-type (plist :key-type symbol :value-type string))
  :group 'jaunder)

(defun jaunder--resolve-blog (file-or-dir)
  "Return the active-blog plist (:base-url :username) for FILE-OR-DIR.
Longest-prefix match against `jaunder-blogs'; else the single-blog globals;
else an error naming the directory."
  (let* ((dir (file-name-as-directory
               (expand-file-name (if (file-directory-p file-or-dir)
                                     file-or-dir
                                   (file-name-directory file-or-dir)))))
         (best nil) (best-len -1))
    (dolist (entry jaunder-blogs)
      (let ((root (file-name-as-directory (expand-file-name (car entry)))))
        (when (and (string-prefix-p root dir) (> (length root) best-len))
          (setq best (cdr entry) best-len (length root)))))
    (cond
     (best (list :base-url (plist-get best :base-url)
                 :username (plist-get best :username)))
     ((and jaunder-base-url jaunder-username)
      (list :base-url jaunder-base-url :username jaunder-username))
     (t (error "jaunder: no blog configured for %s" dir)))))

(defmacro jaunder--with-blog (file &rest body)
  "Resolve the blog for FILE and run BODY with the transport specials bound."
  (declare (indent 1) (debug t))
  (let ((blog (make-symbol "blog")))
    `(let* ((,blog (jaunder--resolve-blog ,file))
            (jaunder-base-url (plist-get ,blog :base-url))
            (jaunder-username (plist-get ,blog :username)))
       ,@body)))
```

- [x] **Step 4: Run, verify pass**

Run: `emacs --batch -Q -l elisp/scripts/run-tests.el` Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add elisp/jaunder.el elisp/test/jaunder-test.el
cargo xtask check
git commit -m "feat(emacs): jaunder-blogs directory->blog config + resolver"
```

---

### Task 6: Publish validation + `Location`→id

**Files:**

- Modify: `elisp/jaunder.el`
- Test: `elisp/test/jaunder-test.el`

**Interfaces:**

- Consumes: `jaunder-entry-body`, `jaunder-entry-draft`,
  `jaunder-entry-published`, `jaunder--make-entry`, `jaunder--org-date->utc`
  (existing).
- Produces: `jaunder--validate-publish (entry status date-raw tz)` (errors or
  nil), `jaunder--location->id (location) → string|nil`,
  `jaunder--force-draft (entry)` (mutates: draft=t, published=nil). Used by
  Task 10.

- [x] **Step 1: Write the failing tests**

```elisp
(ert-deftest jaunder-validate-publish-rejects-empty-body ()
  (let ((e (jaunder--make-entry :body "   \n")))
    (should-error (jaunder--validate-publish e "published" nil nil))))

(ert-deftest jaunder-validate-publish-scheduled-needs-future ()
  (let ((e (jaunder--make-entry :body "x")))
    (should-error (jaunder--validate-publish e "scheduled" "[2000-01-01 Sat 00:00]" nil))
    ;; A far-future date passes.
    (should-not (jaunder--validate-publish e "scheduled" "[2999-01-01 Tue 00:00]" nil))))

(ert-deftest jaunder-location->id-extracts-numeric-tail ()
  (should (equal (jaunder--location->id "https://x/atompub/alice/posts/42") "42"))
  (should (equal (jaunder--location->id "https://x/atompub/alice/posts/42/") "42"))
  (should (null (jaunder--location->id nil))))

(ert-deftest jaunder-force-draft-sets-draft-and-clears-published ()
  ;; A dated, non-draft entry forced to draft must not carry <published>:
  ;; the serializer emits <published> whenever the slot is set, independent of
  ;; the draft flag, so force-draft has to nil it (spec invariant).
  (let ((e (jaunder--make-entry :body "x" :draft nil
                                :published "2026-07-01T13:00:00Z")))
    (jaunder--force-draft e)
    (should (jaunder-entry-draft e))
    (should (null (jaunder-entry-published e)))
    ;; And the wire entry indeed omits <published>.
    (should-not (string-match-p "<published>" (jaunder--atom-entry->xml e)))))
```

- [x] **Step 2: Run, verify fail**

Run: `emacs --batch -Q -l elisp/scripts/run-tests.el` Expected: FAIL — helpers
undefined.

- [x] **Step 3: Implement**

```elisp
(defun jaunder--validate-publish (entry status date-raw tz)
  "Signal an error if ENTRY is not publishable; return nil otherwise.
Requires a non-empty body; a `scheduled' STATUS requires a future #+DATE:
\(DATE-RAW interpreted in TZ)."
  (when (string-empty-p (string-trim (or (jaunder-entry-body entry) "")))
    (error "jaunder: refusing to publish an empty body"))
  (when (and status (string= (downcase status) "scheduled"))
    (let ((utc (and date-raw (jaunder--org-date->utc date-raw tz))))
      (unless (and utc (time-less-p (current-time) (date-to-time utc)))
        (error "jaunder: a scheduled post needs a future #+DATE:"))))
  nil)

(defun jaunder--location->id (location)
  "Return the trailing numeric post id from a create `Location' URL, or nil."
  (when (and location (string-match "/\\([0-9]+\\)/?\\'" location))
    (match-string 1 location)))

(defun jaunder--force-draft (entry)
  "Mark ENTRY a server-side draft in place: set `draft', clear `published'.
Clearing `published' keeps `jaunder--atom-entry->xml' from emitting a
`<published>' on a draft (it emits one whenever the slot is set)."
  (setf (jaunder-entry-draft entry) t
        (jaunder-entry-published entry) nil)
  entry)
```

- [x] **Step 4: Run, verify pass**

Run: `emacs --batch -Q -l elisp/scripts/run-tests.el` Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add elisp/jaunder.el elisp/test/jaunder-test.el
cargo xtask check
git commit -m "feat(emacs): publish validation + Location->id + force-draft"
```

---

### Task 7: `jaunder--rename-to-slug`

**Files:**

- Modify: `elisp/jaunder.el`
- Test: `elisp/test/jaunder-test.el`

**Interfaces:**

- Produces: `jaunder--rename-to-slug (slug) → new-path` (renames the current
  buffer's file + buffer; no-op when already named; appends `-N` on collision).
  Used by Task 10.

- [ ] **Step 1: Write the failing tests**

```elisp
(ert-deftest jaunder-rename-to-slug-renames-and-handles-collision ()
  (let ((dir (make-temp-file "jaunder-rn-" t)))
    (unwind-protect
        (let ((tmp (expand-file-name "draft-20260101T000000.org" dir)))
          (with-temp-file tmp (insert "x"))
          (let ((buf (find-file-noselect tmp)))
            (unwind-protect
                (with-current-buffer buf
                  (let ((p (jaunder--rename-to-slug "my-post")))
                    (should (equal (file-name-nondirectory p) "my-post.org"))
                    (should (equal (buffer-file-name) p))
                    (should (file-exists-p p))
                    (should-not (file-exists-p tmp))
                    ;; Idempotent: renaming to the same slug is a no-op.
                    (should (equal (jaunder--rename-to-slug "my-post") p))))
              (kill-buffer buf)))
          ;; Collision: a second post with the same slug gets -1.
          (let ((tmp2 (expand-file-name "draft-20260101T000001.org" dir)))
            (with-temp-file tmp2 (insert "y"))
            (let ((buf2 (find-file-noselect tmp2)))
              (unwind-protect
                  (with-current-buffer buf2
                    (should (equal (file-name-nondirectory
                                    (jaunder--rename-to-slug "my-post"))
                                   "my-post-1.org")))
                (kill-buffer buf2)))))
      (delete-directory dir t))))
```

- [ ] **Step 2: Run, verify fail**

Run: `emacs --batch -Q -l elisp/scripts/run-tests.el` Expected: FAIL —
`jaunder--rename-to-slug` undefined.

- [ ] **Step 3: Implement**

```elisp
(defun jaunder--rename-to-slug (slug)
  "Rename the current buffer's file and buffer to SLUG.org in its directory.
A no-op when already so named; on collision appends `-N'.  Returns the path."
  (let* ((old (or (buffer-file-name)
                  (error "jaunder: buffer is not visiting a file")))
         (dir (file-name-directory old))
         (target (expand-file-name (concat slug ".org") dir)))
    (if (equal old target)
        old
      (let ((final target) (n 1))
        (while (file-exists-p final)
          (setq final (expand-file-name (format "%s-%d.org" slug n) dir)
                n (1+ n)))
        (rename-file old final)
        (set-visited-file-name final nil t)
        final))))
```

- [ ] **Step 4: Run, verify pass**

Run: `emacs --batch -Q -l elisp/scripts/run-tests.el` Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add elisp/jaunder.el elisp/test/jaunder-test.el
cargo xtask check
git commit -m "feat(emacs): rename temp draft to <slug>.org with collision handling"
```

---

### Task 8: `jaunder--write-back`

**Files:**

- Modify: `elisp/jaunder.el`
- Test: `elisp/test/jaunder-test.el`

**Interfaces:**

- Consumes: `jaunder--atom-entry-fields`, `jaunder--response-header`,
  `jaunder--location->id`, `jaunder--set-property`, `jaunder--set-keyword`,
  `jaunder--buffer-property`, `jaunder--buffer-keyword`,
  `jaunder--utc->org-date`.
- Produces: `jaunder--write-back (response created) → slug` — persists
  `JAUNDER_ID` (create only), `JAUNDER_SLUG`, `JAUNDER_SYNCED`,
  `JAUNDER_SYNCED_AT`, and the resolved publish time; saves the buffer. Used by
  Task 10.

- [ ] **Step 1: Write the failing tests** (construct a fake response plist — no
      server)

```elisp
(defun jaunder-test--response (status headers body)
  "Build a `jaunder--http-request'-shaped plist for tests."
  (list :status status
        :headers (mapcar (lambda (h) (cons (downcase (car h)) (cdr h))) headers)
        :body body))

(ert-deftest jaunder-write-back-create-writes-id-first ()
  (with-temp-buffer
    (org-mode)
    (insert "#+TITLE: T\n#+PROPERTY: JAUNDER_STATUS published\n\nBody.\n")
    (set-visited-file-name (make-temp-file "jaunder-wb-" nil ".org") nil t)
    (unwind-protect
        (let ((resp (jaunder-test--response
                     201
                     '(("Location" . "https://x/atompub/alice/posts/42")
                       ("ETag" . "\"abc\""))
                     (concat "<entry xmlns=\"http://www.w3.org/2005/Atom\""
                             " xmlns:j=\"https://jaunder.org/ns/atompub\">"
                             "<content type=\"text/org\">Body</content>"
                             "<published>2026-07-01T13:00:00+00:00</published>"
                             "<j:slug>my-post</j:slug></entry>"))))
          (should (equal (jaunder--write-back resp t) "my-post"))
          (should (equal (jaunder--buffer-property "JAUNDER_ID") "42"))
          (should (equal (jaunder--buffer-property "JAUNDER_SLUG") "my-post"))
          (should (equal (jaunder--buffer-property "JAUNDER_SYNCED") "\"abc\""))
          ;; publish-now (no author #+DATE:) → #+DATE: rendered from server time.
          (should (jaunder--buffer-keyword "DATE")))
      (when (buffer-file-name) (delete-file (buffer-file-name))))))

(ert-deftest jaunder-write-back-update-keeps-id ()
  (with-temp-buffer
    (org-mode)
    (insert "#+TITLE: T\n#+PROPERTY: JAUNDER_ID 7\n#+DATE: [2026-07-01 Wed 09:00]\n\nBody.\n")
    (set-visited-file-name (make-temp-file "jaunder-wb-" nil ".org") nil t)
    (unwind-protect
        (let ((resp (jaunder-test--response
                     200 '(("ETag" . "\"z\""))
                     (concat "<entry xmlns=\"http://www.w3.org/2005/Atom\""
                             " xmlns:j=\"https://jaunder.org/ns/atompub\">"
                             "<content type=\"text/org\">Body</content>"
                             "<j:slug>my-post</j:slug></entry>"))))
          (jaunder--write-back resp nil)     ; created = nil (update)
          (should (equal (jaunder--buffer-property "JAUNDER_ID") "7"))  ; unchanged
          (should (equal (jaunder--buffer-property "JAUNDER_SYNCED") "\"z\"")))
      (when (buffer-file-name) (delete-file (buffer-file-name))))))
```

- [ ] **Step 2: Run, verify fail**

Run: `emacs --batch -Q -l elisp/scripts/run-tests.el` Expected: FAIL —
`jaunder--write-back` undefined.

- [ ] **Step 3: Implement**

```elisp
(defun jaunder--write-back (response created)
  "Persist server-assigned values from RESPONSE into the current buffer.
RESPONSE is a `jaunder--http-request' plist.  CREATED non-nil (a POST) writes
JAUNDER_ID from the `Location' header; an update leaves it unchanged.  Writes
JAUNDER_ID first, then JAUNDER_SLUG, JAUNDER_SYNCED (ETag, verbatim),
JAUNDER_SYNCED_AT (now), and the resolved publish time.  Saves the buffer and
returns the slug."
  (let* ((fields (jaunder--atom-entry-fields (plist-get response :body)))
         (slug (cdr (assq 'slug fields)))
         (published (cdr (assq 'published fields)))
         (etag (jaunder--response-header response "ETag"))
         (now (format-time-string "%Y-%m-%dT%H:%M:%SZ" nil t)))
    (when created
      (let ((id (jaunder--location->id
                 (jaunder--response-header response "Location"))))
        (when id (jaunder--set-property "JAUNDER_ID" id))))
    (when slug (jaunder--set-property "JAUNDER_SLUG" slug))
    (when etag (jaunder--set-property "JAUNDER_SYNCED" etag))
    (jaunder--set-property "JAUNDER_SYNCED_AT" now)
    (when published
      ;; published→UTC (drop the offset): the canonical value the server stamped.
      (let ((utc (format-time-string "%Y-%m-%dT%H:%M:%SZ"
                                     (date-to-time published) t))
            (tz (jaunder--buffer-property "JAUNDER_DATE_TZ")))
        (jaunder--set-property "JAUNDER_DATE_UTC" utc)
        ;; "publish now": no author #+DATE: — render it from the server time.
        (unless (jaunder--buffer-keyword "DATE")
          (jaunder--set-keyword "DATE" (jaunder--utc->org-date utc tz)))))
    (save-buffer)
    slug))
```

- [ ] **Step 4: Run, verify pass**

Run: `emacs --batch -Q -l elisp/scripts/run-tests.el` Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add elisp/jaunder.el elisp/test/jaunder-test.el
cargo xtask check
git commit -m "feat(emacs): ID-first server-value write-back"
```

---

### Task 9: `jaunder-new-post`

**Files:**

- Modify: `elisp/jaunder.el`
- Test: `elisp/test/jaunder-test.el`

**Interfaces:**

- Consumes: `jaunder-blogs`, `jaunder--set-keyword`.
- Produces: `jaunder-new-post ()` (interactive) + pure
  `jaunder--new-post-in (dir now-string) → path`. Used by end users; the pure
  helper is tested here.

- [ ] **Step 1: Write the failing test** (test the pure core; the interactive
      wrapper just resolves the dir + `now`)

```elisp
(ert-deftest jaunder-new-post-writes-timestamped-draft ()
  (let ((dir (make-temp-file "jaunder-np-" t)))
    (unwind-protect
        (let ((path (jaunder--new-post-in dir "20260703T101500")))
          (should (equal (file-name-nondirectory path) "draft-20260703T101500.org"))
          (should (file-exists-p path))
          (let ((buf (find-file-noselect path)))
            (unwind-protect
                (with-current-buffer buf
                  (should (equal (jaunder--buffer-property "JAUNDER_STATUS") "draft"))
                  (should (jaunder--buffer-keyword "TITLE"))   ; present (may be empty)
                  (should (jaunder--buffer-keyword "DATE")))
              (kill-buffer buf))))
      (delete-directory dir t))))
```

- [ ] **Step 2: Run, verify fail**

Run: `emacs --batch -Q -l elisp/scripts/run-tests.el` Expected: FAIL —
`jaunder--new-post-in` undefined.

- [ ] **Step 3: Implement**

```elisp
(defun jaunder--new-post-in (dir now-string)
  "Create and save a timestamped draft in DIR stamped NOW-STRING; return its path.
Inserts the minimal org template (empty TITLE, DATE now, empty KEYWORDS and
DESCRIPTION, JAUNDER_STATUS draft) and leaves point in the body."
  (let* ((path (expand-file-name (format "draft-%s.org" now-string) dir))
         (buf (find-file-noselect path)))
    (with-current-buffer buf
      (insert "#+TITLE: \n"
              (format "#+DATE: %s\n" (format-time-string "[%Y-%m-%d %a %H:%M]"))
              "#+KEYWORDS: \n"
              "#+DESCRIPTION: \n"
              "#+PROPERTY: JAUNDER_STATUS draft\n\n")
      (save-buffer))
    path))

(defun jaunder-new-post ()
  "Create a new Jaunder draft in the blog whose directory contains `default-directory'.
When no blog matches, prompt to choose one from `jaunder-blogs'.  Inserts the
minimal template and visits the file."
  (interactive)
  (let* ((dir (or (seq-some
                   (lambda (entry)
                     (let ((root (file-name-as-directory (expand-file-name (car entry)))))
                       (and (string-prefix-p root (expand-file-name default-directory))
                            root)))
                   jaunder-blogs)
                  (if jaunder-blogs
                      (completing-read "Blog directory: " (mapcar #'car jaunder-blogs) nil t)
                    default-directory)))
         (path (jaunder--new-post-in dir (format-time-string "%Y%m%dT%H%M%S"))))
    (switch-to-buffer (find-file-noselect path))
    (goto-char (point-max))))
```

- [ ] **Step 4: Run, verify pass**

Run: `emacs --batch -Q -l elisp/scripts/run-tests.el` Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add elisp/jaunder.el elisp/test/jaunder-test.el
cargo xtask check
git commit -m "feat(emacs): jaunder-new-post minimal template + timestamped draft"
```

---

### Task 10: `jaunder-publish` / `jaunder-save-draft` + live end-to-end tests

**Files:**

- Modify: `elisp/jaunder.el`
- Create: `elisp/test/jaunder-publish-integration.el` (auto-globbed live suite)

**Interfaces:**

- Consumes: everything from Tasks 2–9 plus `jaunder--org->atom`,
  `jaunder--atom-entry->xml`, `jaunder--localize-media`,
  `jaunder--http-request`, `jaunder--build-url` (existing).
- Produces: `jaunder-publish (&optional force-draft)` and
  `jaunder-save-draft ()` (interactive).

- [ ] **Step 1: Write the failing live tests**

```elisp
;;; jaunder-publish-integration.el --- C4 live publish tests -*- lexical-binding: t; -*-
;;; Commentary:
;; End-to-end publish flow against a real server (#137 harness, ADR-0035).
;; Runs via `cargo xtask elisp-integration'.  The harness binds
;; `jaunder-base-url'/`jaunder-username', so the publish commands resolve the
;; blog via the single-blog globals fallback (jaunder-blogs unset here).
;;; Code:

(require 'ert)
(require 'jaunder)
(require 'jaunder-integration-helper)

(defmacro jaunder-pub-test--in-buffer (contents &rest body)
  "Write CONTENTS to a temp .org file, visit it, run BODY, then clean up."
  (declare (indent 1) (debug t))
  `(let* ((dir (make-temp-file "jaunder-pub-" t))
          (path (expand-file-name "draft-20260101T000000.org" dir))
          (buf (progn (with-temp-file path (insert ,contents))
                      (find-file-noselect path))))
     (unwind-protect
         (with-current-buffer buf ,@body)
       (when (buffer-live-p buf) (with-current-buffer buf (set-buffer-modified-p nil)))
       (when (buffer-live-p buf) (kill-buffer buf))
       (delete-directory dir t))))

(ert-deftest jaunder-publish-creates-then-updates ()
  (jaunder-test--with-live-server
   (jaunder-pub-test--in-buffer
       "#+TITLE: Hello\n#+PROPERTY: JAUNDER_STATUS published\n\nFirst body.\n"
     (jaunder-publish)
     (let ((id (jaunder--buffer-property "JAUNDER_ID"))
           (slug (jaunder--buffer-property "JAUNDER_SLUG"))
           (synced (jaunder--buffer-property "JAUNDER_SYNCED")))
       (should id)
       (should slug)
       (should synced)
       (should (equal (file-name-nondirectory (buffer-file-name))
                      (concat slug ".org")))
       ;; Re-publish updates the same post (id unchanged), not a duplicate.
       (goto-char (point-max)) (insert "More.\n") (save-buffer)
       (jaunder-publish)
       (should (equal (jaunder--buffer-property "JAUNDER_ID") id))))))

(ert-deftest jaunder-publish-stale-if-match-surfaces-412 ()
  (jaunder-test--with-live-server
   (jaunder-pub-test--in-buffer
       "#+TITLE: T\n#+PROPERTY: JAUNDER_STATUS published\n\nBody.\n"
     (jaunder-publish)
     ;; Corrupt the stored ETag → the next PUT must 412 and leave the file intact.
     (jaunder--set-property "JAUNDER_SYNCED" "\"stale\"") (save-buffer)
     (let ((before (buffer-string)))
       (should-error (jaunder-publish))
       (should (equal (buffer-string) before))))))

(ert-deftest jaunder-publish-untitled-note ()
  (jaunder-test--with-live-server
   (jaunder-pub-test--in-buffer
       "#+PROPERTY: JAUNDER_STATUS published\n\n🎉✨\n"
     (jaunder-publish)
     (should (jaunder--buffer-property "JAUNDER_SLUG")))))

(ert-deftest jaunder-publish-scheduled-future ()
  "A scheduled post with a future #+DATE: is accepted and gets an id."
  (jaunder-test--with-live-server
   (jaunder-pub-test--in-buffer
       (concat "#+TITLE: Later\n"
               "#+DATE: [2999-01-01 Tue 00:00]\n"
               "#+PROPERTY: JAUNDER_STATUS scheduled\n\nFuture body.\n")
     (jaunder-publish)
     (should (jaunder--buffer-property "JAUNDER_ID"))
     (should (jaunder--buffer-property "JAUNDER_DATE_UTC")))))

(ert-deftest jaunder-publish-rejects-empty-body ()
  (jaunder-test--with-live-server
   (jaunder-pub-test--in-buffer
       "#+TITLE: T\n#+PROPERTY: JAUNDER_STATUS published\n\n\n"
     (let ((before (buffer-string)))
       (should-error (jaunder-publish))
       (should (null (jaunder--buffer-property "JAUNDER_ID")))
       (should (equal (buffer-string) before))))))

(ert-deftest jaunder-save-draft-pushes-server-side-draft ()
  (jaunder-test--with-live-server
   (jaunder-pub-test--in-buffer
       "#+TITLE: D\n#+DATE: [2026-07-01 Wed 09:00]\n#+PROPERTY: JAUNDER_STATUS published\n\nDraft body.\n"
     ;; Force-draft even though status=published; must succeed and get an id.
     (jaunder-save-draft)
     (should (jaunder--buffer-property "JAUNDER_ID")))))

(provide 'jaunder-publish-integration)
;;; jaunder-publish-integration.el ends here
```

- [ ] **Step 2: Run the live suite, verify it fails**

Run: `devtool run -- cargo xtask elisp-integration` Then read the parked log:
`rg -n 'FAILED|jaunder-publish' .xtask/run/*.err` (or `.out`). Expected: FAIL —
`jaunder-publish`/`jaunder-save-draft` undefined.

- [ ] **Step 3: Implement the commands**

```elisp
(defconst jaunder--entry-content-type "application/atom+xml;type=entry"
  "Request Content-Type for an AtomPub <entry> POST/PUT.")

(defun jaunder-publish (&optional force-draft)
  "Publish the current buffer's org post over AtomPub.
Resolves the blog from the buffer's file, records the machine zone when unset,
maps + validates, uploads media (sent body only), sends (POST create / PUT with
If-Match on update), writes back server values (ID first), and renames the temp
file to <slug>.org.  With FORCE-DRAFT (see `jaunder-save-draft') pushes an
`app:draft' regardless of JAUNDER_STATUS.  A non-2xx status leaves the on-disk
file pristine."
  (interactive)
  (let ((file (or (buffer-file-name)
                  (error "jaunder: buffer is not visiting a file"))))
    (jaunder--with-blog file
      (let* ((status (jaunder--buffer-property "JAUNDER_STATUS"))
             (date-raw (jaunder--buffer-keyword "DATE"))
             (tz (jaunder--buffer-property "JAUNDER_DATE_TZ"))
             (id (jaunder--buffer-property "JAUNDER_ID"))
             (synced (jaunder--buffer-property "JAUNDER_SYNCED"))
             (entry (jaunder--org->atom)))
        (when force-draft (jaunder--force-draft entry))
        ;; Validate BEFORE any buffer write, so a rejected publish leaves the
        ;; on-disk file pristine.
        (jaunder--validate-publish entry status date-raw tz)
        ;; Record the machine zone (idempotent) so #+DATE: is interpreted in a
        ;; recorded zone on later machines.  A first-publish's org->atom above
        ;; already used the local zone, which equals the captured name.
        (jaunder--ensure-date-tz)
        (setf (jaunder-entry-body entry)
              (jaunder--localize-media (jaunder-entry-body entry)))
        (let* ((xml (jaunder--atom-entry->xml entry))
               (resp (if id
                         (jaunder--http-request
                          "PUT"
                          (jaunder--build-url jaunder-base-url "atompub"
                                              jaunder-username "posts" id)
                          xml jaunder--entry-content-type
                          (when synced (list (cons "If-Match" synced))))
                       (jaunder--http-request
                        "POST"
                        (jaunder--build-url jaunder-base-url "atompub"
                                            jaunder-username "posts")
                        xml jaunder--entry-content-type)))
               (code (plist-get resp :status)))
          (unless (memq code '(200 201))
            (error "jaunder: publish failed (HTTP %s)" code))
          (let ((slug (jaunder--write-back resp (null id))))
            (when slug (jaunder--rename-to-slug slug))
            (message "jaunder: published %s" (or slug ""))))))))

(defun jaunder-save-draft ()
  "Publish the current buffer as a server-side draft (forces `app:draft')."
  (interactive)
  (jaunder-publish t))
```

- [ ] **Step 4: Run the live suite, verify it passes**

Run: `devtool run -- cargo xtask elisp-integration` Expected: all
`jaunder-publish-*` / `jaunder-save-draft-*` tests PASS (check for
`xtask-done: … ok=true`).

- [ ] **Step 5: Full gate + commit**

```bash
git add elisp/jaunder.el elisp/test/jaunder-publish-integration.el
cargo xtask validate --no-e2e   # pure ert + fmt + clippy + coverage
git commit -m "feat(emacs): jaunder-publish + jaunder-save-draft end-to-end (closes #162)"
```

> The full `cargo xtask validate` (incl. the elisp-integration VM) runs at ship
> (`jaunder-ship`); Step 4 already exercised the live suite directly.

---

## Self-Review

- **Spec coverage:** commands (`new-post` T9, `publish`/`save-draft` T10);
  validation (T6); media→send ordering (T10 uses C3 + ordered send); POST/PUT +
  If-Match (T10); ID-first write-back incl. zone-capture + publish-now `#+DATE:`
  render (T4/T8/T10); rename + collision (T7); multi-blog config (T5);
  `atom-entry-fields` slug/published (T3); buffer helpers (T2); follow-ons filed
  (T1). Every spec acceptance criterion and live test maps to a task.
- **Placeholder scan:** every code step carries complete elisp; no TBD/TODO.
- **Type consistency:** `jaunder--write-back (response created)`,
  `jaunder--resolve-blog`/`jaunder--with-blog`, `jaunder--atom-entry-fields`
  alist keys (`slug`/`published`/`content-src`/`content-type`),
  `jaunder--location->id`, `jaunder--utc->org-date (utc tz)` are used
  consistently across tasks. </content> </invoke>
