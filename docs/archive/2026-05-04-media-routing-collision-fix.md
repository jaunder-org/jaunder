# Media Routing Collision Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Resolve routing collisions between media URLs and post permalinks by changing media URLs to a 6-segment structure and adding client-side navigation safeguards.

**Architecture:** Update `common/src/media.rs` to use a 2+2 nested prefix for media paths and URLs. Update server handlers to support the extra segment. Add a "reload on mismatch" safeguard in the client-side post page.

**Tech Stack:** Rust, Leptos, Axum

---

## Task 1: Update Media Path Logic in `common`

**Files:**
- Modify: `common/src/media.rs`

- [ ] **Step 1: Update `media_path` and `media_url` implementations**

Modify the functions to split the first 4 characters of the hash into two 2-character segments.

```rust
// common/src/media.rs

/// Compute the relative filesystem path for a media file.
///
/// Returns `"<source>/<p1>/<p2>/<full-sha256>/<filename>"`.
#[must_use]
pub fn media_path(source: &str, sha256: &str, filename: &str) -> String {
    let p1 = &sha256[..2];
    let p2 = &sha256[2..4];
    format!("{source}/{p1}/{p2}/{sha256}/{filename}")
}

/// Compute the URL path for serving a media file.
///
/// Returns `"/media/<source>/<p1>/<p2>/<full-sha256>/<filename>"`.
#[must_use]
pub fn media_url(source: &str, sha256: &str, filename: &str) -> String {
    let p1 = &sha256[..2];
    let p2 = &sha256[2..4];
    format!("/media/{source}/{p1}/{p2}/{sha256}/{filename}")
}
```

- [ ] **Step 2: Update unit tests in `common/src/media.rs`**

Update the expected values in the `media_path_computation` and `media_url_computation` tests.

```rust
// common/src/media.rs (tests)

    #[test]
    fn media_path_computation() {
        let path = media_path("upload", "a3f2deadbeef1234abcd", "photo.jpg");
        assert_eq!(path, "upload/a3/f2/a3f2deadbeef1234abcd/photo.jpg");
    }

    #[test]
    fn media_url_computation() {
        let url = media_url("upload", "a3f2deadbeef1234abcd", "photo.jpg");
        assert_eq!(url, "/media/upload/a3/f2/a3f2deadbeef1234abcd/photo.jpg");
    }
```

- [ ] **Step 3: Run tests to verify**

Run: `cargo nextest run -p common --all-features`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add common/src/media.rs
git commit -m "fix(media): change media path/URL to 6 segments (2+2 prefix)"
```

---

## Task 2: Update Server Upload and Serve Handlers

**Files:**
- Modify: `server/src/media.rs`
- Modify: `server/src/lib.rs`

- [ ] **Step 1: Update `upload_handler` in `server/src/media.rs`**

Update the target directory construction logic.

```rust
// server/src/media.rs - upload_handler

    let sha256_hex = format!("{:x}", hasher.finalize());
    let p1 = &sha256_hex[..2];
    let p2 = &sha256_hex[2..4];
    let hash_dir = storage_path
        .join("media")
        .join("upload")
        .join(p1)
        .join(p2)
        .join(&sha256_hex);
```

- [ ] **Step 2: Update `serve_handler` and `ServeParams` in `server/src/media.rs`**

Update the params struct and the file path construction.

```rust
// server/src/media.rs

#[derive(Deserialize)]
pub struct ServeParams {
    pub source: String,
    pub p1: String,
    pub p2: String,
    pub hash: String,
    pub filename: String,
}

pub async fn serve_handler(
    Extension(state): Extension<Arc<AppState>>,
    Extension(storage_path): Extension<Arc<PathBuf>>,
    Path(params): Path<ServeParams>,
    req_headers: axum::http::HeaderMap,
) -> Result<Response, StatusCode> {
    // ... validation ...
    if !params.hash.starts_with(&params.p1) || !params.hash[2..].starts_with(&params.p2) {
        return Err(StatusCode::NOT_FOUND);
    }

    let file_path = storage_path
        .join("media")
        .join(source.as_str())
        .join(&params.p1)
        .join(&params.p2)
        .join(&params.hash)
        .join(&params.filename);
    // ... rest of function ...
}
```

- [ ] **Step 3: Update Axum route in `server/src/lib.rs`**

Update the route pattern to match the 6 segments.

```rust
// server/src/lib.rs

        .route(
            "/media/{source}/{p1}/{p2}/{hash}/{filename}",
            axum::routing::get(crate::media::serve_handler),
        )
```

- [ ] **Step 4: Verify with integration tests**

Run: `cargo nextest run -p jaunder --test media_handlers`
Expected: PASS (The tests will need minor updates to match the new URL pattern if they hardcode it, but `media_handlers.rs` uses the returned URL from upload, so it should be mostly transparent).

- [ ] **Step 5: Commit**

```bash
git add server/src/media.rs server/src/lib.rs
git commit -m "fix(media): update server handlers and routes for 6-segment URLs"
```

---

## Task 3: Improve Client-Side Safeguard in `PostPage`

**Files:**
- Modify: `web/src/pages/posts.rs`

- [ ] **Step 1: Update `PostPage` reload logic**

Improve the logic that detects when the Leptos router has incorrectly matched a system route.

```rust
// web/src/pages/posts.rs - PostPage component

    let post = Resource::new(
        post_data,
        |(username, year, month, day, slug): (Option<String>, i32, u32, u32, String)| async move {
            let username = match username {
                Some(value) if value.starts_with('~') => value.strip_prefix('~').unwrap().to_string(),
                _ => {
                    // This is not a post permalink segment.
                    // If we are on WASM, force a full page reload to let the server handle it.
                    #[cfg(target_arch = "wasm32")]
                    if let Some(window) = web_sys::window() {
                        if let Ok(href) = window.location().href() {
                            let _ = window.location().replace(&href);
                        }
                    }
                    return Err(WebError::validation("Invalid permalink"));
                }
            };
            get_post(username, year, month, day, slug).await
        },
    );
```

- [ ] **Step 2: Commit**

```bash
git add web/src/pages/posts.rs
git commit -m "fix(web): improve client-side reload safeguard for permalink routing"
```
