//! Public library-seam tests for deterministic performance fixture population.

use std::collections::BTreeMap;

use performance::{CountOverrides, DatasetProfile, PersistedCursor, Workload, validate_manifest};
use rstest::*;
use rstest_reuse::*;
use storage::test_support::{Backend, backends};

use crate::performance::{PerformanceSeedStorage, seed_performance_fixture_with_audit};

#[apply(backends)]
#[tokio::test]
async fn small_fixture_resolves_a_valid_manifest_from_persisted_records(#[case] backend: Backend) {
    let env = backend.setup().pristine().await;
    let output = tempfile::tempdir().expect("temporary manifest directory");
    let storage_root = tempfile::tempdir().expect("temporary Media storage root");
    let (manifest, audit, _) = seed_performance_fixture_with_audit(
        PerformanceSeedStorage {
            users: env.users(),
            posts: env.posts(),
            subscriptions: env.subscriptions(),
            audiences: env.audiences(),
            media: env.media(),
            write_scope: env.write_scope(),
        },
        DatasetProfile::Small,
        CountOverrides::default(),
        output.path(),
        storage_root.path(),
    )
    .await
    .expect("small fixture seeds through typed storage");
    let persisted: performance::DatasetManifest = serde_json::from_slice(
        &std::fs::read(output.path().join(performance::DATASET_MANIFEST_FILENAME))
            .expect("read emitted manifest"),
    )
    .expect("parse emitted manifest");
    validate_manifest(&persisted).expect("emitted manifest satisfies shared contract");

    validate_manifest(&manifest).expect("persisted manifest satisfies shared contract");
    assert_eq!(manifest.plan.posts, 100);
    assert_eq!(
        manifest
            .plan
            .lifecycle
            .iter()
            .take(4)
            .map(|item| item.count)
            .collect::<Vec<_>>(),
        [60, 15, 15, 10],
    );
    assert_eq!(
        manifest
            .plan
            .revision_distribution
            .iter()
            .map(|item| item.count)
            .collect::<Vec<_>>(),
        [70, 20, 10],
    );
    assert_eq!(
        manifest
            .plan
            .media_distribution
            .iter()
            .map(|item| item.count)
            .collect::<Vec<_>>(),
        [75, 20, 5],
    );
    assert_eq!(manifest.plan.revisions, 500);
    assert_eq!(
        manifest
            .plan
            .tag_distribution
            .iter()
            .map(|item| item.count)
            .collect::<Vec<_>>(),
        [25, 50, 25],
    );
    assert_eq!(
        manifest
            .plan
            .audience_distribution
            .iter()
            .map(|item| item.count)
            .collect::<Vec<_>>(),
        [50, 25, 25],
    );
    assert_eq!(
        manifest
            .plan
            .body_distribution
            .iter()
            .map(|item| item.count)
            .collect::<Vec<_>>(),
        [12, 11, 11, 11, 11, 11, 11, 11, 11],
    );
    assert_eq!(manifest.plan.lifecycle[4].count, 20);
    assert_eq!(manifest.plan.authors, 10);
    assert_eq!(manifest.plan.follows_per_author, 9);
    let expected_audit = BTreeMap::from([
        ("lifecycle.live".to_owned(), 60),
        ("lifecycle.draft".to_owned(), 15),
        ("lifecycle.scheduled".to_owned(), 15),
        ("lifecycle.deleted".to_owned(), 10),
        ("lifecycle.backdated_live".to_owned(), 20),
        ("revisions.one".to_owned(), 70),
        ("revisions.five".to_owned(), 20),
        ("revisions.thirty_three".to_owned(), 10),
        ("tags.none".to_owned(), 25),
        ("tags.two".to_owned(), 50),
        ("tags.eight".to_owned(), 25),
        ("audiences.public_only".to_owned(), 50),
        ("audiences.one_private".to_owned(), 25),
        ("audiences.five_private".to_owned(), 25),
        ("body.markdown_256".to_owned(), 12),
        ("body.markdown_4096".to_owned(), 11),
        ("body.markdown_65536".to_owned(), 11),
        ("body.html_256".to_owned(), 11),
        ("body.html_4096".to_owned(), 11),
        ("body.html_65536".to_owned(), 11),
        ("body.plain_text_256".to_owned(), 11),
        ("body.plain_text_4096".to_owned(), 11),
        ("body.plain_text_65536".to_owned(), 11),
        ("media.none".to_owned(), 75),
        ("media.one".to_owned(), 20),
        ("media.five".to_owned(), 5),
    ]);
    assert_eq!(audit.confirmed, expected_audit);
    assert_eq!(audit.persisted, audit.confirmed);
    assert!(manifest.subjects.history_post_id > 0);
    assert!(manifest.subjects.revision_id > 0);
    assert_eq!(manifest.subjects.browser_initial_rows.home, 50);
    assert_eq!(manifest.subjects.browser_initial_rows.app, 6);
    assert!(manifest.subjects.browser_initial_rows.global_history > 50);
    assert_eq!(manifest.subjects.browser_initial_rows.post_history, 32);
    assert_eq!(manifest.cursors.len(), 4);
    for cursor in &manifest.cursors {
        assert!(cursor.matching_result_count > 0);
        assert!(cursor.resolved_rank > 0);
        match cursor.workload {
            Workload::PublicTimeline | Workload::AuthenticatedTimeline => {
                assert!(matches!(cursor.cursor, PersistedCursor::Timeline(_)));
            }
            Workload::OwnerHistory | Workload::PostHistory => {
                assert!(matches!(cursor.cursor, PersistedCursor::History(_)));
            }
            _ => unreachable!("manifest contains only paginated storage workloads"),
        }
    }
    assert!(
        output
            .path()
            .join(performance::DATASET_MANIFEST_FILENAME)
            .is_file()
    );
    assert!(
        !output
            .path()
            .join(format!(".{}.tmp", performance::DATASET_MANIFEST_FILENAME))
            .exists(),
        "atomic publication leaves no temporary manifest",
    );
    let owner = env
        .users()
        .get_user_by_username(
            &manifest
                .subjects
                .username
                .parse()
                .expect("fixture username"),
        )
        .await
        .expect("typed author lookup succeeds")
        .expect("fixture author exists")
        .user_id;
    let history_post = common::ids::PostId::from(
        i64::try_from(manifest.subjects.history_post_id).expect("manifest post id fits storage id"),
    );
    let persisted_post = env
        .posts()
        .get_post_by_id(
            history_post,
            &common::visibility::ViewerIdentity::local(owner),
        )
        .await
        .expect("typed selected post query succeeds")
        .expect("selected post exists for its manifest owner");
    assert_eq!(persisted_post.user_id, owner);
    assert_eq!(persisted_post.body.len(), 65_536);
    let mut revision_count = 0;
    let mut cursor = None;
    loop {
        let page = env
            .posts()
            .list_post_revision_history(
                owner,
                history_post,
                cursor,
                common::pagination::PageSize::default(),
            )
            .await
            .expect("typed selected history query succeeds")
            .expect("selected post history exists");
        if page.revisions.is_empty() {
            break;
        }
        cursor = page
            .revisions
            .last()
            .map(|revision| storage::PostRevisionCursor {
                revision_id: revision.revision_id,
            });
        revision_count += page.revisions.len();
    }
    assert_eq!(revision_count + 1, 33);
    for index in 0..10 {
        let author = env
            .users()
            .get_user_by_username(
                &format!("perf-author-{index:04}")
                    .parse()
                    .expect("canonical fixture username"),
            )
            .await
            .expect("typed author lookup succeeds")
            .expect("canonical author exists")
            .user_id;
        let subscribers = env
            .subscriptions()
            .list_subscribers(author)
            .await
            .expect("typed subscription listing succeeds");
        assert_eq!(subscribers.len(), 9, "each author has nine ring peers");
        for other in 0..10 {
            if other == index {
                continue;
            }
            let viewer = env
                .users()
                .get_user_by_username(
                    &format!("perf-author-{other:04}")
                        .parse()
                        .expect("canonical fixture username"),
                )
                .await
                .expect("typed author lookup succeeds")
                .expect("canonical author exists")
                .user_id;
            assert!(
                env.subscriptions()
                    .is_subscriber(author, &common::visibility::ViewerIdentity::local(viewer))
                    .await
                    .expect("typed ring subscription lookup succeeds"),
                "every distinct author follows this ring peer",
            );
        }
        let audiences = env
            .audiences()
            .list_audiences(author)
            .await
            .expect("typed audience listing succeeds");
        assert_eq!(audiences.len(), 5);
        for audience in audiences {
            assert_eq!(
                env.audiences()
                    .list_members(author, audience.audience_id)
                    .await
                    .expect("typed audience membership listing succeeds")
                    .len(),
                9,
                "every fixture audience contains all ring peers",
            );
        }
    }
    let media = env
        .media()
        .list_media(
            owner,
            None,
            common::pagination::RowLimit::at_most(10),
            common::pagination::PageOffset::default(),
        )
        .await
        .expect("typed Media query succeeds");
    assert_eq!(media.len(), 5);
    for record in media {
        let content = storage_root.path().join("media").join(common::media::path(
            &record.source,
            &record.sha256,
            &record.filename,
        ));
        assert!(content.is_file(), "canonical Media bytes exist");
    }
}

#[apply(backends)]
#[tokio::test]
async fn overridden_fixture_records_and_validates_exact_requested_totals(#[case] backend: Backend) {
    let env = backend.setup().pristine().await;
    let output = tempfile::tempdir().expect("temporary manifest directory");
    let storage_root = tempfile::tempdir().expect("temporary Media storage root");
    let overrides = CountOverrides {
        posts: Some(120),
        authors: Some(12),
        revisions: Some(777),
    };
    let (manifest, audit, _) = seed_performance_fixture_with_audit(
        PerformanceSeedStorage {
            users: env.users(),
            posts: env.posts(),
            subscriptions: env.subscriptions(),
            audiences: env.audiences(),
            media: env.media(),
            write_scope: env.write_scope(),
        },
        DatasetProfile::Small,
        overrides,
        output.path(),
        storage_root.path(),
    )
    .await
    .expect("overridden fixture seeds through typed storage");

    assert_eq!(
        (
            manifest.plan.posts,
            manifest.plan.authors,
            manifest.plan.revisions
        ),
        (120, 12, 777)
    );
    assert_eq!(audit.persisted, audit.confirmed);
    validate_manifest(&manifest).expect("overridden manifest satisfies shared contract");
}
