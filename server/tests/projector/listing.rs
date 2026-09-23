use std::sync::Arc;

use axum::http::{StatusCode, header};
use tower::ServiceExt;

use common::registration::RegistrationPolicy;
use common::theme::{PublishedThemePresentation, Theme};
use common::time::{PermalinkDate, UtcInstant};
use common::visibility::ViewerIdentity;
use common::{
    seed::{Page, PageSeed, PublicPresentation},
    site::SiteIdentity,
};
use rstest::*;
use rstest_reuse::*;

use crate::helpers::body_string;

use storage::{
    MockSiteConfigStorage, MockUserStorage, SiteConfigStorage, UserStorage,
    test_support::{Backend, backends},
};

use super::fixtures::{
    TEST_SHELL, assert_sanitized_internal_server_error, assert_shell_miss,
    failing_site_theme_selection, get, projector_app, projector_app_with_dependencies,
    projector_app_with_site_config, seed_published_post, seed_tagged_post,
};

#[apply(backends)]
#[tokio::test]
async fn profile_projects_user_timeline(#[case] backend: Backend) {
    let env = backend.setup().await;
    let (u, .., title, _rendered_html) =
        seed_published_post(env.users(), env.posts(), env.write_scope()).await;
    let resp = projector_app(env.posts(), env.users(), env.themes())
        .oneshot(get(&format!("/~{u}")))
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::OK, "profile → 200");
    let html = body_string(resp).await;
    assert!(
        html.contains(&format!("Posts by {u}")),
        "profile heading: {html}"
    );
    assert!(html.contains(title.as_ref()), "post title present");
    assert!(html.contains(r#"id="jaunder-seed""#), "data blob present");
}

#[apply(backends)]
#[tokio::test]
async fn local_discovery_projects_three_existing_syndication_formats(#[case] backend: Backend) {
    let env = backend.setup().await;
    let response = projector_app(env.posts(), env.users(), env.themes())
        .oneshot(get("/feeds"))
        .await
        .expect("request");
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_string(response).await;
    assert!(html.contains("Syndication feeds for Local"), "{html}");
    for (label, url) in [
        ("RSS", "/feed.rss"),
        ("Atom", "/feed.atom"),
        ("JSON Feed", "/feed.json"),
    ] {
        assert!(
            html.contains(&format!(r#"href="{url}">{label}</a>"#)),
            "{html}"
        );
    }
    assert!(html.contains(r#""FeedDiscovery":"Site""#), "{html}");
}

#[apply(backends)]
#[tokio::test]
async fn contextual_discovery_links_match_each_timeline(#[case] backend: Backend) {
    let env = backend.setup().await;
    let (username, _) = seed_tagged_post(env.users(), env.posts(), env.write_scope()).await;
    let app = projector_app(env.posts(), env.users(), env.themes());
    for (path, context, prefix) in [
        (
            "/tags/rust/feeds".to_owned(),
            "site tag #rust".to_owned(),
            "/tags/rust/feed".to_owned(),
        ),
        (
            format!("/~{username}/feeds"),
            format!("User ~{username}"),
            format!("/~{username}/feed"),
        ),
        (
            format!("/~{username}/tags/rust/feeds"),
            format!("User ~{username} tag #rust"),
            format!("/~{username}/tags/rust/feed"),
        ),
    ] {
        let response = app.clone().oneshot(get(&path)).await.expect("request");
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        let html = body_string(response).await;
        assert!(
            html.contains(&format!("Syndication feeds for {context}")),
            "{html}"
        );
        for (extension, label) in [("rss", "RSS"), ("atom", "Atom"), ("json", "JSON Feed")] {
            assert!(
                html.contains(&format!(r#"href="{prefix}.{extension}">{label}</a>"#)),
                "{html}"
            );
        }
        assert!(
            !html.contains("rel=\"EditURI\""),
            "discovery index is not the profile: {html}"
        );
    }
}

#[apply(backends)]
#[tokio::test]
async fn discovery_preserves_empty_and_missing_context_semantics(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = projector_app(env.posts(), env.users(), env.themes());
    for (path, context) in [
        ("/tags/unused/feeds", "site tag #unused"),
        ("/~nobody/feeds", "User ~nobody"),
    ] {
        let response = app.clone().oneshot(get(path)).await.expect("request");
        assert_eq!(response.status(), StatusCode::OK);
        let html = body_string(response).await;
        assert!(
            html.contains(&format!("Syndication feeds for {context}")),
            "{html}"
        );
    }
    for path in [
        "/~nobody/tags/unused/feeds",
        "/tags/%20/feeds",
        "/~%20/feeds",
    ] {
        assert_shell_miss(app.clone().oneshot(get(path)).await.expect("request")).await;
    }
}

#[apply(backends)]
#[tokio::test]
async fn site_timeline_projects_local_posts(#[case] backend: Backend) {
    let env = backend.setup().await;
    let (.., title, _rendered_html) =
        seed_published_post(env.users(), env.posts(), env.write_scope()).await;
    let resp = projector_app(env.posts(), env.users(), env.themes())
        .oneshot(get("/"))
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::OK, "root site timeline → 200");
    let html = body_string(resp).await;
    assert!(html.contains(title.as_ref()), "post present: {html}");
    assert!(html.contains(r#"id="jaunder-seed""#), "data blob present");
    assert!(
        html.contains(r#"data-jaunder-part="site-title">Jaunder"#),
        "Local body identity: {html}"
    );
    assert!(
        html.contains(r#""identity":{"title":"Jaunder"#),
        "Local seed identity: {html}"
    );
    assert_eq!(
        html.matches("data-jaunder-projected-local-metadata")
            .count(),
        4,
        "Local head metadata cardinality: {html}"
    );
    assert!(
        html.contains(r#"name="description" content="""#),
        "absent tagline leaves standard description empty: {html}"
    );
    assert!(
        html.contains(r#"property="og:description" content="""#),
        "absent tagline leaves Open Graph description empty: {html}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn site_timeline_projects_register_only_for_open_policy(#[case] backend: Backend) {
    let env = backend.setup().await;
    for (policy, expect_register) in [
        (RegistrationPolicy::Closed, false),
        (RegistrationPolicy::OperatorInvites, false),
        (RegistrationPolicy::MemberInvites, false),
        (RegistrationPolicy::Open, true),
    ] {
        let mut site_config = MockSiteConfigStorage::new();
        site_config.expect_get_identity().times(1).return_once(|| {
            Ok(SiteIdentity {
                title: "Jaunder".parse().unwrap(),
                tagline: None,
                base_url: None,
            })
        });
        site_config
            .expect_get_registration_policy()
            .times(1)
            .returning(move || Ok(policy));

        let response = projector_app_with_site_config(
            env.posts(),
            env.users(),
            env.themes(),
            Arc::new(site_config) as Arc<dyn SiteConfigStorage>,
        )
        .oneshot(get("/"))
        .await
        .expect("request");
        assert_eq!(response.status(), StatusCode::OK);
        let html = body_string(response).await;
        assert!(html.contains(">Sign in</a>"), "{policy:?}: {html}");
        assert_eq!(
            html.contains(">Register</a>"),
            expect_register,
            "{policy:?}: {html}"
        );
        assert!(
            html.contains(&format!(r#""registration_policy":"{}""#, policy.as_ref())),
            "same policy must travel in the seed for {policy:?}: {html}"
        );
    }
}

#[apply(backends)]
#[tokio::test]
async fn site_timeline_resolves_one_configured_identity_for_head_body_and_seed(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let mut site_config = MockSiteConfigStorage::new();
    site_config.expect_get_identity().times(1).return_once(|| {
        Ok(SiteIdentity {
            title: "Jaunder <Sandbox>".parse().unwrap(),
            tagline: Some("Thoughtful & <publishing>.".parse().unwrap()),
            base_url: None,
        })
    });
    site_config
        .expect_get_registration_policy()
        .times(1)
        .returning(|| Ok(RegistrationPolicy::Open));

    let response = projector_app_with_site_config(
        env.posts(),
        env.users(),
        env.themes(),
        Arc::new(site_config) as Arc<dyn SiteConfigStorage>,
    )
    .oneshot(get("/"))
    .await
    .expect("request");
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_string(response).await;

    assert!(
        html.contains(
            r"<title data-jaunder-projected-local-metadata>Jaunder &lt;Sandbox&gt;</title>"
        ),
        "configured title in head: {html}"
    );
    assert!(
        html.contains(r#"name="description" content="Thoughtful &amp; &lt;publishing&gt;.""#),
        "configured tagline in standard metadata: {html}"
    );
    assert!(
        html.contains(r#"property="og:title" content="Jaunder &lt;Sandbox&gt;""#),
        "configured title in Open Graph metadata: {html}"
    );
    assert!(
        html.contains(
            r#"property="og:description" content="Thoughtful &amp; &lt;publishing&gt;.""#
        ),
        "configured tagline in Open Graph metadata: {html}"
    );
    assert!(
        html.contains(r#"data-jaunder-part="site-title">Jaunder &lt;Sandbox&gt;</span>"#),
        "same configured title in projected body: {html}"
    );
    assert!(
        html.contains(r#"<div class="j-sub">Thoughtful &amp; &lt;publishing&gt;.</div>"#),
        "same configured tagline in projected body: {html}"
    );
    assert!(
        html.contains(
            r#""identity":{"title":"Jaunder <Sandbox>","tagline":"Thoughtful & <publishing>.","base_url":null}"#
        ),
        "same configured identity in seed: {html}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn site_timeline_maps_identity_storage_failure_at_the_projector_boundary(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let mut site_config = MockSiteConfigStorage::new();
    site_config
        .expect_get_identity()
        .times(1)
        .return_once(|| Err(sqlx::Error::PoolClosed));

    let response = projector_app_with_site_config(
        env.posts(),
        env.users(),
        env.themes(),
        Arc::new(site_config) as Arc<dyn SiteConfigStorage>,
    )
    .oneshot(get("/"))
    .await
    .expect("request");

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_sanitized_internal_server_error(response).await;
}

#[apply(backends)]
#[tokio::test]
async fn site_timeline_maps_registration_policy_failure_at_the_projector_boundary(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let mut site_config = MockSiteConfigStorage::new();
    site_config.expect_get_identity().times(1).return_once(|| {
        Ok(SiteIdentity {
            title: "Jaunder".parse().unwrap(),
            tagline: None,
            base_url: None,
        })
    });
    site_config
        .expect_get_registration_policy()
        .times(1)
        .return_once(|| Err(sqlx::Error::PoolClosed));

    let response = projector_app_with_site_config(
        env.posts(),
        env.users(),
        env.themes(),
        Arc::new(site_config) as Arc<dyn SiteConfigStorage>,
    )
    .oneshot(get("/"))
    .await
    .expect("request");

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_sanitized_internal_server_error(response).await;
}

/// The malformed-row case crosses the real Local projector boundary rather than
/// stopping at `SiteConfigStorage::get_identity`: it must retain the cacheable
/// default-title projection while emitting no tagline presentation or metadata.
#[apply(backends)]
#[tokio::test]
async fn site_timeline_treats_malformed_persisted_tagline_as_absent(#[case] backend: Backend) {
    let env = backend.setup().await;
    env.inject_invalid_site_config(
        host::config_key::SiteConfigKey::SiteTagline,
        "invalid\u{2028}persisted tagline",
    )
    .await
    .expect("inject malformed legacy tagline");

    let response =
        projector_app_with_site_config(env.posts(), env.users(), env.themes(), env.site_config())
            .oneshot(get("/"))
            .await
            .expect("request");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL),
        Some(&header::HeaderValue::from_static("public, max-age=300")),
        "invalid optional identity data does not alter Local's five-minute cache policy"
    );
    let html = body_string(response).await;
    assert!(
        html.contains(r#"data-jaunder-part="site-title">Jaunder"#),
        "invalid persisted tagline retains the default title: {html}"
    );
    assert!(!html.contains("j-sub"), "no Local tagline element: {html}");
    assert!(
        html.contains(r#"name="description" content="""#)
            && html.contains(r#"property="og:description" content="""#),
        "invalid persisted tagline leaves descriptions absent: {html}"
    );
    assert!(
        !html.contains(r#""tagline":"#),
        "the Local seed resolves malformed optional tagline to absent: {html}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn direct_order_urls_embed_matching_seed_order(#[case] backend: Backend) {
    let env = backend.setup().await;
    let (username, _) = seed_tagged_post(env.users(), env.posts(), env.write_scope()).await;

    for route in [
        "/".to_owned(),
        format!("/~{username}"),
        "/tags/rust".to_owned(),
        format!("/~{username}/tags/rust"),
    ] {
        for (query, order) in [
            ("", "newest"),
            ("?order=oldest", "oldest"),
            ("?order=unknown", "newest"),
        ] {
            let uri = format!("{route}{query}");
            let response = projector_app(env.posts(), env.users(), env.themes())
                .oneshot(get(&uri))
                .await
                .expect("request");
            assert_eq!(response.status(), StatusCode::OK, "{uri}");
            let body = body_string(response).await;
            assert!(
                body.contains(&format!(r#""order":"{order}""#)),
                "{uri} must seed {order}: {body}"
            );
        }
    }
}

#[apply(backends)]
#[tokio::test]
async fn profile_invalid_username_serves_shell(#[case] backend: Backend) {
    let env = backend.setup().await;
    let resp = projector_app(env.posts(), env.users(), env.themes())
        .oneshot(get("/~in.valid"))
        .await
        .expect("request");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "unparseable username → shell"
    );
    let body = body_string(resp).await;
    assert!(body.contains("test-shell"));
}

#[apply(backends)]
#[tokio::test]
async fn profile_unknown_valid_username_is_cacheable_projection(#[case] backend: Backend) {
    let env = backend.setup().await;
    let resp = projector_app(env.posts(), env.users(), env.themes())
        .oneshot(get("/~ghost"))
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::OK, "unknown profile → 200");
    assert_eq!(
        resp.headers()
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("public, max-age=300"),
        "valid unknown username projects an empty cacheable profile"
    );
    let html = body_string(resp).await;
    assert!(html.contains("Posts by ghost"), "profile heading: {html}");
    assert!(html.contains(r#"id="jaunder-seed""#), "data blob present");
}

#[apply(backends)]
#[tokio::test]
async fn site_timeline_storage_failure_keeps_500_and_reports_boundary_once(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    seed_published_post(env.users(), env.posts(), env.write_scope()).await;
    let app = projector_app(env.posts(), env.users(), env.themes());
    env.base.close_pool().await;

    let (response, event) = crate::assert_error_signal!(
        async { app.oneshot(get("/")).await.expect("request") },
        event = "server function failed",
        event_kind = "Storage",
        event_class = "Bug",
        metric_kind = "storage",
        metric_class = "bug",
        disposition = "boundary",
        context = ""
    );

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        response.headers().get(header::CACHE_CONTROL).is_none(),
        "500 is not cached"
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    assert!(body.is_empty(), "500 body remains sanitized");
    assert!(event.contains("pool"), "typed storage source: {event}");
}

#[apply(backends)]
#[tokio::test]
async fn site_timeline_theme_failure_keeps_500_and_reports_boundary_once(#[case] backend: Backend) {
    let env = backend.setup().await;
    seed_published_post(env.users(), env.posts(), env.write_scope()).await;
    let app = projector_app_with_dependencies(
        env.posts(),
        env.users(),
        failing_site_theme_selection("injected site timeline theme failure"),
    );

    let (response, event) = crate::assert_error_signal!(
        async { app.oneshot(get("/")).await.expect("request") },
        event = "server function failed",
        event_kind = "Storage",
        event_class = "Bug",
        metric_kind = "storage",
        metric_class = "bug",
        disposition = "boundary",
        context = "server.projector.timeline_theme"
    );

    assert_sanitized_internal_server_error(response).await;
    assert!(event.contains("injected site timeline theme failure"));
}

#[apply(backends)]
#[tokio::test]
async fn profile_storage_failure_keeps_no_store_shell_and_reports_once(#[case] backend: Backend) {
    let env = backend.setup().await;
    let (username, ..) = seed_published_post(env.users(), env.posts(), env.write_scope()).await;
    let app = projector_app(env.posts(), env.users(), env.themes());
    env.base.close_pool().await;

    let (response, event) = crate::assert_error_signal!(
        async {
            app.oneshot(get(&format!("/~{username}")))
                .await
                .expect("request")
        },
        event = "error swallowed after reporting",
        event_kind = "storage",
        event_class = "bug",
        metric_kind = "storage",
        metric_class = "bug",
        disposition = "swallowed",
        context = "server.projector.profile"
    );

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    assert_eq!(body.as_ref(), TEST_SHELL.as_bytes(), "exact CSR shell body");
    assert!(event.contains("pool"), "typed storage source: {event}");
}

#[apply(backends)]
#[tokio::test]
async fn profile_owner_lookup_failure_keeps_500_and_reports_boundary_once(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let (username, ..) = seed_published_post(env.users(), env.posts(), env.write_scope()).await;
    let mut users = MockUserStorage::new();
    users
        .expect_get_user_by_username()
        .times(1)
        .return_once(|_| {
            Err(sqlx::Error::Io(std::io::Error::other(
                "injected owner lookup failure",
            )))
        });
    let app = projector_app_with_dependencies(
        env.posts(),
        Arc::new(users) as Arc<dyn UserStorage>,
        env.themes(),
    );

    let (response, event) = crate::assert_error_signal!(
        async {
            app.oneshot(get(&format!("/~{username}")))
                .await
                .expect("request")
        },
        event = "server function failed",
        event_kind = "Storage",
        event_class = "Bug",
        metric_kind = "storage",
        metric_class = "bug",
        disposition = "boundary",
        context = "server.projector.profile"
    );

    assert_sanitized_internal_server_error(response).await;
    assert!(event.contains("injected owner lookup failure"));
}

#[apply(backends)]
#[tokio::test]
async fn profile_theme_failure_keeps_500_and_reports_boundary_once(#[case] backend: Backend) {
    let env = backend.setup().await;
    let (username, ..) = seed_published_post(env.users(), env.posts(), env.write_scope()).await;
    let app = projector_app_with_dependencies(
        env.posts(),
        env.users(),
        failing_site_theme_selection("injected profile theme failure"),
    );

    let (response, event) = crate::assert_error_signal!(
        async {
            app.oneshot(get(&format!("/~{username}")))
                .await
                .expect("request")
        },
        event = "server function failed",
        event_kind = "Storage",
        event_class = "Bug",
        metric_kind = "storage",
        metric_class = "bug",
        disposition = "boundary",
        context = "server.projector.profile"
    );

    assert_sanitized_internal_server_error(response).await;
    assert!(event.contains("injected profile theme failure"));
}

#[apply(backends)]
#[tokio::test]
async fn every_page_seed_variant_serializes_without_null_fallback(#[case] backend: Backend) {
    let env = backend.setup().await;
    let (username, year, month, day, slug, ..) =
        seed_published_post(env.users(), env.posts(), env.write_scope()).await;
    let username = username.parse().expect("seeded username");
    let slug = slug.parse().expect("seeded slug");
    let date = PermalinkDate::from_ymd(year, month, day).expect("seeded date");
    let record = storage::fetch_post_record(
        env.posts().as_ref(),
        &ViewerIdentity::Anonymous,
        &username,
        date,
        &slug,
        UtcInstant::now(),
    )
    .await
    .expect("permalink lookup")
    .expect("seeded post");
    let page = Page {
        posts: Vec::new(),
        next_cursor: None,
        has_more: false,
    };
    let tag: common::tag::Tag = "rust".parse().expect("representative tag");
    let seeds = [
        PageSeed::SiteTimeline {
            identity: SiteIdentity {
                title: "Jaunder".parse().unwrap(),
                tagline: None,
                base_url: None,
            },
            registration_policy: RegistrationPolicy::Open,
            order: common::seed::TimelineOrder::Newest,
            page: page.clone(),
        },
        PageSeed::Profile {
            username: username.clone(),
            order: common::seed::TimelineOrder::Newest,
            page: page.clone(),
        },
        PageSeed::SiteTag {
            tag: tag.clone(),
            order: common::seed::TimelineOrder::Newest,
            page: page.clone(),
        },
        PageSeed::UserTag {
            username,
            tag,
            order: common::seed::TimelineOrder::Newest,
            page,
        },
        PageSeed::Permalink(web::posts::public_authored_post(record, false)),
        PageSeed::FeedDiscovery(common::feed::FeedSurface::Site),
    ];

    for seed in seeds {
        // No wildcard: adding a PageSeed variant makes this proof fail to compile
        // until a representative is added. Every closed field is a derived
        // string/integer/sequence/newtype serializer; none is a fallible map key
        // or custom serializer, so `null` remains defensive only.
        let variant = match &seed {
            PageSeed::SiteTimeline { .. } => "site timeline",
            PageSeed::Profile { .. } => "profile",
            PageSeed::SiteTag { .. } => "site tag",
            PageSeed::UserTag { .. } => "user tag",
            PageSeed::Permalink(_) => "permalink",
            PageSeed::FeedDiscovery(_) => "feed discovery",
        };
        let presentation = PublicPresentation {
            theme: PublishedThemePresentation::built_in(Theme::Studio),
            page: seed,
        };
        let json = serde_json::to_string(&presentation)
            .unwrap_or_else(|error| panic!("{variant} must serialize: {error}"));
        assert_ne!(json, "null", "{variant}");
        let document = jaunder::projector::document_presentation(&presentation);
        assert!(
            !document.contains(r#"id="jaunder-seed">null</script>"#),
            "{variant} selected the defensive null fallback"
        );
    }
}
