# Design Spec: 6-Segment Media URLs and Routing Collision Fix

## Problem Statement
The current 5-segment media URLs (`/media/{source}/{prefix4}/{hash}/{filename}`) collide with the 5-segment post permalinks (`/{username}/{year}/{month}/{day}/{slug}`) in the Leptos client-side router. When a user clicks a media link inside a post, Leptos intercepts the navigation and attempts to render it as a post, resulting in an "Invalid permalink" error because the username segment (e.g., `media`) does not start with the required `~` prefix.

## Proposed Changes

### 1. URL & Filesystem Structure
Change the media storage and URL scheme from a single 4-character prefix to two nested 2-character prefixes. This increases the segment count to 6, making it distinct from any current or planned post permalink structure.

*   **Logic:**
    *   Hash: `a3f2dead...`
    *   New URL: `/media/upload/a3/f2/a3f2dead.../filename.jpg`
    *   New Path: `media/upload/a3/f2/a3f2dead.../filename.jpg`

### 2. Component Updates

#### A. `common/src/media.rs`
*   Update `media_path` and `media_url` to implement the new 2+2 nesting logic.
*   Update unit tests to verify the 6-segment output.

#### B. `server/src/media.rs`
*   **Upload Handler:** Update directory creation and file moving logic to use the nested `p1/p2` structure.
*   **Serve Handler:** Update the `ServeParams` struct and path construction to accommodate the extra segment.

#### C. `server/src/lib.rs`
*   Update the Axum route definition:
    ```rust
    // New route
    .route("/media/{source}/{p1}/{p2}/{hash}/{filename}", axum::routing::get(crate::media::serve_handler))
    ```

#### D. `web/src/pages/posts.rs`
*   **Defense in Depth:** Even though the collision is solved by segment count, we will keep and improve the "reload on mismatch" logic. If `PostPage` matches a route where the first segment doesn't start with `~`, it will trigger a `window.location.replace()` to let the server handle the request (e.g., for other potential future 5-segment system routes).

## Migration
Since the project is in the "throwaway stage," no automated migration of existing data on disk is required. Developers should clear their `data/media` directory after this change is applied.

## Success Criteria
1.  Clicking a media link inside a post successfully displays/downloads the media instead of showing "Invalid permalink".
2.  `cargo nextest` passes for all media-related tests (which will be updated to the new scheme).
3.  Direct navigation to a media URL (without `target="_blank"`) works correctly.
