use chrono::Utc;
use common::ids::PostId;
use common::test_support::{parse_audience_name, parse_row_limit};
use common::visibility::{AudienceTarget, ViewerIdentity, local_subscriber_identity};
use rstest::*;
use rstest_reuse::*;
use storage::test_support::{Backend, SeedRawPost, backends, seed_users};

use super::fixtures::{anon_published, channel_id_by_name, local_channel_id, raw_exec};

// The full resolution matrix: viewers {anonymous, author A, active subscriber S,
// named-member M (in audience G, also subscribed), non-member N (not subscribed)}
// × posts {Public, Private, Subscribers, Named(G), Named(G2), Public+Named(G)},
// asserting both `get_post_by_id` visibility AND presence in `list_published`
// per the truth table in the plan (Task 13). A post is returned to a viewer only
// if the viewer is the author OR a targeted audience admits them; admission is
// `active`-subscription-only (fail-closed).
#[apply(backends)]
#[tokio::test]
async fn resolution_matrix(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let local = local_channel_id(backend, &env).await;

    let [a, s, m, n] = seed_users(state).await;
    state
        .subscriptions
        .subscribe(a, &local_subscriber_identity(local, s))
        .await
        .unwrap();
    let m_sub = state
        .subscriptions
        .subscribe(a, &local_subscriber_identity(local, m))
        .await
        .unwrap();
    let g = state
        .audiences
        .create_audience(a, &parse_audience_name("G"))
        .await
        .unwrap();
    let g2 = state
        .audiences
        .create_audience(a, &parse_audience_name("G2"))
        .await
        .unwrap();
    state.audiences.add_member(a, g, m_sub).await.unwrap();

    let make = |audiences: Vec<AudienceTarget>| SeedRawPost::new(a).audiences(audiences);
    let p_public = make(vec![AudienceTarget::Public]).seed(state).await.post_id;
    let p_private = make(vec![]).seed(state).await.post_id;
    let p_subscribers = make(vec![AudienceTarget::Subscribers])
        .seed(state)
        .await
        .post_id;
    let p_named_g = make(vec![AudienceTarget::Named(g)])
        .seed(state)
        .await
        .post_id;
    let p_named_g2 = make(vec![AudienceTarget::Named(g2)])
        .seed(state)
        .await
        .post_id;
    let p_public_named_g = make(vec![AudienceTarget::Public, AudienceTarget::Named(g)])
        .seed(state)
        .await
        .post_id;

    let anon = ViewerIdentity::Anonymous;
    let viewer_a = ViewerIdentity::local(a);
    let viewer_s = ViewerIdentity::local(s);
    let viewer_m = ViewerIdentity::local(m);
    let viewer_n = ViewerIdentity::local(n);

    raw_exec(
        backend,
        &env,
        "INSERT INTO channels (name) VALUES ('activitypub')",
    )
    .await;
    let remote_channel = channel_id_by_name(backend, &env, "activitypub").await;
    let impostor = ViewerIdentity::Remote {
        channel_id: remote_channel,
        subscriber_ref: a.to_string().into(),
    };

    let matrix: &[(&str, PostId, [bool; 6])] = &[
        ("Public", p_public, [true, true, true, true, true, true]),
        (
            "Private",
            p_private,
            [false, true, false, false, false, false],
        ),
        (
            "Subscribers",
            p_subscribers,
            [false, true, true, true, false, false],
        ),
        (
            "Named(G)",
            p_named_g,
            [false, true, false, true, false, false],
        ),
        (
            "Named(G2)",
            p_named_g2,
            [false, true, false, false, false, false],
        ),
        (
            "Public+Named(G)",
            p_public_named_g,
            [true, true, true, true, true, true],
        ),
    ];
    let viewers: [(&str, &ViewerIdentity); 6] = [
        ("anon", &anon),
        ("A", &viewer_a),
        ("S", &viewer_s),
        ("M", &viewer_m),
        ("N", &viewer_n),
        ("impostor", &impostor),
    ];

    for (label, post_id, expected) in matrix {
        for (i, (vlabel, viewer)) in viewers.iter().enumerate() {
            let visible = state
                .posts
                .get_post_by_id(*post_id, viewer)
                .await
                .unwrap()
                .is_some();
            assert_eq!(
                visible, expected[i],
                "get_post_by_id: post {label} for viewer {vlabel}: expected {}, got {visible}",
                expected[i]
            );
        }
    }

    for (vi, (vlabel, viewer)) in viewers.iter().enumerate() {
        let listed: std::collections::HashSet<PostId> = state
            .posts
            .list_published(None, parse_row_limit("100"), viewer, Utc::now())
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.post_id)
            .collect();
        for (label, post_id, expected) in matrix {
            assert_eq!(
                listed.contains(post_id),
                expected[vi],
                "list_published: post {label} for viewer {vlabel}: expected {}, present={}",
                expected[vi],
                listed.contains(post_id)
            );
        }
    }
}

// An anonymous viewer must not be admitted by a subscription row whose
// `subscriber_ref` is the empty string (#686).
#[apply(backends)]
#[tokio::test]
async fn anonymous_is_not_admitted_by_an_empty_subscriber_ref(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let [a] = seed_users(state).await;

    raw_exec(
        backend,
        &env,
        &format!(
            "INSERT INTO subscriptions (author_user_id, channel_id, subscriber_ref, status_id) \
             VALUES ({a}, (SELECT channel_id FROM channels WHERE name='local'), '', \
                     (SELECT status_id FROM subscription_statuses WHERE name='active'))"
        ),
    )
    .await;

    let subscribers_only = SeedRawPost::new(a)
        .audiences(vec![AudienceTarget::Subscribers])
        .seed(state)
        .await
        .post_id;

    let anon = ViewerIdentity::Anonymous;
    assert!(
        state
            .posts
            .get_post_by_id(subscribers_only, &anon)
            .await
            .unwrap()
            .is_none(),
        "get_post_by_id: an empty subscriber_ref must not admit an anonymous viewer"
    );
    let listed: std::collections::HashSet<PostId> = anon_published(state, "100")
        .await
        .into_iter()
        .map(|p| p.post_id)
        .collect();
    assert!(
        !listed.contains(&subscribers_only),
        "list_published: an empty subscriber_ref must not admit an anonymous viewer"
    );
}
