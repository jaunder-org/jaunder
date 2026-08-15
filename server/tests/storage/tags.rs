use common::ids::PostId;
use common::tag::{Tag, TagLabel};
use common::test_support::parse_row_limit;
use common::visibility::ViewerIdentity;
use rstest::*;
use rstest_reuse::*;
use storage::test_support::{Backend, SeedRawPost, SeedUser, backends};
use storage::{AppState, PostTag};

use super::fixtures::anon_by_tag;

/// The post's tags, read back through the normal post read path (#772 hydrates
/// them onto the record, so there is no separate tag-read call to make).
///
/// Two dozen tag tests below re-read a post purely to assert on its tags; the
/// unwrapping is noise that buries the assertion. Mirrors `slugs_of` in
/// `storage/src/posts.rs`' test module, which extracted the same shape there.
async fn tags_of(state: &AppState, post_id: PostId) -> Vec<PostTag> {
    state
        .posts
        .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
        .await
        .expect("get_post_by_id failed")
        .expect("post exists")
        .tags
}

// =============================================================================
// Tag Tests
// =============================================================================

#[apply(backends)]
#[tokio::test]
async fn multiple_tags_on_single_post(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new()
        .display_name("Multi")
        .seed(state)
        .await
        .user_id;

    let post_id = SeedRawPost::new(user).seed(state).await.post_id;

    state
        .posts
        .set_post_tags(
            post_id,
            &[
                "rust".parse::<TagLabel>().unwrap(),
                "performance".parse::<TagLabel>().unwrap(),
                "systems-programming".parse::<TagLabel>().unwrap(),
            ],
        )
        .await
        .expect("set_post_tags failed");

    let tags = tags_of(state, post_id).await;

    assert_eq!(tags.len(), 3);
    let tag_slugs: Vec<&str> = tags.iter().map(|t| t.tag_slug.as_ref()).collect();
    assert!(tag_slugs.contains(&"rust"));
    assert!(tag_slugs.contains(&"performance"));
    assert!(tag_slugs.contains(&"systems-programming"));
}

#[apply(backends)]
#[tokio::test]
async fn empty_tag_list(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new()
        .display_name("NoTag")
        .seed(state)
        .await
        .user_id;

    let post_id = SeedRawPost::new(user).seed(state).await.post_id;

    let tags = tags_of(state, post_id).await;

    assert_eq!(tags.len(), 0);
}

#[apply(backends)]
#[tokio::test]
async fn tag_case_preservation_variants(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new()
        .display_name("Case")
        .seed(state)
        .await
        .user_id;

    let post1 = SeedRawPost::new(user).seed(state).await.post_id;

    let post2 = SeedRawPost::new(user).seed(state).await.post_id;

    // Tag with different casings but same canonical form - should map to same slug
    state
        .posts
        .set_post_tags(post1, &["Web-Development".parse::<TagLabel>().unwrap()])
        .await
        .expect("set_post_tags post1 failed");
    state
        .posts
        .set_post_tags(post2, &["WEB-DEVELOPMENT".parse::<TagLabel>().unwrap()])
        .await
        .expect("set_post_tags post2 failed");

    let tags1 = tags_of(state, post1).await;
    let tags2 = tags_of(state, post2).await;

    assert_eq!(tags1[0].tag_slug, "web-development");
    assert_eq!(tags2[0].tag_slug, "web-development");
    assert_eq!(tags1[0].tag_display, "Web-Development");
    assert_eq!(tags2[0].tag_display, "WEB-DEVELOPMENT");

    let tag_slug: Tag = "web-development".parse().unwrap();
    let posts = anon_by_tag(state, &tag_slug, "50").await;

    assert_eq!(posts.len(), 2);
}

#[apply(backends)]
#[tokio::test]
async fn restating_the_set_without_one_tag_drops_only_that_tag(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new()
        .display_name("Selective")
        .seed(state)
        .await
        .user_id;

    let post_id = SeedRawPost::new(user).seed(state).await.post_id;

    state
        .posts
        .set_post_tags(
            post_id,
            &[
                "tag-a".parse::<TagLabel>().unwrap(),
                "tag-b".parse::<TagLabel>().unwrap(),
                "tag-c".parse::<TagLabel>().unwrap(),
            ],
        )
        .await
        .expect("set_post_tags failed");

    let tags = tags_of(state, post_id).await;
    assert_eq!(tags.len(), 3);

    // Dropping one tag is expressed by restating the desired set without it.
    state
        .posts
        .set_post_tags(
            post_id,
            &[
                "tag-a".parse::<TagLabel>().unwrap(),
                "tag-c".parse::<TagLabel>().unwrap(),
            ],
        )
        .await
        .expect("set_post_tags failed");

    let tags = tags_of(state, post_id).await;
    assert_eq!(tags.len(), 2);
    let tag_slugs: Vec<&str> = tags.iter().map(|t| t.tag_slug.as_ref()).collect();
    assert!(!tag_slugs.contains(&"tag-b"));
    assert!(tag_slugs.contains(&"tag-a"));
    assert!(tag_slugs.contains(&"tag-c"));
}

#[apply(backends)]
#[tokio::test]
async fn numeric_tag(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new()
        .display_name("Numeric")
        .seed(state)
        .await
        .user_id;

    let post_id = SeedRawPost::new(user).seed(state).await.post_id;

    state
        .posts
        .set_post_tags(
            post_id,
            &[
                "python3".parse::<TagLabel>().unwrap(),
                "rust-2024".parse::<TagLabel>().unwrap(),
                "0day".parse::<TagLabel>().unwrap(),
            ],
        )
        .await
        .expect("set_post_tags failed");

    let tags = tags_of(state, post_id).await;

    assert_eq!(tags.len(), 3);
    let tag_slugs: Vec<&str> = tags.iter().map(|t| t.tag_slug.as_ref()).collect();
    assert!(tag_slugs.contains(&"python3"));
    assert!(tag_slugs.contains(&"rust-2024"));
    assert!(tag_slugs.contains(&"0day"));
}

#[apply(backends)]
#[tokio::test]
async fn many_tags_many_posts(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new()
        .display_name("ManyTags")
        .seed(state)
        .await
        .user_id;

    let mut post_ids = Vec::new();
    let tags = vec!["rust", "golang", "python", "javascript", "typescript"];
    let labels: Vec<TagLabel> = tags
        .iter()
        .map(|tag| tag.parse::<TagLabel>().unwrap())
        .collect();

    for _ in 0..3 {
        let post_id = SeedRawPost::new(user).seed(state).await.post_id;
        post_ids.push(post_id);

        state
            .posts
            .set_post_tags(post_id, &labels)
            .await
            .expect("set_post_tags failed");
    }

    for post_id in &post_ids {
        let tags_on_post = tags_of(state, *post_id).await;
        assert_eq!(tags_on_post.len(), 5);
    }

    for tag in &tags {
        let tag_slug: Tag = tag.parse().unwrap();
        let posts = anon_by_tag(state, &tag_slug, "50").await;
        assert_eq!(posts.len(), 3);
    }
}

#[apply(backends)]
#[tokio::test]
async fn tag_all_numeric(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new()
        .display_name("NumericOnly")
        .seed(state)
        .await
        .user_id;

    let post_id = SeedRawPost::new(user).seed(state).await.post_id;

    state
        .posts
        .set_post_tags(
            post_id,
            &[
                "2024".parse::<TagLabel>().unwrap(),
                "42".parse::<TagLabel>().unwrap(),
            ],
        )
        .await
        .expect("set_post_tags failed");

    let tags = tags_of(state, post_id).await;

    assert_eq!(tags.len(), 2);
    let tag_slugs: Vec<&str> = tags.iter().map(|t| t.tag_slug.as_ref()).collect();
    assert!(tag_slugs.contains(&"2024"));
    assert!(tag_slugs.contains(&"42"));
}

#[apply(backends)]
#[tokio::test]
async fn tag_hyphen_boundaries(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new()
        .display_name("Hyphen")
        .seed(state)
        .await
        .user_id;

    let post_id = SeedRawPost::new(user).seed(state).await.post_id;

    // Valid: hyphens in the middle and at end
    state
        .posts
        .set_post_tags(
            post_id,
            &[
                "web-development".parse::<TagLabel>().unwrap(),
                "a-b-c".parse::<TagLabel>().unwrap(),
                "end-".parse::<TagLabel>().unwrap(),
            ],
        )
        .await
        .expect("set_post_tags failed");

    let tags = tags_of(state, post_id).await;

    assert_eq!(tags.len(), 3);

    // Invalid slugs (leading hyphen, underscore) can no longer reach
    // `set_post_tags`: its `&[TagLabel]` argument is validated at construction,
    // so those cases are unconstructible here (they are rejected at the type
    // boundary / atompub ingest filter instead).
}

#[apply(backends)]
#[tokio::test]
async fn tag_with_long_display(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new()
        .display_name("LongTagUser")
        .seed(state)
        .await
        .user_id;

    let post_id = SeedRawPost::new(user).seed(state).await.post_id;

    let long_display = "very-long-technical-term-with-many-hyphens-and-lowercase-letters";
    state
        .posts
        .set_post_tags(post_id, &[long_display.parse::<TagLabel>().unwrap()])
        .await
        .expect("set_post_tags failed");

    let tags = tags_of(state, post_id).await;

    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].tag_display, long_display);
}

#[apply(backends)]
#[tokio::test]
async fn tag_list_ordering(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new()
        .display_name("Ordering")
        .seed(state)
        .await
        .user_id;

    let post1 = SeedRawPost::new(user).seed(state).await.post_id;

    let post2 = SeedRawPost::new(user).seed(state).await.post_id;

    // Tag in an order that is not the expected slug order.
    state
        .posts
        .set_post_tags(
            post1,
            &[
                "zebra".parse::<TagLabel>().unwrap(),
                "apple".parse::<TagLabel>().unwrap(),
                "mango".parse::<TagLabel>().unwrap(),
            ],
        )
        .await
        .expect("set_post_tags failed");

    state
        .posts
        .set_post_tags(post2, &["mango".parse::<TagLabel>().unwrap()])
        .await
        .expect("set_post_tags failed");

    let tags1 = tags_of(state, post1).await;

    assert_eq!(tags1.len(), 3);
    let slugs1: Vec<&str> = tags1.iter().map(|t| t.tag_slug.as_ref()).collect();
    assert_eq!(slugs1, vec!["apple", "mango", "zebra"]);

    // Verify consistency on multiple calls
    let tags1_again = tags_of(state, post1).await;

    assert_eq!(tags1_again.len(), 3);
    assert_eq!(tags1_again[0].tag_slug, "apple");
}

#[apply(backends)]
#[tokio::test]
async fn tags_for_multiple_posts(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new()
        .display_name("MultiPost")
        .seed(state)
        .await
        .user_id;

    let post1 = SeedRawPost::new(user).seed(state).await.post_id;

    let post2 = SeedRawPost::new(user).seed(state).await.post_id;

    // Only post2 is tagged; post1 stays untagged to assert the empty case.
    state
        .posts
        .set_post_tags(post2, &["featured".parse::<TagLabel>().unwrap()])
        .await
        .expect("set_post_tags failed");

    let tags1 = tags_of(state, post1).await;
    assert_eq!(tags1.len(), 0);

    let tags2 = tags_of(state, post2).await;
    assert_eq!(tags2.len(), 1);
}

#[apply(backends)]
#[tokio::test]
async fn tag_mixed_alphanumeric(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new()
        .display_name("Mixed")
        .seed(state)
        .await
        .user_id;

    let post_id = SeedRawPost::new(user).seed(state).await.post_id;

    state
        .posts
        .set_post_tags(
            post_id,
            &[
                "version-2-0-1".parse::<TagLabel>().unwrap(),
                "HTTP2".parse::<TagLabel>().unwrap(),
                "3D-Graphics".parse::<TagLabel>().unwrap(),
            ],
        )
        .await
        .expect("set_post_tags failed");

    let tags = tags_of(state, post_id).await;

    assert_eq!(tags.len(), 3);
    assert_eq!(tags[0].tag_slug, "3d-graphics");
    assert_eq!(tags[1].tag_slug, "http2");
    assert_eq!(tags[2].tag_slug, "version-2-0-1");
}

#[apply(backends)]
#[tokio::test]
async fn simple_tag_lifecycle(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new()
        .display_name("Simple")
        .seed(state)
        .await
        .user_id;

    let post_id = SeedRawPost::new(user).seed(state).await.post_id;

    state
        .posts
        .set_post_tags(post_id, &["test".parse::<TagLabel>().unwrap()])
        .await
        .expect("set_post_tags failed");

    let tags_before = tags_of(state, post_id).await;
    assert_eq!(tags_before.len(), 1);
    assert_eq!(tags_before[0].tag_display, "test");

    let tag_slug: Tag = "test".parse().unwrap();
    let posts_before = anon_by_tag(state, &tag_slug, "50").await;
    assert_eq!(posts_before.len(), 1);

    // An empty desired set clears the post's tags (D11).
    state
        .posts
        .set_post_tags(post_id, &[])
        .await
        .expect("set_post_tags failed");

    let tags_after = tags_of(state, post_id).await;
    assert_eq!(tags_after.len(), 0);

    // List by tag again - should return empty list (tag exists but no posts have it)
    let posts_after = anon_by_tag(state, &tag_slug, "50").await;
    assert_eq!(posts_after.len(), 0);
}

#[apply(backends)]
#[tokio::test]
async fn tag_creation_and_retrieval(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new()
        .display_name("Alice")
        .seed(state)
        .await
        .user_id;

    let post_id = SeedRawPost::new(user).seed(state).await.post_id;

    state
        .posts
        .set_post_tags(post_id, &["rust".parse::<TagLabel>().unwrap()])
        .await
        .expect("set_post_tags failed");

    let tags = tags_of(state, post_id).await;

    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].tag_slug, "rust");
    assert_eq!(tags[0].tag_display, "rust");
}

#[apply(backends)]
#[tokio::test]
async fn tag_normalization(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new()
        .display_name("Bob")
        .seed(state)
        .await
        .user_id;

    let post_id = SeedRawPost::new(user).seed(state).await.post_id;

    state
        .posts
        .set_post_tags(post_id, &["Rust-Web".parse::<TagLabel>().unwrap()])
        .await
        .expect("set_post_tags failed");

    let tags = tags_of(state, post_id).await;

    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].tag_slug, "rust-web"); // normalized
    assert_eq!(tags[0].tag_display, "Rust-Web"); // original preserved
}

#[apply(backends)]
#[tokio::test]
async fn tag_edge_case_formats(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new().seed(state).await.user_id;

    let post_id = SeedRawPost::new(user).seed(state).await.post_id;

    state
        .posts
        .set_post_tags(
            post_id,
            &[
                "123".parse::<TagLabel>().unwrap(),
                "my-tag-here".parse::<TagLabel>().unwrap(),
                "MyTag".parse::<TagLabel>().unwrap(),
            ],
        )
        .await
        .expect("numeric, hyphenated and mixed-case tags failed");

    let tags = tags_of(state, post_id).await;

    assert_eq!(tags.len(), 3);
}

#[apply(backends)]
#[tokio::test]
async fn tag_display_preservation(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new().seed(state).await.user_id;

    let post_id = SeedRawPost::new(user).seed(state).await.post_id;

    state
        .posts
        .set_post_tags(post_id, &["MySpecialTag".parse::<TagLabel>().unwrap()])
        .await
        .expect("set_post_tags failed");

    let tags = tags_of(state, post_id).await;

    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].tag_display, "MySpecialTag");
    assert_eq!(tags[0].tag_slug, "myspecialtag");
}

#[apply(backends)]
#[tokio::test]
async fn reconciling_to_a_smaller_set_preserves_the_surviving_tags(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new().seed(state).await.user_id;

    let post_id = SeedRawPost::new(user).seed(state).await.post_id;

    state
        .posts
        .set_post_tags(
            post_id,
            &[
                "tag1".parse::<TagLabel>().unwrap(),
                "tag2".parse::<TagLabel>().unwrap(),
                "tag3".parse::<TagLabel>().unwrap(),
            ],
        )
        .await
        .expect("set_post_tags failed");

    let tags = tags_of(state, post_id).await;
    assert_eq!(tags.len(), 3);

    // Restating the set without tag2 drops it and leaves the others in place.
    state
        .posts
        .set_post_tags(
            post_id,
            &[
                "tag1".parse::<TagLabel>().unwrap(),
                "tag3".parse::<TagLabel>().unwrap(),
            ],
        )
        .await
        .expect("set_post_tags failed");

    let tags = tags_of(state, post_id).await;
    assert_eq!(tags.len(), 2);
    let tag_slugs: Vec<_> = tags.iter().map(|t| t.tag_slug.as_ref()).collect();
    assert!(!tag_slugs.contains(&"tag2"));
}

// ====== tags.2: list_tags + tags on the post record ======

#[apply(backends)]
#[tokio::test]
async fn list_tags_returns_alphabetical_with_prefix(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new()
        .display_name("ListTags")
        .seed(state)
        .await
        .user_id;
    let post = SeedRawPost::new(user).seed(state).await.post_id;

    // Mixed-case display tokens — the slug should normalize to lowercase.
    let labels: Vec<TagLabel> = ["Rust", "rust-lang", "performance", "PostgreSQL", "web"]
        .iter()
        .map(|display| display.parse::<TagLabel>().unwrap())
        .collect();
    state.posts.set_post_tags(post, &labels).await.unwrap();

    // No prefix → all tags, alphabetical by slug.
    let all = state
        .posts
        .list_tags(None, parse_row_limit("50"))
        .await
        .unwrap();
    let slugs: Vec<&str> = all.iter().map(|t| t.tag_slug.as_ref()).collect();
    assert_eq!(
        slugs,
        vec!["performance", "postgresql", "rust", "rust-lang", "web"]
    );

    // Prefix "rust" → "rust" and "rust-lang", still alphabetical.
    let rs = state
        .posts
        .list_tags(Some("rust"), parse_row_limit("50"))
        .await
        .unwrap();
    let rs_slugs: Vec<&str> = rs.iter().map(|t| t.tag_slug.as_ref()).collect();
    assert_eq!(rs_slugs, vec!["rust", "rust-lang"]);

    // Prefix case-insensitive: "RUST" matches the same set.
    let upper = state
        .posts
        .list_tags(Some("RUST"), parse_row_limit("50"))
        .await
        .unwrap();
    let upper_slugs: Vec<&str> = upper.iter().map(|t| t.tag_slug.as_ref()).collect();
    assert_eq!(upper_slugs, vec!["rust", "rust-lang"]);

    // Limit clamps the result.
    let limited = state
        .posts
        .list_tags(None, parse_row_limit("2"))
        .await
        .unwrap();
    assert_eq!(limited.len(), 2);

    // Empty-string prefix is treated as "no prefix".
    let empty = state
        .posts
        .list_tags(Some("   "), parse_row_limit("50"))
        .await
        .unwrap();
    assert_eq!(empty.len(), 5);

    // Nonexistent prefix → empty.
    let none = state
        .posts
        .list_tags(Some("zz"), parse_row_limit("50"))
        .await
        .unwrap();
    assert!(none.is_empty());
}

#[apply(backends)]
#[tokio::test]
async fn post_record_carries_tags(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new()
        .display_name("Inline")
        .seed(state)
        .await
        .user_id;

    let mut post_ids = Vec::new();
    for _ in 1..=3 {
        let id = SeedRawPost::new(user).seed(state).await.post_id;
        post_ids.push(id);
    }
    let (p1, p2, p3) = (post_ids[0], post_ids[1], post_ids[2]);

    // p1: two tags, applied in reverse-slug order so the assertion below tests
    // ordering rather than coinciding with insertion order (#772);
    // p2: one tag; p3: none.
    state
        .posts
        .set_post_tags(
            p1,
            &[
                "web".parse::<TagLabel>().unwrap(),
                "Rust".parse::<TagLabel>().unwrap(),
            ],
        )
        .await
        .unwrap();
    state
        .posts
        .set_post_tags(p2, &["performance".parse::<TagLabel>().unwrap()])
        .await
        .unwrap();

    // Each loaded post carries its own tags from the same query that loaded
    // the rest of the row — no separate batch call.
    let p1_record = state
        .posts
        .get_post_by_id(p1, &ViewerIdentity::Anonymous)
        .await
        .expect("get_post_by_id p1")
        .expect("p1 should exist");
    let p1_slugs: Vec<&str> = p1_record.tags.iter().map(|t| t.tag_slug.as_ref()).collect();
    assert_eq!(p1_slugs, vec!["rust", "web"]);
    // Display casing is preserved.
    assert!(p1_record.tags.iter().any(|t| t.tag_display == "Rust"));

    let p2_record = state
        .posts
        .get_post_by_id(p2, &ViewerIdentity::Anonymous)
        .await
        .expect("get_post_by_id p2")
        .expect("p2 should exist");
    assert_eq!(p2_record.tags.len(), 1);
    assert_eq!(p2_record.tags[0].tag_slug, "performance");
    assert_eq!(p2_record.tags[0].tag_display, "performance");

    let p3_record = state
        .posts
        .get_post_by_id(p3, &ViewerIdentity::Anonymous)
        .await
        .expect("get_post_by_id p3")
        .expect("p3 should exist");
    assert!(p3_record.tags.is_empty());
}
