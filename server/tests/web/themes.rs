use std::{collections::BTreeMap, fs, sync::Arc};

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use common::{
    MutationOutcome,
    theme::{PublicThemeRoute, PublicThemeSelection, Theme, ThemeImageRole},
};
use host::theme_package::{ThemePackageLimits, export_theme_package, validate_theme_package};
use rstest::*;
use rstest_reuse::*;
use server_fn::ServerFn;
use storage::test_support::{Backend, backends, backends_matrix, confirmed_for, seed_media};
use storage::{ThemeManager, ThemeOwner, ThemePoolInput as StorageThemePoolInput};
use tempfile::TempDir;
use tower::ServiceExt;
use web::themes::{Draft, OwnershipScope, ThemeBindingInput, ThemeMediaInput, ThemePoolInput};

use crate::helpers::{
    body_string, create_operator_and_session, create_user_and_session, make_app, post_server_fn,
};

const NO_STORE: &str = "private, no-store";

#[apply(backends)]
#[tokio::test]
async fn editor_preview_unsaved_new_theme_is_read_only(#[case] backend: Backend) {
    let env = backend.setup().await;
    let actor = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let input = serde_json::json!({
        "scope": "author",
        "input": {
            "name": "Unsaved theme",
            "stylesheet": "body { --editor-proof: unsaved-current; color: rebeccapurple; }",
            "logo": {"kind":"set", "value":{"kind":"explicit_absent"}},
            "header": {"kind":"set", "value":{"kind":"packaged_default"}}
        }
    });
    let request = Request::builder()
        .method("POST")
        .uri("/api/themes/preview_editor")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::COOKIE, actor.cookie())
        .body(Body::from(serde_qs::to_string(&input).unwrap()))
        .unwrap();
    let response = make_app!(&env, &env.base).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], NO_STORE);
    let preview: web::themes::ThemePreview =
        serde_json::from_str(&body_string(response).await).unwrap();
    assert!(
        preview.css.contains("--editor-proof:unsaved-current"),
        "{}",
        preview.css
    );
    assert!(preview.html.contains("data-jaunder-theme-surface"));
    let owner = ThemeOwner::Author(actor.user_id);
    assert!(env.themes().list_themes(owner).await.unwrap().is_empty());
    assert_eq!(env.themes().selection(owner).await.unwrap(), None);
}

#[apply(backends)]
#[tokio::test]
async fn editor_save_creates_private_then_applies_only_when_selected(#[case] backend: Backend) {
    let env = backend.setup().await;
    let actor = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let owner = ThemeOwner::Author(actor.user_id);
    let app = make_app!(&env, &env.base);
    let form = |name: &str, css: &str| {
        serde_json::json!({
            "scope": "author", "input": {"name": name, "stylesheet": css,
                "logo": {"kind":"set", "value":{"kind":"explicit_absent"}},
                "header": {"kind":"set", "value":{"kind":"packaged_default"}}}
        })
    };
    let response = editor_response(
        app.clone(),
        "save_editor",
        form("Editor theme", "body { color: red; }"),
        &actor.cookie(),
    )
    .await;
    let status = response.status();
    let body = body_string(response).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let saved: serde_json::Value = serde_json::from_str(&body).unwrap();
    let saved = &saved["Confirmed"];
    let id: common::ids::ThemeId = serde_json::from_value(saved["id"].clone()).unwrap();
    assert_eq!(saved["applied"], false);
    assert!(
        env.themes()
            .list_revisions(owner, id)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(env.themes().selection(owner).await.unwrap(), None);
    for operation in ["publish", "select"] {
        // Use the typed existing selection API so this test does not duplicate its wire encoding.
        let (status, body) = if operation == "publish" {
            post_server_fn(
                app.clone(),
                &web::themes::Publish {
                    scope: OwnershipScope::Author,
                    theme_id: id,
                },
                Some(&actor.cookie()),
            )
            .await
        } else {
            post_server_fn(
                app.clone(),
                &web::themes::Select {
                    scope: OwnershipScope::Author,
                    selection: Some(PublicThemeSelection::Custom(id)),
                },
                Some(&actor.cookie()),
            )
            .await
        };
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let previous = env.themes().list_revisions(owner, id).await.unwrap();
    let mut input = form("Renamed editor", "body { color: blue; }");
    input["target"] = serde_json::json!(id);
    let response =
        editor_response(app.clone(), "save_editor", input.clone(), &actor.cookie()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let saved: serde_json::Value = serde_json::from_str(&body_string(response).await).unwrap();
    assert_eq!(saved["Confirmed"]["applied"], true);
    assert_eq!(
        env.themes().selection(owner).await.unwrap(),
        Some(PublicThemeSelection::Custom(id))
    );
    let published = env.themes().list_revisions(owner, id).await.unwrap();
    assert_eq!(published.len(), previous.len() + 1);
    post_server_fn(
        app.clone(),
        &web::themes::Select {
            scope: OwnershipScope::Author,
            selection: None,
        },
        Some(&actor.cookie()),
    )
    .await;
    input["input"]["stylesheet"] = serde_json::json!("body { color: green; }");
    let response = editor_response(app, "save_editor", input, &actor.cookie()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let saved: serde_json::Value = serde_json::from_str(&body_string(response).await).unwrap();
    assert_eq!(saved["Confirmed"]["applied"], false);
    assert_eq!(
        env.themes().list_revisions(owner, id).await.unwrap(),
        published
    );
    assert_eq!(env.themes().selection(owner).await.unwrap(), None);
}

#[apply(backends)]
#[tokio::test]
async fn editor_preserves_pool_and_media_provenance_until_explicit_header_replacement(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let original =
        create_operator_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let editor = create_operator_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let app = make_app!(&env, &env.base);
    let (status, body) = post_server_fn(
        app.clone(),
        &web::themes::Create {
            scope: OwnershipScope::Site,
            name: "Pooled editor".into(),
            draft: package_draft("body { color: red; }"),
        },
        Some(&original.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let entry: web::themes::CatalogEntry = crate::helpers::confirmed_mutation(&body);
    let media = seed_media(
        env.media(),
        env.write_scope(),
        original.user_id,
        "editor-header.png",
    )
    .await;
    let manager = ThemeManager::new(
        env.themes(),
        env.media(),
        env.write_scope(),
        Arc::new(env.media_content_locks()),
    );
    manager
        .replace_role(
            original.user_id,
            ThemeOwner::Site,
            entry.id,
            ThemeImageRole::Logo,
            storage::ThemeRoleInput::Media(media.clone()),
        )
        .await
        .unwrap();
    manager
        .replace_header_pool(
            original.user_id,
            ThemeOwner::Site,
            entry.id,
            vec![StorageThemePoolInput::Media(media.clone())],
            [7; 32],
        )
        .await
        .unwrap();
    let before = manager
        .copy_source(ThemeOwner::Site, entry.id)
        .await
        .unwrap();
    let keep = web::themes::ThemeEditorInput {
        name: "Pooled renamed".into(),
        stylesheet: "body { --editor-proof: pooled-current; color: blue; }".into(),
        logo: web::themes::ThemeEditorPatch::Keep,
        header: web::themes::ThemeEditorPatch::Keep,
    };
    let response = server_fn_response(
        app.clone(),
        &web::themes::PreviewEditor {
            scope: OwnershipScope::Site,
            target: Some(entry.id),
            input: keep.clone(),
        },
        Some(&editor.cookie()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let preview: web::themes::ThemePreview =
        serde_json::from_str(&body_string(response).await).unwrap();
    assert!(preview.html.contains("editor-header.png"));
    assert!(preview.html.contains("data-jaunder-part=\"header-image\""));
    assert!(preview.css.contains("pooled-current"));
    assert_eq!(
        manager
            .copy_source(ThemeOwner::Site, entry.id)
            .await
            .unwrap(),
        before
    );
    let (status, body) = post_server_fn(
        app.clone(),
        &web::themes::SaveEditor {
            scope: OwnershipScope::Site,
            target: Some(entry.id),
            input: keep.clone(),
        },
        Some(&editor.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let after_keep = manager
        .copy_source(ThemeOwner::Site, entry.id)
        .await
        .unwrap();
    assert_eq!(after_keep.bindings, before.bindings);
    assert_eq!(after_keep.header_pool, before.header_pool);
    assert_eq!(after_keep.entry.name, "Pooled renamed");
    assert_eq!(after_keep.draft.manifest, before.draft.manifest);
    assert_eq!(after_keep.draft.assets, before.draft.assets);
    assert_ne!(after_keep.draft.stylesheet, before.draft.stylesheet);
    let mut fixed = keep;
    fixed.header = web::themes::ThemeEditorPatch::Set(ThemeBindingInput::ExplicitAbsent);
    let (status, body) = post_server_fn(
        app.clone(),
        &web::themes::PreviewEditor {
            scope: OwnershipScope::Site,
            target: Some(entry.id),
            input: fixed.clone(),
        },
        Some(&editor.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let preview: web::themes::ThemePreview = serde_json::from_str(&body).unwrap();
    assert!(!preview.html.contains("data-jaunder-part=\"header-image\""));
    assert_eq!(
        manager
            .copy_source(ThemeOwner::Site, entry.id)
            .await
            .unwrap(),
        after_keep
    );
    // A late name conflict must roll back every field, including the header mode.
    let (status, _) = post_server_fn(
        app.clone(),
        &web::themes::Create {
            scope: OwnershipScope::Site,
            name: "Name collision".into(),
            draft: package_draft("body { color: black; }"),
        },
        Some(&original.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    fixed.name = "Name collision".into();
    let (status, _) = post_server_fn(
        app.clone(),
        &web::themes::SaveEditor {
            scope: OwnershipScope::Site,
            target: Some(entry.id),
            input: fixed.clone(),
        },
        Some(&editor.cookie()),
    )
    .await;
    assert_ne!(status, StatusCode::OK);
    assert_eq!(
        manager
            .copy_source(ThemeOwner::Site, entry.id)
            .await
            .unwrap(),
        after_keep
    );
    fixed.name = "Fixed header".into();
    let (status, body) = post_server_fn(
        app,
        &web::themes::SaveEditor {
            scope: OwnershipScope::Site,
            target: Some(entry.id),
            input: fixed,
        },
        Some(&original.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let after_set = manager
        .copy_source(ThemeOwner::Site, entry.id)
        .await
        .unwrap();
    assert!(after_set.header_pool.is_empty());
    assert!(after_set.bindings.iter().any(|binding| matches!(
        binding,
        storage::ThemeRoleBinding::ExplicitAbsent {
            role: ThemeImageRole::Header,
            ..
        }
    )));
    assert_eq!(after_set.bindings[0], before.bindings[0]);
}

#[apply(backends_matrix)]
#[case("missing")]
#[case("new_keep")]
#[case("media")]
#[case("css")]
#[case("name")]
#[case("asset")]
#[tokio::test]
async fn editor_rejects_foreign_anonymous_site_and_invalid_inputs_without_writes(
    backend: Backend,
    #[case] invalid: &str,
) {
    let env = backend.setup().await;
    let actor = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let foreign = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let app = make_app!(&env, &env.base);
    let entry = create_author_theme(
        app.clone(),
        &actor.cookie(),
        "Editor access",
        "body { color: red; }",
    )
    .await;
    let manager = ThemeManager::new(
        env.themes(),
        env.media(),
        env.write_scope(),
        Arc::new(env.media_content_locks()),
    );
    let owner = ThemeOwner::Author(actor.user_id);
    let before = manager.copy_source(owner, entry.id).await.unwrap();
    let input = web::themes::ThemeEditorInput {
        name: "Changed".into(),
        stylesheet: "body { color: blue; }".into(),
        logo: web::themes::ThemeEditorPatch::Keep,
        header: web::themes::ThemeEditorPatch::Keep,
    };
    for cookie in [None, Some(foreign.cookie()), Some(actor.cookie())] {
        let scope = if cookie.as_ref() == Some(&actor.cookie()) {
            OwnershipScope::Site
        } else {
            OwnershipScope::Author
        };
        let (status, _) = post_server_fn(
            app.clone(),
            &web::themes::PreviewEditor {
                scope,
                target: Some(entry.id),
                input: input.clone(),
            },
            cookie.as_deref(),
        )
        .await;
        assert_ne!(status, StatusCode::OK);
        let (status, _) = post_server_fn(
            app.clone(),
            &web::themes::SaveEditor {
                scope,
                target: Some(entry.id),
                input: input.clone(),
            },
            cookie.as_deref(),
        )
        .await;
        assert_ne!(status, StatusCode::OK);
    }
    let media = seed_media(
        env.media(),
        env.write_scope(),
        foreign.user_id,
        "foreign-editor.png",
    )
    .await;
    let mut foreign_media = input.clone();
    foreign_media.logo =
        web::themes::ThemeEditorPatch::Set(ThemeBindingInput::Media(ThemeMediaInput {
            source: media.source,
            sha256: media.sha256,
            filename: media.filename,
        }));
    let mut invalid_css = input.clone();
    invalid_css.stylesheet = "@import url(https://example.com/evil.css);".into();
    let mut invalid_name = input.clone();
    invalid_name.name = "Studio".into();
    let mut invalid_asset = input.clone();
    invalid_asset.header =
        web::themes::ThemeEditorPatch::Set(ThemeBindingInput::PackageAsset("missing.png".into()));
    let target = match invalid {
        "missing" => Some(common::ids::ThemeId::from(i64::MAX)),
        "new_keep" => None,
        _ => Some(entry.id),
    };
    let input = match invalid {
        "media" => foreign_media,
        "css" => invalid_css,
        "name" => invalid_name,
        "asset" => invalid_asset,
        "missing" | "new_keep" => input,
        _ => unreachable!("declared invalid input cases"),
    };
    {
        let (status, body) = post_server_fn(
            app.clone(),
            &web::themes::PreviewEditor {
                scope: OwnershipScope::Author,
                target,
                input: input.clone(),
            },
            Some(&actor.cookie()),
        )
        .await;
        assert_ne!(status, StatusCode::OK);
        assert!(!body.contains("rate limited"), "{body}");
        let (status, body) = post_server_fn(
            app.clone(),
            &web::themes::SaveEditor {
                scope: OwnershipScope::Author,
                target,
                input,
            },
            Some(&actor.cookie()),
        )
        .await;
        assert_ne!(status, StatusCode::OK);
        assert!(!body.contains("rate limited"), "{body}");
    }
    assert_eq!(manager.copy_source(owner, entry.id).await.unwrap(), before);
    assert_eq!(env.themes().list_themes(owner).await.unwrap().len(), 1);
    assert_eq!(env.themes().selection(owner).await.unwrap(), None);
}

#[apply(backends_matrix)]
#[case("default")]
#[case("absent")]
#[case("package")]
#[case("media")]
#[tokio::test]
async fn editor_preview_and_save_share_image_choice_semantics(
    backend: Backend,
    #[case] choice: &str,
) {
    let env = backend.setup().await;
    let actor = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let app = make_app!(&env, &env.base);
    let response = multipart_response(
        app.clone(),
        multipart_import(
            "author",
            Some("Image editor"),
            &archive_with_package_defaults("body { color: red; }"),
        ),
        &actor.cookie(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let entry: web::themes::CatalogEntry =
        crate::helpers::confirmed_mutation(&body_string(response).await);
    let manager = ThemeManager::new(
        env.themes(),
        env.media(),
        env.write_scope(),
        Arc::new(env.media_content_locks()),
    );
    let owner = ThemeOwner::Author(actor.user_id);
    let before = manager.copy_source(owner, entry.id).await.unwrap();
    let reference = seed_media(
        env.media(),
        env.write_scope(),
        actor.user_id,
        "chosen-editor.png",
    )
    .await;
    let binding = match choice {
        "default" => ThemeBindingInput::PackagedDefault,
        "absent" => ThemeBindingInput::ExplicitAbsent,
        "package" => ThemeBindingInput::PackageAsset("assets/logo.png".into()),
        "media" => ThemeBindingInput::Media(ThemeMediaInput {
            source: reference.source,
            sha256: reference.sha256,
            filename: reference.filename,
        }),
        _ => unreachable!("declared image choices"),
    };
    let input = web::themes::ThemeEditorInput {
        name: "Images renamed".into(),
        stylesheet: "body { --editor-image: current; color: blue; }".into(),
        logo: web::themes::ThemeEditorPatch::Set(binding.clone()),
        header: web::themes::ThemeEditorPatch::Set(binding.clone()),
    };
    let (status, body) = post_server_fn(
        app.clone(),
        &web::themes::PreviewEditor {
            scope: OwnershipScope::Author,
            target: Some(entry.id),
            input: input.clone(),
        },
        Some(&actor.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let preview: web::themes::ThemePreview = serde_json::from_str(&body).unwrap();
    assert!(preview.css.contains("--editor-image:current"));
    assert_eq!(
        preview.html.contains("data-jaunder-part=\"header-image\""),
        choice != "absent"
    );
    if choice == "media" {
        assert!(preview.html.contains("chosen-editor.png"));
    }
    if choice == "package" {
        assert!(preview.html.contains("/assets/logo.png"));
    }
    assert_eq!(manager.copy_source(owner, entry.id).await.unwrap(), before);
    let (status, body) = post_server_fn(
        app.clone(),
        &web::themes::SaveEditor {
            scope: OwnershipScope::Author,
            target: Some(entry.id),
            input,
        },
        Some(&actor.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let after = manager.copy_source(owner, entry.id).await.unwrap();
    assert_eq!(after.draft.manifest, before.draft.manifest);
    assert_eq!(after.draft.assets, before.draft.assets);
    let (status, body) = post_server_fn(
        app,
        &web::themes::PreviewEditor {
            scope: OwnershipScope::Author,
            target: Some(entry.id),
            input: web::themes::ThemeEditorInput {
                name: after.entry.name,
                stylesheet: String::from_utf8(after.draft.stylesheet).unwrap(),
                logo: web::themes::ThemeEditorPatch::Keep,
                header: web::themes::ThemeEditorPatch::Keep,
            },
        },
        Some(&actor.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        serde_json::from_str::<web::themes::ThemePreview>(&body).unwrap(),
        preview
    );
    assert!(
        env.themes()
            .list_revisions(owner, entry.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(env.themes().selection(owner).await.unwrap(), None);
}

async fn editor_response(
    app: axum::Router,
    operation: &str,
    input: serde_json::Value,
    cookie: &str,
) -> axum::response::Response {
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/themes/{operation}"))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::COOKIE, cookie)
        .body(Body::from(serde_qs::to_string(&input).unwrap()))
        .unwrap();
    app.oneshot(request).await.unwrap()
}

#[apply(backends)]
#[tokio::test]
async fn copy_builtins_creates_canonical_private_unselected_drafts(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    for scope in [OwnershipScope::Author, OwnershipScope::Site] {
        let actor =
            create_operator_and_session(env.users(), env.sessions(), env.write_scope()).await;
        for theme in [Theme::Studio, Theme::Reader, Theme::Terminal] {
            let response = server_fn_response(
                app.clone(),
                &web::themes::CopyTheme {
                    scope,
                    source: PublicThemeSelection::BuiltIn(theme),
                },
                Some(&actor.cookie()),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::CACHE_CONTROL], NO_STORE);
            let entry: web::themes::CatalogEntry =
                crate::helpers::confirmed_mutation(&body_string(response).await);
            assert!(!entry.published);
            assert_eq!(
                entry.name,
                match theme {
                    Theme::Studio => "Studio 2",
                    Theme::Reader => "Reader 2",
                    Theme::Terminal => "Terminal 2",
                }
            );
            let (status, body) = post_server_fn(
                app.clone(),
                &web::themes::GetDraft {
                    scope,
                    theme_id: entry.id,
                },
                Some(&actor.cookie()),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            let draft: Draft = serde_json::from_str(&body).unwrap();
            let canonical = host::system_theme::compile_system_artifact_inventory().unwrap();
            let package = validate_theme_package(
                canonical.theme(theme).package_bytes(),
                ThemePackageLimits::default(),
            )
            .unwrap();
            assert_eq!(draft.manifest, package.canonical_manifest());
            assert_eq!(draft.stylesheet, package.authored_css());
        }
        let (_, body) = post_server_fn(
            app.clone(),
            &web::themes::List { scope },
            Some(&actor.cookie()),
        )
        .await;
        let entries: Vec<web::themes::CatalogEntry> = serde_json::from_str(&body).unwrap();
        let source = entries
            .iter()
            .find(|entry| entry.name == "Terminal 2")
            .unwrap();
        let (status, body) = post_server_fn(
            app.clone(),
            &web::themes::CopyTheme {
                scope,
                source: PublicThemeSelection::Custom(source.id),
            },
            Some(&actor.cookie()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let copied: web::themes::CatalogEntry = crate::helpers::confirmed_mutation(&body);
        assert_eq!(copied.name, "Terminal 2 2");
        assert!(!copied.published);
        let (_, selection) = post_server_fn(
            app.clone(),
            &web::themes::GetSelection { scope },
            Some(&actor.cookie()),
        )
        .await;
        assert_eq!(selection, "null");
    }
}

#[apply(backends)]
#[tokio::test]
async fn copy_saved_custom_preserves_latest_package_complete_presentation_and_source(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let actor = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let app = make_app!(&env, &env.base);
    let response = multipart_response(
        app.clone(),
        multipart_import(
            "author",
            Some("Catalog label"),
            &archive_with_logo("body { color: red; }"),
        ),
        &actor.cookie(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let source: web::themes::CatalogEntry =
        crate::helpers::confirmed_mutation(&body_string(response).await);
    let (status, _) = post_server_fn(
        app.clone(),
        &web::themes::Publish {
            scope: OwnershipScope::Author,
            theme_id: source.id,
        },
        Some(&actor.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = post_server_fn(
        app.clone(),
        &web::themes::ReplaceCss {
            scope: OwnershipScope::Author,
            theme_id: source.id,
            stylesheet: b"body { color: blue; }".to_vec(),
        },
        Some(&actor.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let media = seed_media(
        env.media(),
        env.write_scope(),
        actor.user_id,
        "copy-header.png",
    )
    .await;
    let manager = ThemeManager::new(
        env.themes(),
        env.media(),
        env.write_scope(),
        Arc::new(env.media_content_locks()),
    );
    let owner = ThemeOwner::Author(actor.user_id);
    manager
        .replace_role(
            actor.user_id,
            owner,
            source.id,
            ThemeImageRole::Logo,
            storage::ThemeRoleInput::ExplicitAbsent,
        )
        .await
        .unwrap();
    manager
        .replace_header_pool(
            actor.user_id,
            owner,
            source.id,
            vec![
                StorageThemePoolInput::Media(media),
                StorageThemePoolInput::PackageAsset("assets/logo.png".into()),
            ],
            [7; 32],
        )
        .await
        .unwrap();
    let before = manager.copy_source(owner, source.id).await.unwrap();
    let (status, body) = post_server_fn(
        app.clone(),
        &web::themes::CopyTheme {
            scope: OwnershipScope::Author,
            source: PublicThemeSelection::Custom(source.id),
        },
        Some(&actor.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let entry: web::themes::CatalogEntry = crate::helpers::confirmed_mutation(&body);
    assert_ne!(entry.id, source.id);
    assert_eq!(entry.name, "Catalog label 2");
    assert!(!entry.published);
    let copied = manager.copy_source(owner, entry.id).await.unwrap();
    assert_eq!(copied.draft.stylesheet, b"body { color: blue; }");
    assert_eq!(copied.draft.manifest, before.draft.manifest);
    assert_eq!(copied.draft.assets, before.draft.assets);
    assert_eq!(copied.header_pool, before.header_pool);
    assert!(
        matches!(copied.bindings.as_slice(), [storage::ThemeRoleBinding::ExplicitAbsent { .. }, storage::ThemeRoleBinding::HeaderPool { shuffle_seed, .. }] if *shuffle_seed == [7; 32])
    );
    assert_eq!(manager.copy_source(owner, source.id).await.unwrap(), before);
    assert_eq!(env.themes().selection(owner).await.unwrap(), None);
    assert!(
        env.themes()
            .list_revisions(owner, entry.id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[apply(backends)]
#[tokio::test]
async fn copy_rejects_foreign_missing_and_unauthorized_sources(#[case] backend: Backend) {
    let env = backend.setup().await;
    let actor = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let other = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let app = make_app!(&env, &env.base);
    let source = create_author_theme(app.clone(), &other.cookie(), "Other", "body {}").await;
    let input = web::themes::CopyTheme {
        scope: OwnershipScope::Author,
        source: PublicThemeSelection::Custom(source.id),
    };
    let (status, body) = post_server_fn(app.clone(), &input, None).await;
    assert_ne!(status, StatusCode::OK);
    assert!(body.contains("unauthorized"));
    for id in [source.id, common::ids::ThemeId::from(99999)] {
        let (status, body) = post_server_fn(
            app.clone(),
            &web::themes::CopyTheme {
                scope: OwnershipScope::Author,
                source: PublicThemeSelection::Custom(id),
            },
            Some(&actor.cookie()),
        )
        .await;
        assert_ne!(status, StatusCode::OK);
        assert!(body.contains("not found"), "{body}");
    }
    let (status, _) = post_server_fn(
        app,
        &web::themes::CopyTheme {
            scope: OwnershipScope::Site,
            source: PublicThemeSelection::BuiltIn(Theme::Studio),
        },
        Some(&actor.cookie()),
    )
    .await;
    assert_ne!(status, StatusCode::OK);
    assert!(
        env.themes()
            .list_themes(ThemeOwner::Author(actor.user_id))
            .await
            .unwrap()
            .is_empty()
    );
}

#[apply(backends)]
#[tokio::test]
async fn copy_site_rejects_another_operators_private_media_without_partial_destination(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let actor = create_operator_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let other = create_operator_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let app = make_app!(&env, &env.base);
    let (status, body) = post_server_fn(
        app.clone(),
        &web::themes::Create {
            scope: OwnershipScope::Site,
            name: "Shared catalog".into(),
            draft: package_draft("body {}"),
        },
        Some(&other.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let source: web::themes::CatalogEntry = crate::helpers::confirmed_mutation(&body);
    let media = seed_media(env.media(), env.write_scope(), other.user_id, "private.png").await;
    let manager = ThemeManager::new(
        env.themes(),
        env.media(),
        env.write_scope(),
        Arc::new(env.media_content_locks()),
    );
    manager
        .replace_role(
            other.user_id,
            ThemeOwner::Site,
            source.id,
            ThemeImageRole::Logo,
            storage::ThemeRoleInput::Media(media),
        )
        .await
        .unwrap();
    let before = manager
        .copy_source(ThemeOwner::Site, source.id)
        .await
        .unwrap();
    let (status, _) = post_server_fn(
        app,
        &web::themes::CopyTheme {
            scope: OwnershipScope::Site,
            source: PublicThemeSelection::Custom(source.id),
        },
        Some(&actor.cookie()),
    )
    .await;
    assert_ne!(status, StatusCode::OK);
    assert_eq!(
        env.themes()
            .list_themes(ThemeOwner::Site)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        manager
            .copy_source(ThemeOwner::Site, source.id)
            .await
            .unwrap(),
        before
    );
}

#[apply(backends)]
#[tokio::test]
async fn omitted_zip_name_allocates_manifest_label_but_explicit_override_stays_strict(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let actor = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let app = make_app!(&env, &env.base);
    for (manifest, expected) in [
        ("Paper", "Paper"),
        ("paper", "paper 2"),
        ("Reader", "Reader 2"),
    ] {
        let response = multipart_response(
            app.clone(),
            multipart_import("author", None, &named_archive(manifest, "body {}")),
            &actor.cookie(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let entry: web::themes::CatalogEntry =
            crate::helpers::confirmed_mutation(&body_string(response).await);
        assert_eq!(entry.name, expected);
    }
    let response = multipart_response(
        app,
        multipart_import("author", Some("Paper"), &archive("body {}")),
        &actor.cookie(),
    )
    .await;
    assert_ne!(response.status(), StatusCode::OK);
}

#[apply(backends)]
#[tokio::test]
async fn copy_builtin_rejects_missing_or_mismatched_installed_identity(
    #[case] backend: Backend,
    #[values("missing", "source", "revision", "manifest")] fault: &str,
) {
    let env = backend.setup().await;
    let actor = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let app = make_app!(&env, &env.base);
    match fault {
        "missing" => env.base.pool().execute("DELETE FROM system_theme_current WHERE theme_token = 'studio'").await.unwrap(),
        "source" => env.base.pool().execute("UPDATE system_theme_revisions SET source_digest = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' WHERE theme_token = 'studio'").await.unwrap(),
        "revision" => {
            env.base.pool().execute("INSERT INTO system_theme_revisions (theme_token, revision_digest, source_digest, stylesheet_digest, manifest) SELECT theme_token, 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', source_digest, stylesheet_digest, manifest FROM system_theme_revisions WHERE theme_token = 'studio'").await.unwrap();
            env.base.pool().execute("UPDATE system_theme_current SET revision_digest = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' WHERE theme_token = 'studio'").await.unwrap();
        }
        "manifest" => storage::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query("UPDATE system_theme_revisions SET manifest = $1 WHERE theme_token = 'studio'").bind(b"invalid".to_vec()).execute(pool).await.unwrap();
        }),
        _ => unreachable!("closed test cases"),
    }
    let response = server_fn_response(
        app,
        &web::themes::CopyTheme {
            scope: OwnershipScope::Author,
            source: PublicThemeSelection::BuiltIn(Theme::Studio),
        },
        Some(&actor.cookie()),
    )
    .await;
    assert_ne!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], NO_STORE);
    assert!(
        env.themes()
            .list_themes(ThemeOwner::Author(actor.user_id))
            .await
            .unwrap()
            .is_empty()
    );
}

#[apply(backends)]
#[tokio::test]
async fn copy_binding_failure_rolls_back_new_catalog_and_quota(#[case] backend: Backend) {
    let env = backend.setup().await;
    let actor = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let app = make_app!(&env, &env.base);
    let response = multipart_response(
        app.clone(),
        multipart_import("author", None, &archive_with_logo("body {}")),
        &actor.cookie(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let source: web::themes::CatalogEntry =
        crate::helpers::confirmed_mutation(&body_string(response).await);
    let owner = ThemeOwner::Author(actor.user_id);
    let manager = ThemeManager::new(
        env.themes(),
        env.media(),
        env.write_scope(),
        Arc::new(env.media_content_locks()),
    );
    manager
        .replace_role(
            actor.user_id,
            owner,
            source.id,
            ThemeImageRole::Logo,
            storage::ThemeRoleInput::PackageAsset("assets/logo.png".into()),
        )
        .await
        .unwrap();
    let (status, _) = post_server_fn(
        app.clone(),
        &web::themes::ImportPackage {
            scope: OwnershipScope::Author,
            theme_id: source.id,
            draft: package_draft("body {}"),
        },
        Some(&actor.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let quota = env.themes().owner_quota(owner).await.unwrap();
    let (status, _) = post_server_fn(
        app,
        &web::themes::CopyTheme {
            scope: OwnershipScope::Author,
            source: PublicThemeSelection::Custom(source.id),
        },
        Some(&actor.cookie()),
    )
    .await;
    assert_ne!(status, StatusCode::OK);
    assert_eq!(env.themes().list_themes(owner).await.unwrap().len(), 1);
    assert_eq!(env.themes().owner_quota(owner).await.unwrap(), quota);
}

#[apply(backends)]
#[tokio::test]
async fn copy_revalidates_package_before_creating_any_destination(#[case] backend: Backend) {
    let env = backend.setup().await;
    let actor = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let app = make_app!(&env, &env.base);
    let source = create_author_theme(
        app.clone(),
        &actor.cookie(),
        "Invalid saved source",
        "body {}",
    )
    .await;
    let owner = ThemeOwner::Author(actor.user_id);
    let mut draft = env
        .themes()
        .get_draft(owner, source.id)
        .await
        .unwrap()
        .unwrap();
    draft.stylesheet = b"body { background: url(https://external.invalid/image.png); }".to_vec();
    let invalid = draft.clone();
    let themes = env.themes();
    // A deliberately corrupted private saved draft must not bypass Copy's package admission.
    env.write_scope()
        .run(move |transaction| {
            Box::pin(async move {
                themes
                    .replace_draft(
                        transaction,
                        owner,
                        &invalid,
                        storage::ThemeQuotaLimits::production(),
                    )
                    .await
            })
        })
        .await
        .unwrap();
    let (status, _) = post_server_fn(
        app,
        &web::themes::CopyTheme {
            scope: OwnershipScope::Author,
            source: PublicThemeSelection::Custom(source.id),
        },
        Some(&actor.cookie()),
    )
    .await;
    assert_ne!(status, StatusCode::OK);
    assert_eq!(env.themes().list_themes(owner).await.unwrap().len(), 1);
    assert_eq!(
        env.themes()
            .get_draft(owner, source.id)
            .await
            .unwrap()
            .unwrap(),
        draft
    );
}

#[apply(backends)]
#[tokio::test]
async fn copy_quota_rejection_preserves_source_and_does_not_allocate_a_destination(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let actor = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let app = make_app!(&env, &env.base);
    let source = create_author_theme(app.clone(), &actor.cookie(), "Full library", "body {}").await;
    let owner = ThemeOwner::Author(actor.user_id);
    let draft = env
        .themes()
        .get_draft(owner, source.id)
        .await
        .unwrap()
        .unwrap();
    for index in 1..64 {
        let themes = env.themes();
        let draft = draft.clone();
        env.write_scope()
            .run(move |transaction| {
                Box::pin(async move {
                    themes
                        .create_theme(
                            transaction,
                            owner,
                            &format!("Entry {index}"),
                            &draft,
                            storage::ThemeQuotaLimits::production(),
                        )
                        .await
                })
            })
            .await
            .unwrap();
    }
    let (status, _) = post_server_fn(
        app,
        &web::themes::CopyTheme {
            scope: OwnershipScope::Author,
            source: PublicThemeSelection::Custom(source.id),
        },
        Some(&actor.cookie()),
    )
    .await;
    assert_ne!(status, StatusCode::OK);
    assert_eq!(env.themes().list_themes(owner).await.unwrap().len(), 64);
    assert_eq!(
        env.themes()
            .get_draft(owner, source.id)
            .await
            .unwrap()
            .unwrap(),
        draft
    );
}

#[apply(backends)]
#[tokio::test]
async fn copy_commit_acknowledgement_loss_reconciles_one_destination_without_replay(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let actor = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let source = create_author_theme(
        make_app!(&env, &env.base),
        &actor.cookie(),
        "Saved source",
        "body {}",
    )
    .await;
    let lost =
        storage::test_support::write_scope_with_commit_acknowledgement_loss(&env.write_scope());
    let app = make_app!(&env, &env.base; override_write_scope = lost);
    let (status, body) = post_server_fn(
        app,
        &web::themes::CopyTheme {
            scope: OwnershipScope::Author,
            source: PublicThemeSelection::Custom(source.id),
        },
        Some(&actor.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let outcome: MutationOutcome<web::themes::CatalogEntry> = serde_json::from_str(&body).unwrap();
    let MutationOutcome::CommitIndeterminate(entry) = outcome else {
        panic!("lost acknowledgement must remain uncertain");
    };
    let (status, body) = post_server_fn(
        make_app!(&env, &env.base),
        &web::themes::List {
            scope: OwnershipScope::Author,
        },
        Some(&actor.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let entries: Vec<web::themes::CatalogEntry> = serde_json::from_str(&body).unwrap();
    assert_eq!(entries.len(), 2);
    assert!(entries.contains(&entry));
    assert_ne!(entry.id, source.id);
    assert!(!entry.published);
    assert_eq!(
        env.themes()
            .selection(ThemeOwner::Author(actor.user_id))
            .await
            .unwrap(),
        None
    );
}

#[apply(backends)]
#[tokio::test]
async fn concurrent_upload_copy_strict_create_and_rename_share_one_namespace(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let owner = create_operator_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let copier = create_operator_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let uploader =
        create_operator_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let renamer = create_operator_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let app = make_app!(&env, &env.base);
    let (status, body) = post_server_fn(
        app.clone(),
        &web::themes::Create {
            scope: OwnershipScope::Site,
            name: "Paper".into(),
            draft: package_draft("body {}"),
        },
        Some(&owner.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let source: web::themes::CatalogEntry = crate::helpers::confirmed_mutation(&body);
    let (_, body) = post_server_fn(
        app.clone(),
        &web::themes::Create {
            scope: OwnershipScope::Site,
            name: "Other".into(),
            draft: package_draft("body {}"),
        },
        Some(&owner.cookie()),
    )
    .await;
    let other: web::themes::CatalogEntry = crate::helpers::confirmed_mutation(&body);
    let copy_input = web::themes::CopyTheme {
        scope: OwnershipScope::Site,
        source: PublicThemeSelection::Custom(source.id),
    };
    let strict_input = web::themes::Create {
        scope: OwnershipScope::Site,
        name: "Paper 2".into(),
        draft: package_draft("body {}"),
    };
    let rename_input = web::themes::Rename {
        scope: OwnershipScope::Site,
        theme_id: other.id,
        name: "Paper 3".into(),
    };
    let owner_cookie = owner.cookie();
    let copy_cookie = copier.cookie();
    let upload_cookie = uploader.cookie();
    let rename_cookie = renamer.cookie();
    // Distinct principals permit real concurrent Site mutations; the database,
    // not the per-principal operation coordinator, owns name serialization.
    let (copied, uploaded, strict, renamed) = tokio::join!(
        post_server_fn(app.clone(), &copy_input, Some(&copy_cookie)),
        multipart_response(
            app.clone(),
            multipart_import("site", None, &archive("body {}")),
            &upload_cookie
        ),
        post_server_fn(app.clone(), &strict_input, Some(&owner_cookie)),
        post_server_fn(app.clone(), &rename_input, Some(&rename_cookie))
    );
    assert_eq!(copied.0, StatusCode::OK, "{}", copied.1);
    assert_eq!(uploaded.status(), StatusCode::OK);
    let copied: web::themes::CatalogEntry = crate::helpers::confirmed_mutation(&copied.1);
    let uploaded: web::themes::CatalogEntry =
        crate::helpers::confirmed_mutation(&body_string(uploaded).await);
    assert_ne!(copied.id, uploaded.id);
    assert_ne!(copied.name.to_lowercase(), uploaded.name.to_lowercase());
    for (status, body) in [strict, renamed] {
        if status != StatusCode::OK {
            assert!(body.contains("already exists"), "{body}");
        }
    }
    let entries = env.themes().list_themes(ThemeOwner::Site).await.unwrap();
    let keys = entries
        .iter()
        .map(|entry| entry.name.to_lowercase())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(keys.len(), entries.len());
    assert!(keys.contains("paper 2"));
    assert_eq!(
        env.themes().selection(ThemeOwner::Site).await.unwrap(),
        None
    );
}

#[apply(backends)]
#[tokio::test]
async fn copy_builtin_without_canonical_inventory_returns_error_not_fallback(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let actor = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let coordinator = Arc::new(host::theme_operations::ThemeOperationCoordinator::new());
    // This deliberately incomplete native root supplies real authentication and
    // admission, but no canonical source provider. Missing DI must be an error.
    let app = crate::helpers::prepare_app(&env.base, |_| {
        axum::Router::new()
            .route(
                "/api/{*fn_name}",
                axum::routing::post(move |request: axum::extract::Request| {
                    let coordinator = coordinator.clone();
                    leptos_axum::handle_server_fns_with_context(
                        move || leptos::prelude::provide_context(coordinator.clone()),
                        request,
                    )
                }),
            )
            .layer(axum::Extension(env.sessions()))
            .layer(axum::Extension(env.write_scope()))
    });
    let response = server_fn_response(
        app,
        &web::themes::CopyTheme {
            scope: OwnershipScope::Author,
            source: PublicThemeSelection::BuiltIn(Theme::Studio),
        },
        Some(&actor.cookie()),
    )
    .await;
    assert_ne!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], NO_STORE);
    assert!(
        env.themes()
            .list_themes(ThemeOwner::Author(actor.user_id))
            .await
            .unwrap()
            .is_empty()
    );
}

fn package_draft(css: &str) -> Draft {
    Draft {
        manifest: br#"{"schema":1,"name":"Paper","style_contract":1,"assets":{},"defaults":{}}"#
            .to_vec(),
        stylesheet: css.as_bytes().to_vec(),
        assets: Vec::new(),
    }
}

fn archive(css: &str) -> Vec<u8> {
    named_archive("Paper", css)
}

fn named_archive(name: &str, css: &str) -> Vec<u8> {
    let manifest = serde_json::json!({
        "schema": 1, "name": name, "style_contract": 1, "assets": {}, "defaults": {}
    });
    export_theme_package(
        &serde_json::to_vec(&manifest).unwrap(),
        css.as_bytes(),
        &BTreeMap::new(),
    )
    .expect("no-asset theme package exports")
}

fn valid_png() -> Vec<u8> {
    b"\x89\x50\x4e\x47\x0d\x0a\x1a\x0a\x00\x00\x00\x0d\x49\x48\x44\x52\x00\x00\x00\x01\x00\x00\x00\x01\x08\x04\x00\x00\x00\xb5\x1c\x0c\x02\x00\x00\x00\x0b\x49\x44\x41\x54\x78\xda\x63\x64\xf8\x0f\x00\x01\x05\x01\x01\x27\x18\xe3\x66\x00\x00\x00\x00\x49\x45\x4e\x44\xae\x42\x60\x82".to_vec()
}

fn archive_with_logo(css: &str) -> Vec<u8> {
    export_theme_package(
        br#"{"schema":1,"name":"Bindings","style_contract":1,"assets":{"assets/logo.png":"image/png"},"defaults":{}}"#,
        css.as_bytes(),
        &BTreeMap::from([("assets/logo.png".into(), valid_png())]),
    )
    .expect("valid package asset exports")
}

fn archive_with_package_defaults(css: &str) -> Vec<u8> {
    export_theme_package(
        br#"{"schema":1,"name":"Package defaults","style_contract":1,"assets":{"assets/logo.png":"image/png","assets/header-a.png":"image/png","assets/header-b.png":"image/png"},"defaults":{"logo":"assets/logo.png","header":["assets/header-a.png","assets/header-b.png"]}}"#,
        css.as_bytes(),
        &BTreeMap::from([
            ("assets/logo.png".into(), valid_png()),
            ("assets/header-a.png".into(), valid_png()),
            ("assets/header-b.png".into(), valid_png()),
        ]),
    )
    .expect("valid package-default asset exports")
}

async fn create_author_theme(
    app: axum::Router,
    cookie: &str,
    name: &str,
    css: &str,
) -> web::themes::CatalogEntry {
    let (status, body) = post_server_fn(
        app,
        &web::themes::Create {
            scope: OwnershipScope::Author,
            name: name.to_owned(),
            draft: package_draft(css),
        },
        Some(cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    confirmed_for(
        serde_json::from_str::<MutationOutcome<_>>(&body).expect("create outcome JSON"),
        "theme creation",
    )
}

fn server_fn_request<F>(input: &F, cookie: Option<&str>) -> Request<Body>
where
    F: serde::Serialize + ServerFn,
{
    let mut request = Request::builder()
        .method("POST")
        .uri(F::PATH)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    request
        .body(Body::from(
            serde_qs::to_string(input).expect("server function encodes"),
        ))
        .expect("server function request builds")
}

async fn server_fn_response<F>(
    app: axum::Router,
    input: &F,
    cookie: Option<&str>,
) -> axum::response::Response
where
    F: serde::Serialize + ServerFn,
{
    app.oneshot(server_fn_request(input, cookie))
        .await
        .expect("router accepts request")
}

fn multipart_request(body: Vec<u8>, cookie: &str) -> Request<Body> {
    let boundary = "----jaunder-theme-test-boundary";
    Request::builder()
        .method("POST")
        .uri(<web::themes::ImportZip as ServerFn>::PATH)
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .header(header::COOKIE, cookie)
        .body(Body::from(body))
        .expect("multipart request builds")
}

async fn multipart_response(
    app: axum::Router,
    body: Vec<u8>,
    cookie: &str,
) -> axum::response::Response {
    app.oneshot(multipart_request(body, cookie))
        .await
        .expect("router accepts multipart request")
}

fn multipart_body(scope: &str, name: &str, archive: &[u8]) -> Vec<u8> {
    multipart_import(scope, Some(name), archive)
}

fn multipart_import(scope: &str, name: Option<&str>, archive: &[u8]) -> Vec<u8> {
    let boundary = "----jaunder-theme-test-boundary";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"scope\"\r\n\r\n{scope}\r\n"
    )
    .into_bytes();
    if let Some(name) = name {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"name\"\r\n\r\n{name}\r\n"
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"archive\"; filename=\"theme.zip\"\r\nContent-Type: application/zip\r\n\r\n").as_bytes());
    body.extend_from_slice(archive);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

fn multipart_fields(fields: &[(&str, &[u8])]) -> Vec<u8> {
    let boundary = "----jaunder-theme-test-boundary";
    let mut body = Vec::new();
    for (name, value) in fields {
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n")
                .as_bytes(),
        );
        body.extend_from_slice(value);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

#[apply(backends)]
#[tokio::test]
async fn installed_preview_reads_studio_without_selecting_it(#[case] backend: Backend) {
    let env = backend.setup().await;
    let member = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let app = make_app!(&env, &env.base);
    let response = server_fn_response(
        app.clone(),
        &web::themes::PreviewInstalled {
            scope: OwnershipScope::Author,
            theme: PublicThemeSelection::BuiltIn(Theme::Studio),
        },
        Some(&member.cookie()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], NO_STORE);
    let body = body_string(response).await;
    let preview: web::themes::ThemePreview = serde_json::from_str(&body).unwrap();
    assert!(preview.html.contains("data-jaunder-theme-surface"));
    assert!(!preview.css.is_empty());
    let (status, selection) = post_server_fn(
        app,
        &web::themes::GetSelection {
            scope: OwnershipScope::Author,
        },
        Some(&member.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(selection, "null");
}

#[apply(backends)]
#[tokio::test]
async fn installed_preview_all_builtins_respect_each_owner_scope(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    for scope in [OwnershipScope::Author, OwnershipScope::Site] {
        let actor =
            create_operator_and_session(env.users(), env.sessions(), env.write_scope()).await;
        let before = post_server_fn(
            app.clone(),
            &web::themes::GetSelection { scope },
            Some(&actor.cookie()),
        )
        .await;
        for theme in [Theme::Studio, Theme::Terminal, Theme::Reader] {
            let response = server_fn_response(
                app.clone(),
                &web::themes::PreviewInstalled {
                    scope,
                    theme: PublicThemeSelection::BuiltIn(theme),
                },
                Some(&actor.cookie()),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::CACHE_CONTROL], NO_STORE);
            let preview: web::themes::ThemePreview =
                serde_json::from_str(&body_string(response).await).unwrap();
            assert!(preview.html.contains(&format!("data-theme=\"{theme}\"")));
            assert!(!preview.css.is_empty());
        }
        assert_eq!(
            post_server_fn(
                app.clone(),
                &web::themes::GetSelection { scope },
                Some(&actor.cookie())
            )
            .await,
            before
        );
        let (_, catalog) = post_server_fn(
            app.clone(),
            &web::themes::List { scope },
            Some(&actor.cookie()),
        )
        .await;
        assert_eq!(catalog, "[]");
    }
}

#[apply(backends)]
#[tokio::test]
async fn installed_preview_requires_auth_and_site_operator_even_for_builtins(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let member = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let app = make_app!(&env, &env.base);
    for scope in [OwnershipScope::Author, OwnershipScope::Site] {
        let (status, body) = post_server_fn(
            app.clone(),
            &web::themes::PreviewInstalled {
                scope,
                theme: PublicThemeSelection::BuiltIn(Theme::Studio),
            },
            None,
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(body.contains("unauthorized"));
    }
    for theme in [
        PublicThemeSelection::BuiltIn(Theme::Studio),
        PublicThemeSelection::Custom(common::ids::ThemeId::from(42)),
    ] {
        let (status, body) = post_server_fn(
            app.clone(),
            &web::themes::PreviewInstalled {
                scope: OwnershipScope::Site,
                theme,
            },
            Some(&member.cookie()),
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(body.contains("unauthorized"));
    }
}

#[apply(backends)]
#[tokio::test]
async fn installed_preview_preserves_unpublished_custom_and_legacy_behavior(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    for scope in [OwnershipScope::Author, OwnershipScope::Site] {
        let actor =
            create_operator_and_session(env.users(), env.sessions(), env.write_scope()).await;
        let (status, created) = post_server_fn(
            app.clone(),
            &web::themes::Create {
                scope,
                name: "Saved preview".into(),
                draft: package_draft("[data-jaunder-part=main] { color: purple; }"),
            },
            Some(&actor.cookie()),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let entry: web::themes::CatalogEntry = confirmed_for(
            serde_json::from_str::<MutationOutcome<_>>(&created).unwrap(),
            "preview setup",
        );
        let before = post_server_fn(
            app.clone(),
            &web::themes::GetDraft {
                scope,
                theme_id: entry.id,
            },
            Some(&actor.cookie()),
        )
        .await;
        let (status, generic) = post_server_fn(
            app.clone(),
            &web::themes::PreviewInstalled {
                scope,
                theme: PublicThemeSelection::Custom(entry.id),
            },
            Some(&actor.cookie()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{generic}");
        let value: web::themes::ThemePreview = serde_json::from_str(&generic).unwrap();
        assert!(value.css.contains("purple"));
        let (status, legacy) = post_server_fn(
            app.clone(),
            &web::themes::Preview {
                scope,
                theme_id: entry.id,
            },
            Some(&actor.cookie()),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            serde_json::from_str::<web::themes::ThemePreview>(&legacy).unwrap(),
            value
        );
        assert_eq!(
            post_server_fn(
                app.clone(),
                &web::themes::GetDraft {
                    scope,
                    theme_id: entry.id
                },
                Some(&actor.cookie())
            )
            .await,
            before
        );
        let (_, selection) = post_server_fn(
            app.clone(),
            &web::themes::GetSelection { scope },
            Some(&actor.cookie()),
        )
        .await;
        assert_eq!(selection, "null");
        let (_, catalog) = post_server_fn(
            app.clone(),
            &web::themes::List { scope },
            Some(&actor.cookie()),
        )
        .await;
        let entries: Vec<web::themes::CatalogEntry> = serde_json::from_str(&catalog).unwrap();
        assert!(entries.iter().all(|entry| !entry.published));
    }
}

#[apply(backends)]
#[tokio::test]
async fn installed_preview_masks_foreign_and_missing_custom_ids(#[case] backend: Backend) {
    let env = backend.setup().await;
    let first = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let second = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let app = make_app!(&env, &env.base);
    let theme = create_author_theme(
        app.clone(),
        &first.cookie(),
        "Private",
        "[data-jaunder-part=main] { color: purple; }",
    )
    .await;
    for id in [theme.id, common::ids::ThemeId::from(999_999)] {
        let response = server_fn_response(
            app.clone(),
            &web::themes::PreviewInstalled {
                scope: OwnershipScope::Author,
                theme: PublicThemeSelection::Custom(id),
            },
            Some(&second.cookie()),
        )
        .await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(response.headers()[header::CACHE_CONTROL], NO_STORE);
        let body = body_string(response).await;
        assert!(body.contains("not found"));
        assert!(!body.contains("purple"));
        assert!(!body.contains("Private"));
    }
}

#[apply(backends)]
#[tokio::test]
async fn installed_preview_fails_closed_on_missing_or_corrupt_system_content(
    #[case] backend: Backend,
    #[values("missing", "ineligible", "file", "integrity", "metadata", "utf8")] fault: &str,
) {
    use sha2::{Digest, Sha256};
    let env = backend.setup().await;
    let member = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let app = make_app!(&env, &env.base);
    let revision = env
        .themes()
        .system_theme_revision(Theme::Studio)
        .await
        .unwrap()
        .unwrap();
    let digest = revision.stylesheet_digest.as_ref();
    let path = env
        .base
        .path()
        .join("themes")
        .join(&digest[..2])
        .join(&digest[2..4])
        .join(digest);
    // Fault injection deliberately bypasses normal compiler-minted admission in
    // this private per-test database; assertions remain at the HTTP read boundary.
    match fault {
        "missing" => env
            .base
            .pool()
            .execute("DELETE FROM system_theme_current WHERE theme_token = 'studio'")
            .await
            .unwrap(),
        "ineligible" => {
            env.base.pool().execute("DELETE FROM system_theme_content_references WHERE digest IN (SELECT stylesheet_digest FROM system_theme_revisions WHERE theme_token = 'studio')").await.unwrap();
            env.base.pool().execute("DELETE FROM theme_content_eligibility WHERE digest IN (SELECT stylesheet_digest FROM system_theme_revisions WHERE theme_token = 'studio')").await.unwrap();
        }
        "file" => fs::remove_file(path).unwrap(),
        "integrity" => fs::write(path, b"corrupt stylesheet").unwrap(),
        "metadata" => storage::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query(
                "UPDATE system_theme_revisions SET manifest = $1 WHERE theme_token = 'studio'",
            )
            .bind(b"not json".to_vec())
            .execute(pool)
            .await
            .unwrap();
        }),
        "utf8" => {
            let bytes = vec![255_u8];
            let bad_digest =
                common::theme::ThemeStylesheetDigest::from_digest(Sha256::digest(&bytes).into());
            let token = bad_digest.as_ref();
            let path = env
                .base
                .path()
                .join("themes")
                .join(&token[..2])
                .join(&token[2..4])
                .join(token);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
            storage::with_closeable_pool!(env.base.pool(), pool, {
                sqlx::query("INSERT INTO theme_content_eligibility (digest, mime, retained_until_unix_seconds, live_references) VALUES ($1, 'text/css; charset=utf-8', 0, 1)").bind(token).execute(pool).await.unwrap();
                sqlx::query("UPDATE system_theme_revisions SET stylesheet_digest = $1 WHERE theme_token = 'studio'").bind(token).execute(pool).await.unwrap();
            });
        }
        _ => unreachable!("closed test fault cases"),
    }
    let response = server_fn_response(
        app,
        &web::themes::PreviewInstalled {
            scope: OwnershipScope::Author,
            theme: PublicThemeSelection::BuiltIn(Theme::Studio),
        },
        Some(&member.cookie()),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "{fault}"
    );
    assert_eq!(response.headers()[header::CACHE_CONTROL], NO_STORE);
    let body = body_string(response).await;
    assert!(
        !body.contains("data-jaunder-theme-surface"),
        "{fault}: {body}"
    );
    assert!(!body.contains("corrupt stylesheet"));
    if fault == "missing" {
        assert!(body.contains("not found"));
    }
}

#[apply(backends)]
#[tokio::test]
async fn theme_catalog_enforces_authentication_and_scope(#[case] backend: Backend) {
    let env = backend.setup().await;
    let member = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let operator = create_operator_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;

    let (anonymous_status, anonymous_body) = post_server_fn(
        make_app!(&env, &env.base),
        &web::themes::List {
            scope: OwnershipScope::Author,
        },
        None,
    )
    .await;
    assert_eq!(
        anonymous_status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "body: {anonymous_body}"
    );
    assert!(
        anonymous_body.contains("unauthorized"),
        "body: {anonymous_body}"
    );

    let (author_status, author_body) = post_server_fn(
        make_app!(&env, &env.base),
        &web::themes::List {
            scope: OwnershipScope::Author,
        },
        Some(&member.cookie()),
    )
    .await;
    assert_eq!(author_status, StatusCode::OK, "body: {author_body}");
    assert_eq!(
        serde_json::from_str::<Vec<web::themes::CatalogEntry>>(&author_body).expect("catalog JSON"),
        Vec::new()
    );

    let (site_denied_status, site_denied_body) = post_server_fn(
        make_app!(&env, &env.base),
        &web::themes::List {
            scope: OwnershipScope::Site,
        },
        Some(&member.cookie()),
    )
    .await;
    assert_eq!(
        site_denied_status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "body: {site_denied_body}"
    );
    assert!(
        site_denied_body.contains("unauthorized"),
        "body: {site_denied_body}"
    );

    let (site_status, site_body) = post_server_fn(
        make_app!(&env, &env.base),
        &web::themes::List {
            scope: OwnershipScope::Site,
        },
        Some(&operator.cookie()),
    )
    .await;
    assert_eq!(site_status, StatusCode::OK, "body: {site_body}");
}

// A catalog name collision reaches the real database constraint before the
// server-function boundary projects the driver error.
#[apply(backends)]
#[tokio::test]
async fn duplicate_theme_names_are_user_facing_for_create_and_rename(#[case] backend: Backend) {
    let env = backend.setup().await;
    let owner = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let storage = TempDir::new().expect("temporary storage");
    let first = create_author_theme(
        make_app!(&env, &storage),
        &owner.cookie(),
        "Paper",
        "body { color: navy; }",
    )
    .await;

    let (status, body) = post_server_fn(
        make_app!(&env, &storage),
        &web::themes::Create {
            scope: OwnershipScope::Author,
            name: "Paper".into(),
            draft: package_draft("body { color: teal; }"),
        },
        Some(&owner.cookie()),
    )
    .await;
    assert_ne!(status, StatusCode::OK, "duplicate create must be rejected");
    assert!(body.contains("already exists"), "body: {body}");

    let second = create_author_theme(
        make_app!(&env, &storage),
        &owner.cookie(),
        "Ink",
        "body { color: black; }",
    )
    .await;
    let (status, body) = post_server_fn(
        make_app!(&env, &storage),
        &web::themes::Rename {
            scope: OwnershipScope::Author,
            theme_id: second.id,
            name: first.name,
        },
        Some(&owner.cookie()),
    )
    .await;
    assert_ne!(status, StatusCode::OK, "duplicate rename must be rejected");
    assert!(body.contains("already exists"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn theme_drafts_mask_foreign_ids_and_never_cache_private_responses(#[case] backend: Backend) {
    let env = backend.setup().await;
    let owner = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let stranger = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let theme = create_author_theme(
        make_app!(&env, &env.base),
        &owner.cookie(),
        "Draft",
        "body { color: navy; }",
    )
    .await;
    let storage = TempDir::new().expect("temporary storage");

    let draft = server_fn_response(
        make_app!(&env, &storage),
        &web::themes::GetDraft {
            scope: OwnershipScope::Author,
            theme_id: theme.id,
        },
        Some(&owner.cookie()),
    )
    .await;
    assert_eq!(draft.status(), StatusCode::OK);
    assert_eq!(draft.headers()[header::CACHE_CONTROL], NO_STORE);
    let draft: Draft = serde_json::from_str(&body_string(draft).await).expect("draft JSON");
    assert_eq!(draft.stylesheet, b"body { color: navy; }");
    assert!(draft.assets.is_empty());
    assert_eq!(
        draft.manifest,
        br#"{"assets":{},"defaults":{},"name":"Paper","schema":1,"style_contract":1}"#
    );

    let foreign = server_fn_response(
        make_app!(&env, &storage),
        &web::themes::GetDraft {
            scope: OwnershipScope::Author,
            theme_id: theme.id,
        },
        Some(&stranger.cookie()),
    )
    .await;
    assert_eq!(foreign.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(foreign.headers()[header::CACHE_CONTROL], NO_STORE);
    assert!(body_string(foreign).await.contains("not found"));
}

#[apply(backends)]
#[tokio::test]
async fn theme_export_is_private_safe_and_contains_only_package_state(#[case] backend: Backend) {
    let env = backend.setup().await;
    let owner = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let theme = create_author_theme(
        make_app!(&env, &env.base),
        &owner.cookie(),
        "dangerous; filename=owned\".zip",
        "body { color: teal; }",
    )
    .await;
    let storage = TempDir::new().expect("temporary storage");
    let response = server_fn_response(
        make_app!(&env, &storage),
        &web::themes::Export {
            scope: OwnershipScope::Author,
            theme_id: theme.id,
        },
        Some(&owner.cookie()),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], NO_STORE);
    assert_eq!(
        response.headers()[header::CONTENT_DISPOSITION],
        "attachment; filename=\"dangerousfilenameownedzip.zip\""
    );
    let export: web::themes::ExportedPackage =
        serde_json::from_str(&body_string(response).await).expect("export JSON");
    assert_eq!(export.filename, "dangerousfilenameownedzip.zip");
    let package = validate_theme_package(&export.bytes, ThemePackageLimits::default())
        .expect("export is a valid package");
    assert_eq!(package.authored_css(), b"body { color: teal; }");
    assert_eq!(
        package.asset_paths().collect::<Vec<_>>(),
        Vec::<&str>::new()
    );
    let stranger = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let foreign = server_fn_response(
        make_app!(&env, &storage),
        &web::themes::Export {
            scope: OwnershipScope::Author,
            theme_id: theme.id,
        },
        Some(&stranger.cookie()),
    )
    .await;
    assert_eq!(foreign.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(foreign.headers()[header::CACHE_CONTROL], NO_STORE);
}

#[apply(backends)]
#[tokio::test]
async fn theme_import_zip_defaults_to_manifest_name_without_publishing_or_selecting(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let owner = create_user_and_session(
        Arc::clone(&env.users()),
        Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let app = make_app!(&env, &env.base);
    let (status, body) = post_server_fn(
        app.clone(),
        &web::themes::Select {
            scope: OwnershipScope::Author,
            selection: Some(PublicThemeSelection::BuiltIn(Theme::Reader)),
        },
        Some(&owner.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let response = multipart_response(
        app.clone(),
        multipart_import("author", None, &archive("body { color: teal; }")),
        &owner.cookie(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], NO_STORE);
    let imported: web::themes::CatalogEntry = confirmed_for(
        serde_json::from_str::<MutationOutcome<_>>(&body_string(response).await).unwrap(),
        "manifest-named ZIP import",
    );
    assert_eq!(imported.name, "Paper");
    assert!(!imported.published);
    let (status, body) = post_server_fn(
        app,
        &web::themes::GetSelection {
            scope: OwnershipScope::Author,
        },
        Some(&owner.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        serde_json::from_str::<Option<PublicThemeSelection>>(&body).unwrap(),
        Some(PublicThemeSelection::BuiltIn(Theme::Reader))
    );
}

#[apply(backends)]
#[tokio::test]
async fn theme_import_zip_rejects_invalid_catalog_names_without_fallback_or_side_effects(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let owner = create_user_and_session(
        Arc::clone(&env.users()),
        Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let too_long = "界".repeat(101);
    for (manifest, override_name, expected) in [
        ("Paper", Some(""), "must not be empty"),
        ("Paper", Some(" \t "), "must not be empty"),
        ("Paper", Some("sTuDiO"), "conflicts with a built-in"),
        ("Paper", Some("Terminal"), "conflicts with a built-in"),
        ("Paper", Some("READER"), "conflicts with a built-in"),
        ("Paper", Some(too_long.as_str()), "must not exceed 100"),
        (too_long.as_str(), None, "must not exceed 100"),
    ] {
        let response = multipart_response(
            make_app!(&env, &env.base),
            multipart_import("author", override_name, &named_archive(manifest, "body {}")),
            &owner.cookie(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = body_string(response).await;
        assert!(body.contains(expected), "{body}");
        if override_name.is_some() {
            assert!(
                body.contains("override"),
                "explicit name errors explain recovery: {body}"
            );
        }
    }
    let (status, body) = post_server_fn(
        make_app!(&env, &env.base),
        &web::themes::List {
            scope: OwnershipScope::Author,
        },
        Some(&owner.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        serde_json::from_str::<Vec<web::themes::CatalogEntry>>(&body)
            .unwrap()
            .is_empty()
    );
}

#[apply(backends)]
#[tokio::test]
async fn theme_import_zip_collisions_offer_an_override_without_partial_import(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let owner = create_user_and_session(
        Arc::clone(&env.users()),
        Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let original = create_author_theme(
        make_app!(&env, &env.base),
        &owner.cookie(),
        "Paper",
        "body { color: navy; }",
    )
    .await;
    for override_name in [Some(" Paper "), Some("paper")] {
        let response = multipart_response(
            make_app!(&env, &env.base),
            multipart_import("author", override_name, &archive("body { color: red; }")),
            &owner.cookie(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = body_string(response).await;
        assert!(body.contains("theme name already exists"), "{body}");
        assert!(body.contains("override"), "{body}");
    }
    let (status, body) = post_server_fn(
        make_app!(&env, &env.base),
        &web::themes::List {
            scope: OwnershipScope::Author,
        },
        Some(&owner.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        serde_json::from_str::<Vec<web::themes::CatalogEntry>>(&body).unwrap(),
        vec![original]
    );
}

#[apply(backends)]
#[tokio::test]
async fn theme_import_zip_trims_effective_names_and_counts_unicode_scalars(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let owner = create_user_and_session(
        Arc::clone(&env.users()),
        Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let unicode_name = "界".repeat(100);
    for (manifest, override_name, expected) in [
        (" \t Trimmed manifest \n ", None, "Trimmed manifest"),
        ("Paper", Some("  Explicit override  "), "Explicit override"),
        (
            "sTuDiO",
            Some("Reserved package override"),
            "Reserved package override",
        ),
        (unicode_name.as_str(), None, unicode_name.as_str()),
    ] {
        let response = multipart_response(
            make_app!(&env, &env.base),
            multipart_import("author", override_name, &named_archive(manifest, "body {}")),
            &owner.cookie(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let imported: web::themes::CatalogEntry = confirmed_for(
            serde_json::from_str::<MutationOutcome<_>>(&body_string(response).await).unwrap(),
            "effective catalog name",
        );
        assert_eq!(imported.name, expected);
        assert!(!imported.published);
    }
}

#[apply(backends)]
#[tokio::test]
async fn theme_import_zip_validates_packages_before_default_names_and_authorizes_before_archives(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let owner = create_user_and_session(
        Arc::clone(&env.users()),
        Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    for (bytes, override_name) in [
        (b"not a zip".to_vec(), None),
        (named_archive(" \t ", "body {}"), None),
        (named_archive(" \t ", "body {}"), Some("Valid override")),
        (
            named_archive("Studio", "@import url('https://example.com/style.css');"),
            None,
        ),
    ] {
        let response = multipart_response(
            make_app!(&env, &env.base),
            multipart_import("author", override_name, &bytes),
            &owner.cookie(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = body_string(response).await;
        assert!(body.contains("invalid theme package"), "{body}");
        assert!(
            !body.contains("theme name conflicts"),
            "package validation precedes default naming: {body}"
        );
        assert!(
            !body.contains("override"),
            "invalid packages need package repair: {body}"
        );
    }
    for body in [
        multipart_fields(&[("scope", b"author")]),
        multipart_fields(&[("scope", b"author"), ("name", b"Valid")]),
    ] {
        let response = multipart_response(make_app!(&env, &env.base), body, &owner.cookie()).await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            body_string(response)
                .await
                .contains("missing theme archive")
        );
    }
    let oversized = vec![0; ThemePackageLimits::default().max_archive_bytes + 1];
    let response = multipart_response(
        make_app!(&env, &env.base),
        multipart_import("author", None, &oversized),
        &owner.cookie(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(body_string(response).await.contains("too large"));
    let response = multipart_response(
        make_app!(&env, &env.base),
        multipart_import("site", None, b"not a zip"),
        &owner.cookie(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(body_string(response).await.contains("unauthorized"));
    let mut anonymous = multipart_request(multipart_import("author", None, b"not a zip"), "");
    anonymous.headers_mut().remove(header::COOKIE);
    let response = make_app!(&env, &env.base).oneshot(anonymous).await.unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(body_string(response).await.contains("unauthorized"));
    let (status, body) = post_server_fn(
        make_app!(&env, &env.base),
        &web::themes::List {
            scope: OwnershipScope::Author,
        },
        Some(&owner.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        serde_json::from_str::<Vec<web::themes::CatalogEntry>>(&body)
            .unwrap()
            .is_empty()
    );
}

#[apply(backends)]
#[tokio::test]
async fn theme_import_zip_creates_drafts_and_rejects_invalid_packages(#[case] backend: Backend) {
    let env = backend.setup().await;
    let themes: std::sync::Arc<dyn storage::ThemeStorage> = std::sync::Arc::clone(&env.themes());

    let owner = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let operator = create_operator_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let storage = TempDir::new().expect("temporary storage");

    let imported = multipart_response(
        make_app!(&env, &storage),
        multipart_body("author", "Imported", &archive("body { color: green; }")),
        &owner.cookie(),
    )
    .await;
    assert_eq!(imported.status(), StatusCode::OK);
    assert_eq!(imported.headers()[header::CACHE_CONTROL], NO_STORE);
    let imported: web::themes::CatalogEntry = confirmed_for(
        serde_json::from_str::<MutationOutcome<_>>(&body_string(imported).await)
            .expect("import outcome JSON"),
        "theme ZIP import",
    );
    assert_eq!(imported.name, "Imported");
    let site_import = multipart_response(
        make_app!(&env, &storage),
        multipart_import(
            "site",
            None,
            &named_archive("Site imported", "body { color: navy; }"),
        ),
        &operator.cookie(),
    )
    .await;
    assert_eq!(site_import.status(), StatusCode::OK);
    let site_catalog = themes
        .list_themes(ThemeOwner::Site)
        .await
        .expect("site catalog lookup");
    assert_eq!(
        site_catalog
            .iter()
            .map(|theme| theme.name.as_str())
            .collect::<Vec<_>>(),
        ["Site imported"]
    );

    let malformed = multipart_response(
        make_app!(&env, &storage),
        multipart_body("author", "Rejected", b"not a zip"),
        &owner.cookie(),
    )
    .await;
    assert_eq!(malformed.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let oversized = multipart_response(
        make_app!(&env, &storage),
        multipart_body(
            "author",
            "Too Large",
            &vec![0_u8; ThemePackageLimits::default().max_archive_bytes + 1],
        ),
        &owner.cookie(),
    )
    .await;
    assert_eq!(oversized.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let catalog = themes
        .list_themes(ThemeOwner::Author(owner.user_id))
        .await
        .expect("catalog lookup");
    assert_eq!(catalog.len(), 1, "failed imports do not create drafts");
    let stored = themes
        .get_draft(ThemeOwner::Author(owner.user_id), imported.id)
        .await
        .expect("draft lookup")
        .expect("imported theme remains");
    assert_eq!(stored.stylesheet, b"body { color: green; }");
}

#[apply(backends)]
#[tokio::test]
async fn cli_package_import_creates_an_unselected_private_draft_without_changing_selection(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let themes: Arc<dyn storage::ThemeStorage> = Arc::clone(&env.themes());
    let owner = create_user_and_session(
        Arc::clone(&env.users()),
        Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let (status, body) = post_server_fn(
        make_app!(&env, &env.base),
        &web::themes::Select {
            scope: OwnershipScope::Author,
            selection: Some(PublicThemeSelection::BuiltIn(Theme::Reader)),
        },
        Some(&owner.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    let repository = TempDir::new().expect("repository");
    fs::write(
        repository.path().join("theme.json"),
        r#"{"defaults":{},"name":"CLI package","schema":1,"style_contract":1,"assets":{}}"#,
    )
    .expect("manifest");
    fs::write(
        repository.path().join("style.css"),
        "body { color: rebeccapurple; }",
    )
    .expect("stylesheet");
    let output = repository.path().join("theme.zip");
    jaunder::commands::cmd_theme_package(repository.path(), &output).expect("CLI package");
    let archive = fs::read(&output).expect("CLI package bytes");

    let response = multipart_response(
        make_app!(&env, &env.base),
        multipart_body("author", "CLI import", &archive),
        &owner.cookie(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let imported: web::themes::CatalogEntry = confirmed_for(
        serde_json::from_str::<MutationOutcome<_>>(&body_string(response).await)
            .expect("import outcome JSON"),
        "CLI package import",
    );
    assert_eq!(
        themes
            .selection(ThemeOwner::Author(owner.user_id))
            .await
            .expect("selection lookup"),
        Some(PublicThemeSelection::BuiltIn(Theme::Reader))
    );
    assert!(
        themes
            .get_draft(ThemeOwner::Author(owner.user_id), imported.id)
            .await
            .expect("draft lookup")
            .is_some(),
        "the imported package remains a private draft"
    );
}

#[apply(backends)]
#[tokio::test]
async fn theme_import_zip_rejects_out_of_order_and_extra_fields(#[case] backend: Backend) {
    let env = backend.setup().await;
    let owner = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let storage = TempDir::new().expect("temporary storage");
    let valid_archive = archive("body { color: green; }");
    let cases = [
        multipart_fields(&[("name", b"Wrong first")]),
        multipart_fields(&[("scope", b"author"), ("extra", b"Wrong second")]),
        multipart_fields(&[
            ("scope", b"author"),
            ("name", b"Wrong third"),
            ("extra", b"not an archive"),
        ]),
        multipart_fields(&[
            ("scope", b"author"),
            ("archive", &valid_archive),
            ("name", b"Late override"),
        ]),
        multipart_fields(&[
            ("scope", b"author"),
            ("name", b"Extra field"),
            ("archive", &valid_archive),
            ("extra", b"unexpected"),
        ]),
    ];

    for body in cases {
        let response = multipart_response(make_app!(&env, &storage), body, &owner.cookie()).await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            body_string(response)
                .await
                .contains("theme import fields must be scope, name, archive"),
            "field order is a public validation error",
        );
    }
}

#[apply(backends)]
#[tokio::test]
async fn theme_import_zip_rejects_truncated_multipart_framing(#[case] backend: Backend) {
    let env = backend.setup().await;
    let owner = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let storage = TempDir::new().expect("temporary storage");
    let body =
        b"------jaunder-theme-test-boundary\r\nContent-Disposition: form-data; name=\"scope\"\r\n\r\nauthor"
            .to_vec();

    let response = multipart_response(make_app!(&env, &storage), body, &owner.cookie()).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        body_string(response)
            .await
            .contains("invalid multipart theme import"),
        "Axum's multipart decoder errors are projected as public validation failures",
    );
}

#[apply(backends)]
#[tokio::test]
async fn theme_import_zip_admission_precedes_archive_parsing_and_is_principal_scoped(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let owner = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let other = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let storage = TempDir::new().expect("temporary storage");
    let app = make_app!(&env, &storage);
    for _ in 0..4 {
        let response = app
            .clone()
            .oneshot(multipart_request(
                multipart_body("invalid", "Rejected", b""),
                &owner.cookie(),
            ))
            .await
            .expect("router accepts invalid ZIP import");
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    let limited = app
        .clone()
        .oneshot(multipart_request(
            multipart_body("author", "Malformed", b"not a zip"),
            &owner.cookie(),
        ))
        .await
        .expect("router accepts rate-limited ZIP import");
    assert_eq!(limited.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        body_string(limited).await.contains("rate limited"),
        "admission rejects before malformed archive parsing"
    );

    let imported = app
        .clone()
        .oneshot(multipart_request(
            multipart_body("author", "Other", &archive("body { color: teal; }")),
            &other.cookie(),
        ))
        .await
        .expect("router accepts independent principal ZIP import");
    assert_eq!(imported.status(), StatusCode::OK);

    let owner_catalog = app
        .clone()
        .oneshot(server_fn_request(
            &web::themes::List {
                scope: OwnershipScope::Author,
            },
            Some(&owner.cookie()),
        ))
        .await
        .expect("router serves owner catalog");
    assert_eq!(owner_catalog.status(), StatusCode::OK);
    assert_eq!(
        serde_json::from_str::<Vec<web::themes::CatalogEntry>>(&body_string(owner_catalog).await)
            .expect("owner catalog JSON"),
        Vec::new(),
        "rejected imports leave the saturated principal's catalog unchanged"
    );

    let other_catalog = app
        .oneshot(server_fn_request(
            &web::themes::List {
                scope: OwnershipScope::Author,
            },
            Some(&other.cookie()),
        ))
        .await
        .expect("router serves independent principal catalog");
    assert_eq!(other_catalog.status(), StatusCode::OK);
    assert_eq!(
        serde_json::from_str::<Vec<web::themes::CatalogEntry>>(&body_string(other_catalog).await)
            .expect("independent principal catalog JSON")
            .len(),
        1
    );
}

#[apply(backends)]
#[tokio::test]
async fn theme_import_zip_validation_releases_the_request_permit(#[case] backend: Backend) {
    let env = backend.setup().await;
    let owner = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let storage = TempDir::new().expect("temporary storage");
    let app = make_app!(&env, &storage);

    let invalid = app
        .clone()
        .oneshot(multipart_request(
            multipart_body("invalid", "Rejected", b""),
            &owner.cookie(),
        ))
        .await
        .expect("router accepts invalid ZIP import");
    assert_eq!(invalid.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let imported = app
        .oneshot(multipart_request(
            multipart_body("author", "Admitted", &archive("body { color: navy; }")),
            &owner.cookie(),
        ))
        .await
        .expect("router accepts import after validation failure");
    assert_eq!(imported.status(), StatusCode::OK);
    let entry: web::themes::CatalogEntry = confirmed_for(
        serde_json::from_str::<MutationOutcome<_>>(&body_string(imported).await)
            .expect("import outcome JSON"),
        "theme ZIP import after validation failure",
    );
    assert_eq!(entry.name, "Admitted");
}

#[apply(backends)]
#[tokio::test]
async fn theme_import_css_and_presentation_are_owner_private(#[case] backend: Backend) {
    let env = backend.setup().await;
    let owner = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let storage = TempDir::new().expect("temporary storage");
    let imported = server_fn_response(
        make_app!(&env, &storage),
        &web::themes::ImportCss {
            scope: OwnershipScope::Author,
            name: "CSS import".to_owned(),
            stylesheet: b"body { color: orange; }".to_vec(),
        },
        Some(&owner.cookie()),
    )
    .await;
    assert_eq!(imported.status(), StatusCode::OK);
    assert_eq!(imported.headers()[header::CACHE_CONTROL], NO_STORE);
    let imported: web::themes::CatalogEntry = confirmed_for(
        serde_json::from_str::<MutationOutcome<_>>(&body_string(imported).await)
            .expect("CSS import outcome JSON"),
        "theme CSS import",
    );
    assert_eq!(imported.name, "CSS import");

    let presentation = server_fn_response(
        make_app!(&env, &storage),
        &web::themes::GetPresentation {
            scope: OwnershipScope::Author,
            theme_id: imported.id,
        },
        Some(&owner.cookie()),
    )
    .await;
    assert_eq!(presentation.status(), StatusCode::OK);
    assert_eq!(presentation.headers()[header::CACHE_CONTROL], NO_STORE);
    assert_eq!(
        serde_json::from_str::<web::themes::ThemePresentation>(&body_string(presentation).await)
            .expect("presentation JSON"),
        web::themes::ThemePresentation {
            logo: None,
            header: None,
            header_pool: Vec::new(),
            shuffle_seed: None,
        }
    );
}

#[apply(backends)]
#[tokio::test]
async fn theme_binding_inputs_persist_and_project_through_server_functions(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let owner = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let storage = TempDir::new().expect("temporary storage");
    let imported = multipart_response(
        make_app!(&env, &storage),
        multipart_body(
            "author",
            "Bindings",
            &archive_with_logo("body { color: orange; }"),
        ),
        &owner.cookie(),
    )
    .await;
    let status = imported.status();
    let body = body_string(imported).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let theme = confirmed_for(
        serde_json::from_str::<MutationOutcome<web::themes::CatalogEntry>>(&body)
            .expect("binding theme import outcome JSON"),
        "binding theme import",
    );
    let media = seed_media(
        std::sync::Arc::clone(&env.media()),
        env.write_scope(),
        owner.user_id,
        "logo.png",
    )
    .await;

    let inputs = [
        ThemeBindingInput::PackagedDefault,
        ThemeBindingInput::ExplicitAbsent,
        ThemeBindingInput::PackageAsset("assets/logo.png".into()),
        ThemeBindingInput::Media(ThemeMediaInput {
            source: media.source,
            sha256: media.sha256.clone(),
            filename: media.filename.clone(),
        }),
    ];

    for input in inputs {
        let (status, body) = post_server_fn(
            make_app!(&env, &storage),
            &web::themes::ReplaceBinding {
                scope: OwnershipScope::Author,
                theme_id: theme.id,
                role: ThemeImageRole::Logo,
                input: input.clone(),
            },
            Some(&owner.cookie()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        confirmed_for(
            serde_json::from_str::<MutationOutcome<()>>(&body)
                .expect("binding replacement outcome JSON"),
            "binding replacement",
        );

        let (status, body) = post_server_fn(
            make_app!(&env, &storage),
            &web::themes::GetPresentation {
                scope: OwnershipScope::Author,
                theme_id: theme.id,
            },
            Some(&owner.cookie()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        let presentation: web::themes::ThemePresentation =
            serde_json::from_str(&body).expect("presentation JSON");
        assert_eq!(presentation.logo, Some(input));
    }

    let (status, body) = post_server_fn(
        make_app!(&env, &storage),
        &web::themes::List {
            scope: OwnershipScope::Author,
        },
        Some(&owner.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(
        serde_json::from_str::<Vec<web::themes::CatalogEntry>>(&body).expect("catalog JSON"),
        vec![theme],
    );
}

#[apply(backends)]
#[tokio::test]
async fn imported_package_defaults_resolve_then_explicit_absence_suppresses_them(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let themes: Arc<dyn storage::ThemeStorage> = Arc::clone(&env.themes());
    let operator = create_operator_and_session(
        Arc::clone(&env.users()),
        Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let storage = TempDir::new().expect("temporary storage");
    let response = multipart_response(
        make_app!(&env, &storage),
        multipart_body(
            "site",
            "Package defaults",
            &archive_with_package_defaults("body { color: orange; }"),
        ),
        &operator.cookie(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let theme: web::themes::CatalogEntry = confirmed_for(
        serde_json::from_str::<MutationOutcome<_>>(&body_string(response).await)
            .expect("package-default import outcome JSON"),
        "package-default theme import",
    );
    assert_eq!(
        themes
            .role_binding(ThemeOwner::Site, theme.id, ThemeImageRole::Logo)
            .await
            .expect("logo binding lookup"),
        None,
        "ZIP import leaves package-default logo binding absent"
    );
    assert_eq!(
        themes
            .role_binding(ThemeOwner::Site, theme.id, ThemeImageRole::Header)
            .await
            .expect("header binding lookup"),
        None,
        "ZIP import leaves package-default header binding absent"
    );
    let (status, body) = post_server_fn(
        make_app!(&env, &storage),
        &web::themes::Publish {
            scope: OwnershipScope::Site,
            theme_id: theme.id,
        },
        Some(&operator.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    confirmed_for(
        serde_json::from_str::<MutationOutcome<()>>(&body).expect("publish outcome JSON"),
        "package-default theme publish",
    );
    let (status, body) = post_server_fn(
        make_app!(&env, &storage),
        &web::themes::Select {
            scope: OwnershipScope::Site,
            selection: Some(PublicThemeSelection::Custom(theme.id)),
        },
        Some(&operator.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    confirmed_for(
        serde_json::from_str::<MutationOutcome<()>>(&body).expect("selection outcome JSON"),
        "package-default theme selection",
    );

    let presentation = storage::resolve_public_theme(
        storage::PublicThemeOwner::Site,
        &PublicThemeRoute::site(),
        themes.as_ref(),
    )
    .await
    .expect("selected package-default public presentation resolves");
    let logo_url = presentation
        .logo_url
        .expect("absent logo binding resolves the package-default logo");
    assert!(
        logo_url.as_ref().starts_with("/theme/"),
        "package-default logo resolves to a public package asset URL"
    );
    let header_url = presentation
        .header_url
        .expect("absent header binding resolves a package-default header");
    assert!(
        header_url.as_ref().starts_with("/theme/"),
        "package-default header resolves to a public package asset URL"
    );
    let repeated_presentation = storage::resolve_public_theme(
        storage::PublicThemeOwner::Site,
        &PublicThemeRoute::site(),
        themes.as_ref(),
    )
    .await
    .expect("selected package-default public presentation resolves repeatedly");
    assert_eq!(
        repeated_presentation.header_url,
        Some(header_url),
        "package-default header selection is deterministic for the same route and revision"
    );

    for role in [ThemeImageRole::Logo, ThemeImageRole::Header] {
        let (status, body) = post_server_fn(
            make_app!(&env, &storage),
            &web::themes::ReplaceBinding {
                scope: OwnershipScope::Site,
                theme_id: theme.id,
                role,
                input: ThemeBindingInput::ExplicitAbsent,
            },
            Some(&operator.cookie()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        confirmed_for(
            serde_json::from_str::<MutationOutcome<()>>(&body)
                .expect("explicit-absence binding outcome JSON"),
            "explicit-absence binding",
        );
    }

    let suppressed_presentation = storage::resolve_public_theme(
        storage::PublicThemeOwner::Site,
        &PublicThemeRoute::site(),
        themes.as_ref(),
    )
    .await
    .expect("selected public presentation resolves after explicit absence");
    assert_eq!(
        suppressed_presentation.logo_url, None,
        "persisted explicit absence suppresses the package-default public logo"
    );
    assert_eq!(
        suppressed_presentation.header_url, None,
        "persisted explicit absence suppresses the package-default public header"
    );
}

#[apply(backends)]
#[tokio::test]
async fn theme_presentation_projects_a_seeded_header_pool(#[case] backend: Backend) {
    let env = backend.setup().await;
    let owner = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let storage = TempDir::new().expect("temporary storage");
    let theme = create_author_theme(
        make_app!(&env, &storage),
        &owner.cookie(),
        "Pooled",
        "body { color: orange; }",
    )
    .await;
    let media = seed_media(
        std::sync::Arc::clone(&env.media()),
        env.write_scope(),
        owner.user_id,
        "header.png",
    )
    .await;
    let manager = ThemeManager::new(
        std::sync::Arc::clone(&env.themes()),
        std::sync::Arc::clone(&env.media()),
        env.write_scope(),
        Arc::new(env.media_content_locks()),
    );
    let shuffle_seed = [7; 32];
    confirmed_for(
        manager
            .replace_header_pool(
                owner.user_id,
                ThemeOwner::Author(owner.user_id),
                theme.id,
                vec![StorageThemePoolInput::Media(media.clone())],
                shuffle_seed,
            )
            .await
            .expect("seed header-pool state"),
        "header-pool seed",
    );

    let response = server_fn_response(
        make_app!(&env, &storage),
        &web::themes::GetPresentation {
            scope: OwnershipScope::Author,
            theme_id: theme.id,
        },
        Some(&owner.cookie()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], NO_STORE);
    let presentation: web::themes::ThemePresentation =
        serde_json::from_str(&body_string(response).await).expect("presentation JSON");
    assert_eq!(presentation.header, None);
    assert_eq!(presentation.shuffle_seed, Some(shuffle_seed));
    assert_eq!(
        presentation.header_pool,
        vec![ThemePoolInput::Media(web::themes::ThemeMediaInput {
            source: media.source,
            sha256: media.sha256,
            filename: media.filename,
        })],
    );
}

#[apply(backends)]
#[tokio::test]
async fn theme_selection_rejects_unpublished_custom_and_preview_isolated(#[case] backend: Backend) {
    let env = backend.setup().await;
    let themes: std::sync::Arc<dyn storage::ThemeStorage> = std::sync::Arc::clone(&env.themes());

    let owner = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let theme = create_author_theme(
        make_app!(&env, &env.base),
        &owner.cookie(),
        "Preview",
        "body { color: purple; }",
    )
    .await;
    let storage = TempDir::new().expect("temporary storage");

    let (select_status, select_body) = post_server_fn(
        make_app!(&env, &env.base),
        &web::themes::Select {
            scope: OwnershipScope::Author,
            selection: Some(PublicThemeSelection::Custom(theme.id)),
        },
        Some(&owner.cookie()),
    )
    .await;
    assert_eq!(
        select_status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "body: {select_body}"
    );
    assert!(select_body.contains("not found"), "body: {select_body}");

    let preview = server_fn_response(
        make_app!(&env, &storage),
        &web::themes::Preview {
            scope: OwnershipScope::Author,
            theme_id: theme.id,
        },
        Some(&owner.cookie()),
    )
    .await;
    assert_eq!(preview.status(), StatusCode::OK);
    assert_eq!(preview.headers()[header::CACHE_CONTROL], NO_STORE);
    let preview: web::themes::ThemePreview =
        serde_json::from_str(&body_string(preview).await).expect("preview JSON");
    assert!(preview.html.contains("data-jaunder-theme-surface"));
    assert!(preview.css.contains("purple"), "CSS: {}", preview.css);

    let operator = create_operator_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let (site_status, site_body) = post_server_fn(
        make_app!(&env, &env.base),
        &web::themes::Create {
            scope: OwnershipScope::Site,
            name: "Site preview".to_owned(),
            draft: package_draft("body { color: teal; }"),
        },
        Some(&operator.cookie()),
    )
    .await;
    assert_eq!(site_status, StatusCode::OK, "body: {site_body}");
    let site_theme: web::themes::CatalogEntry = confirmed_for(
        serde_json::from_str::<MutationOutcome<_>>(&site_body).expect("site create outcome JSON"),
        "site theme creation",
    );
    let site_preview = server_fn_response(
        make_app!(&env, &storage),
        &web::themes::Preview {
            scope: OwnershipScope::Site,
            theme_id: site_theme.id,
        },
        Some(&operator.cookie()),
    )
    .await;
    assert_eq!(site_preview.status(), StatusCode::OK);
    let site_preview: web::themes::ThemePreview =
        serde_json::from_str(&body_string(site_preview).await).expect("site preview JSON");
    assert!(site_preview.html.contains("data-jaunder-theme-surface"));
    assert!(
        site_preview.css.contains("teal"),
        "CSS: {}",
        site_preview.css
    );

    assert_eq!(
        themes
            .selection(ThemeOwner::Author(owner.user_id))
            .await
            .expect("selection lookup"),
        None,
        "preview must not select the draft"
    );
}
