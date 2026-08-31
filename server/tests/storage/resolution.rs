use std::sync::Arc;

use common::ids::PostId;
use common::test_support::{parse_audience_name, parse_row_limit};
use common::visibility::{
    AudienceTarget, SubscriberIdentity, ViewerIdentity, local_subscriber_identity,
};
use rstest::*;
use rstest_reuse::*;
use storage::test_support::{
    Backend, SeedRawPost, backends, confirmed_for as confirmed, seed_users,
};
use storage::{AudienceStorage, SubscriptionStorage, WriteScope};

use super::fixtures::{channel_id_by_name, local_channel_id, raw_exec};

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
    subscribe_confirmed(
        &state.write_scope,
        Arc::clone(&state.subscriptions),
        a,
        local_subscriber_identity(local, s),
    )
    .await;
    let m_sub = subscribe_confirmed(
        &state.write_scope,
        Arc::clone(&state.subscriptions),
        a,
        local_subscriber_identity(local, m),
    )
    .await;
    let g =
        create_audience_confirmed(&state.write_scope, Arc::clone(&state.audiences), a, "G").await;
    let g2 =
        create_audience_confirmed(&state.write_scope, Arc::clone(&state.audiences), a, "G2").await;
    add_member_confirmed(
        &state.write_scope,
        Arc::clone(&state.audiences),
        a,
        g,
        m_sub,
    )
    .await;

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
        subscriber_ref: a.to_string().parse().unwrap(),
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
            .list_published(
                None,
                parse_row_limit("100"),
                viewer,
                common::time::UtcInstant::now(),
            )
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

async fn subscribe_confirmed(
    write_scope: &WriteScope,
    subscriptions: Arc<dyn SubscriptionStorage>,
    author: common::ids::UserId,
    subscriber: SubscriberIdentity,
) -> common::ids::SubscriptionId {
    let outcome = write_scope
        .run(move |transaction| {
            Box::pin(async move {
                subscriptions
                    .subscribe(transaction, author, &subscriber)
                    .await
            })
        })
        .await
        .expect("subscription fixture setup should succeed");
    confirmed(outcome, "subscription fixture setup")
}

async fn create_audience_confirmed(
    write_scope: &WriteScope,
    audiences: Arc<dyn AudienceStorage>,
    author: common::ids::UserId,
    name: &str,
) -> common::ids::AudienceId {
    let name = parse_audience_name(name);
    let outcome = write_scope
        .run(move |transaction| {
            Box::pin(async move { audiences.create_audience(transaction, author, &name).await })
        })
        .await
        .expect("audience fixture setup should succeed");
    confirmed(outcome, "audience fixture setup")
}

async fn add_member_confirmed(
    write_scope: &WriteScope,
    audiences: Arc<dyn AudienceStorage>,
    author: common::ids::UserId,
    audience: common::ids::AudienceId,
    subscription: common::ids::SubscriptionId,
) {
    let outcome = write_scope
        .run(move |transaction| {
            Box::pin(async move {
                audiences
                    .add_member(transaction, author, audience, subscription)
                    .await
            })
        })
        .await
        .expect("audience membership fixture setup should succeed");
    confirmed(outcome, "audience membership fixture setup");
}
