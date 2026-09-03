# Issue #1263: Shorten redundant owner-qualified names

## Outcome

Production Rust call sites retain one meaningful owner qualifier without
repeating that owner in the item name. The refactor changes source names only;
behavior, wire values, generated endpoint paths, and module ownership remain
unchanged.

## Load-bearing decisions

- Rename `client_telemetry::CLIENT_TELEMETRY_VERSION` to
  `client_telemetry::WIRE_VERSION`.
- Rename `media::{media_path, media_url}` to `media::{path, url}` and
  `backup::backup_table_set` to `backup::table_set`.
- Rename the render leaves to
  `render::{body, post_inner, post_content, masthead, load_more}`; the `post_`
  qualifier remains where `inner` or `content` alone would be ambiguous.
- Rename `regenerate::regenerate_feed` to `regenerate::feed`,
  `instance_identity::ensure_instance_identity` to `instance_identity::ensure`,
  and `render::render_with_media` to `render::with_media`.
- Retain `viewer::viewer_identity`: `identity` alone loses the distinction
  between account, viewer-session, and anonymous resolution.
- Retain `timeline::list_local_timeline`: its generated server-function endpoint
  is an existing wire contract. The inventoried `enqueue_feed_events` symbol is
  absent and requires no replacement.
- Public and private Rust symbols receive the same clean cutover: update every
  production caller plus tests, doctests, and test support needed to compile;
  add no alias, compatibility re-export, or deprecated spelling.

## Acceptance

- All twelve confirmed symbols and every resolved caller use the selected names;
  none of their old spellings remain.
- The eight production Rust roots contain no additional confirmed owner-token
  repetition from this inventory, and every retained candidate has the rationale
  above.
- Existing tests pass without behavior-focused test additions because no runtime
  contract changes.
- Public serialized values and `/api/timeline/list_local_timeline` remain
  byte-for-byte unchanged.

## Boundaries

- Naming only: no module moves, behavior changes, import cleanup, or new
  abstractions.
- Exclude generated output and unrelated test naming.
- Do not shorten domain types or names whose repeated token carries semantic
  meaning.
