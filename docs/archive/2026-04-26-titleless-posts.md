# Titleless Posts and Inferred Article Titles

**Goal:** Preserve the inline composer as a simple note/tweet-like surface while allowing blog-style posts to gain real titles without an explicit mode switch.

**Decision:** Do not add `title_source`. The only public distinction needed now is whether a post has a meaningful title. The server derives that from explicit title input or body syntax.

## Plan

1. Add pure post metadata derivation.
   - Create a helper that accepts optional explicit title, body, and `PostFormat`.
   - Prefer non-empty explicit title.
   - For Markdown, extract a first meaningful `# Title` heading.
   - For Org, extract `#+title: Title`.
   - Otherwise leave the public title empty and derive only a fallback slug/summary label from body text.

2. Make post titles nullable in storage.
   - Add SQLite and Postgres migrations that make `posts.title` and `post_revisions.title` nullable.
   - Convert any existing empty string titles to `NULL`.
   - For SQLite, rebuild the affected tables and rebuild `post_tags` so its foreign key continues to reference the new `posts` table.
   - Carry `Option<String>` through storage records and create/update inputs.

3. Update create/update server behavior.
   - `create_post` and `update_post` accept empty `title`.
   - Reject only when both title and body are empty.
   - Compute slug from slug override, derived title, or body fallback.
   - Preserve explicit title precedence over heading extraction.

4. Update the inline composer.
   - Stop submitting the first 100 body characters as a hidden title.
   - Submit an empty hidden title.
   - A plain note remains titleless; a body beginning with `# Title` becomes article-like automatically.

5. Update rendering.
   - Public post responses expose `title: Option<String>`.
   - Render article headings only when `title` is present.
   - Timeline cards display title when present and otherwise render the note body/excerpt without a fake heading.

6. Add tests.
   - Unit tests for metadata derivation.
   - Integration tests for explicit title, extracted Markdown title, titleless body fallback, and slug behavior.
   - End-to-end coverage for inline composer titleless notes and inferred Markdown title posts.

## Commit Sequence

1. Metadata helper and unit tests.
2. Nullable-title migrations and storage type updates.
3. Server/API behavior with integration tests.
4. Inline composer and rendering updates with end-to-end tests.

Per repository process, request review before committing.
