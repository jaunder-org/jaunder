use common::ids::UserId;
use common::media::{MediaRef, MediaSource};
use common::test_support::{
    parse_byte_size, parse_content_hash, parse_content_type, parse_filename, parse_page_offset,
    parse_row_limit, parse_url,
};
use common::time::UtcInstant;
use rstest::*;
use rstest_reuse::*;
use storage::test_support::{Backend, SeedUser, backends, seed_users};
use storage::{
    CreateMediaError, DeleteMediaError, MediaRecord, MediaReferenceEvidence, TryDeleteOutcome,
};

// ── MediaStorage tests ────────────────────────────────────────────────────────

fn make_media_record(
    user_id: UserId,
    sha256: &str,
    filename: &str,
    source: MediaSource,
) -> MediaRecord {
    MediaRecord {
        user_id,
        sha256: parse_content_hash(sha256),
        filename: parse_filename(filename),
        source,
        content_type: parse_content_type("image/jpeg"),
        size_bytes: parse_byte_size("12345"),
        source_url: None,
        created_at: UtcInstant::now(),
    }
}

#[apply(backends)]
#[tokio::test]
async fn create_and_get_media(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;
    let sha256 =
        parse_content_hash("abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234");
    let created_at = "2026-08-26T12:34:56.789012Z"
        .parse::<UtcInstant>()
        .expect("valid microsecond instant");
    let mut record = make_media_record(user_id, &sha256, "test.jpg", MediaSource::Upload);
    record.created_at = created_at;
    state.media.create_media(&record).await.unwrap();

    let fetched = state
        .media
        .get_media(
            user_id,
            &sha256,
            &parse_filename("test.jpg"),
            &MediaSource::Upload,
        )
        .await
        .unwrap()
        .expect("record should exist");
    assert_eq!(fetched.user_id, user_id);
    assert_eq!(fetched.sha256, sha256);
    assert_eq!(fetched.filename, "test.jpg");
    assert_eq!(fetched.source, MediaSource::Upload);
    assert_eq!(fetched.content_type, "image/jpeg");
    assert_eq!(fetched.size_bytes, parse_byte_size("12345"));
    assert_eq!(fetched.created_at, created_at);

    let listed = state
        .media
        .list_media(user_id, None, parse_row_limit("10"), parse_page_offset("0"))
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].created_at, created_at);
}

#[apply(backends)]
#[tokio::test]
async fn media_source_url_round_trips_through_the_typed_column(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    let sha256 =
        parse_content_hash("beef1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234");
    let mut record = make_media_record(user_id, &sha256, "cached.jpg", MediaSource::Cached);
    // Spelled non-canonically on the way in: `TaggedUrl`'s `FromStr` lowercases the host
    // and strips the default port, so the value stored is already normalized — asserting the
    // canonical form on read-back is what shows the column carries the *newtype*, not the
    // text as typed (#675).
    record.source_url = Some(parse_url("https://Example.COM:443/x.png"));

    state.media.create_media(&record).await.unwrap();

    let fetched = state
        .media
        .get_media(
            user_id,
            &sha256,
            &parse_filename("cached.jpg"),
            &MediaSource::Cached,
        )
        .await
        .unwrap()
        .expect("record should exist");
    assert_eq!(
        fetched.source_url.as_deref(),
        Some("https://example.com/x.png")
    );
}

#[apply(backends)]
#[tokio::test]
async fn media_row_with_an_invalid_source_url_fails_to_decode(#[case] backend: Backend) {
    // This is what makes `Option<MediaSourceUrl>` a contract rather than documentation: a
    // value that is not a valid absolute `http(s)` URL cannot be read back as one. Nothing
    // writes `source_url` yet (the remote-caching ingest does not exist), so a hand-edited
    // or future-buggy writer is exactly the threat, and it is inserted by raw SQL here
    // because the type makes it unconstructible in Rust.
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    env.base
        .pool()
        .execute(&format!(
            "INSERT INTO media (user_id, sha256, filename, source, content_type, size_bytes, \
             source_url) VALUES ({}, \
             'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc', 'c.png', \
             'cached', 'image/png', 10, 'not a url')",
            i64::from(user_id),
        ))
        .await
        .expect("raw insert should succeed — the database has no opinion on the text");

    let fetched = state
        .media
        .get_media(
            user_id,
            &parse_content_hash("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"),
            &parse_filename("c.png"),
            &MediaSource::Cached,
        )
        .await;
    assert!(
        fetched.is_err(),
        "a non-URL source_url must be a column-decode error, got {fetched:?}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn list_media_skips_rows_that_fail_to_decode(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    // A valid record via the normal (validating) path.
    let good_sha = "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234";
    let record = make_media_record(user_id, good_sha, "good.jpg", MediaSource::Upload);
    state.media.create_media(&record).await.unwrap();

    // A row whose `filename` column is a non-canonical value, inserted directly to
    // bypass the validating `create_media` (the `Filename` type makes an un-sanitized
    // name unconstructible in Rust). `media_record_from_row` fails to decode it.
    // created_at/source_url are omitted so both backends' column defaults apply.
    env.base
        .pool()
        .execute(&format!(
            "INSERT INTO media (user_id, sha256, filename, source, content_type, size_bytes) \
             VALUES ({}, 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb', \
             '../escape', 'upload', 'image/png', 10)",
            i64::from(user_id),
        ))
        .await
        .unwrap();

    // list_media returns the decodable row and silently skips the corrupt one, rather
    // than failing the whole query (which would hide the user's valid media too).
    let listed = state
        .media
        .list_media(user_id, None, parse_row_limit("10"), parse_page_offset("0"))
        .await
        .unwrap();
    assert_eq!(
        listed.len(),
        1,
        "the corrupt row must be skipped and the valid row returned"
    );
    assert_eq!(listed[0].filename, "good.jpg");

    // A direct lookup of the corrupt row still surfaces the decode error (single-row
    // lookups stay strict — only the list path degrades gracefully).
    let direct = state
        .media
        .find_by_hash(
            &parse_content_hash("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            &MediaSource::Upload,
        )
        .await;
    assert!(
        direct.is_err(),
        "a direct lookup of the corrupt row must error"
    );
}

#[apply(backends)]
#[tokio::test]
async fn duplicate_media_returns_already_exists(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    let sha256 = "bbbb1234bbbb1234bbbb1234bbbb1234bbbb1234bbbb1234bbbb1234bbbb1234".to_string();
    let record = make_media_record(user_id, &sha256, "dup.jpg", MediaSource::Upload);
    state.media.create_media(&record).await.unwrap();
    let err = state.media.create_media(&record).await.unwrap_err();
    assert!(
        matches!(err, CreateMediaError::AlreadyExists),
        "expected AlreadyExists, got {err:?}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn delete_media_removes_record(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    let sha256 =
        parse_content_hash("cccc1234cccc1234cccc1234cccc1234cccc1234cccc1234cccc1234cccc1234");
    let record = make_media_record(user_id, &sha256, "del.jpg", MediaSource::Upload);
    state.media.create_media(&record).await.unwrap();
    let evidence = MediaReferenceEvidence::new(env.base.instance_id().clone());
    let outcome = state
        .media
        .try_delete_media(
            user_id,
            &MediaRef {
                source: MediaSource::Upload,
                sha256: sha256.clone(),
                filename: parse_filename("del.jpg"),
            },
            env.base.instance_id(),
            &evidence,
            false,
        )
        .await
        .unwrap();
    assert_eq!(outcome, TryDeleteOutcome::Deleted);

    let fetched = state
        .media
        .get_media(
            user_id,
            &sha256,
            &parse_filename("del.jpg"),
            &MediaSource::Upload,
        )
        .await
        .unwrap();
    assert!(fetched.is_none(), "record should have been deleted");
}

#[apply(backends)]
#[tokio::test]
async fn delete_nonexistent_returns_not_found(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    let sha256 =
        parse_content_hash("dddd1234dddd1234dddd1234dddd1234dddd1234dddd1234dddd1234dddd1234");
    let evidence = MediaReferenceEvidence::new(env.base.instance_id().clone());
    let err = state
        .media
        .try_delete_media(
            user_id,
            &MediaRef {
                source: MediaSource::Upload,
                sha256: sha256.clone(),
                filename: parse_filename("ghost.jpg"),
            },
            env.base.instance_id(),
            &evidence,
            false,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, DeleteMediaError::NotFound),
        "expected NotFound, got {err:?}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn list_media_returns_records_for_user(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let [user_a, user_b] = seed_users(state).await;

    let sha1 = "eeee1234eeee1234eeee1234eeee1234eeee1234eeee1234eeee1234eeee1234".to_string();
    let sha2 = "ffff1234ffff1234ffff1234ffff1234ffff1234ffff1234ffff1234ffff1234".to_string();
    let sha3 = "9999123499991234999912349999123499991234999912349999123499991234".to_string();

    state
        .media
        .create_media(&make_media_record(
            user_a,
            &sha1,
            "a1.jpg",
            MediaSource::Upload,
        ))
        .await
        .unwrap();
    state
        .media
        .create_media(&make_media_record(
            user_a,
            &sha2,
            "a2.jpg",
            MediaSource::Upload,
        ))
        .await
        .unwrap();
    state
        .media
        .create_media(&make_media_record(
            user_b,
            &sha3,
            "b1.jpg",
            MediaSource::Upload,
        ))
        .await
        .unwrap();

    let results = state
        .media
        .list_media(user_a, None, parse_row_limit("10"), parse_page_offset("0"))
        .await
        .unwrap();
    assert_eq!(results.len(), 2, "user_a should have 2 records");
    assert!(results.iter().all(|r| r.user_id == user_a));
}

#[apply(backends)]
#[tokio::test]
async fn list_media_filtered_by_source(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    let sha_up = "8888123488881234888812348888123488881234888812348888123488881234".to_string();
    let sha_ca = "7777123477771234777712347777123477771234777712347777123477771234".to_string();

    state
        .media
        .create_media(&make_media_record(
            user_id,
            &sha_up,
            "up.jpg",
            MediaSource::Upload,
        ))
        .await
        .unwrap();
    state
        .media
        .create_media(&make_media_record(
            user_id,
            &sha_ca,
            "ca.jpg",
            MediaSource::Cached,
        ))
        .await
        .unwrap();

    let uploads = state
        .media
        .list_media(
            user_id,
            Some(&MediaSource::Upload),
            parse_row_limit("10"),
            parse_page_offset("0"),
        )
        .await
        .unwrap();
    assert_eq!(uploads.len(), 1);
    assert_eq!(uploads[0].source, MediaSource::Upload);

    let cached = state
        .media
        .list_media(
            user_id,
            Some(&MediaSource::Cached),
            parse_row_limit("10"),
            parse_page_offset("0"),
        )
        .await
        .unwrap();
    assert_eq!(cached.len(), 1);
    assert_eq!(cached[0].source, MediaSource::Cached);
}

#[apply(backends)]
#[tokio::test]
async fn get_user_upload_usage_returns_zero_initially(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    let usage = state.media.get_user_upload_usage(user_id).await.unwrap();
    assert_eq!(usage, parse_byte_size("0"));
}

#[apply(backends)]
#[tokio::test]
async fn get_user_upload_usage_sums_uploads_only(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    let sha_up = "6666123466661234666612346666123466661234666612346666123466661234".to_string();
    let sha_ca = "5555123455551234555512345555123455551234555512345555123455551234".to_string();

    let mut upload = make_media_record(user_id, &sha_up, "upload.jpg", MediaSource::Upload);
    upload.size_bytes = parse_byte_size("1000");
    state.media.create_media(&upload).await.unwrap();

    let mut cached = make_media_record(user_id, &sha_ca, "cached.jpg", MediaSource::Cached);
    cached.size_bytes = parse_byte_size("9999");
    state.media.create_media(&cached).await.unwrap();

    let usage = state.media.get_user_upload_usage(user_id).await.unwrap();
    assert_eq!(
        usage,
        parse_byte_size("1000"),
        "only upload bytes should count toward usage"
    );
}

#[apply(backends)]
#[tokio::test]
async fn find_by_hash_returns_any_match(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    let sha256 =
        parse_content_hash("4444123444441234444412344444123444441234444412344444123444441234");
    let record = make_media_record(user_id, &sha256, "find.jpg", MediaSource::Upload);
    state.media.create_media(&record).await.unwrap();

    let found = state
        .media
        .find_by_hash(&sha256, &MediaSource::Upload)
        .await
        .unwrap();
    let found = found.expect("should find the record by hash");
    assert_eq!(found.sha256, sha256);
}
