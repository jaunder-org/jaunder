use std::collections::BTreeMap;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use common::{MutationOutcome, theme::PublicThemeSelection};
use host::theme_package::{ThemePackageLimits, export_theme_package, validate_theme_package};
use rstest::*;
use rstest_reuse::*;
use server_fn::ServerFn;
use storage::ThemeOwner;
use storage::test_support::{Backend, TestEnv, backends, confirmed_for};
use tempfile::TempDir;
use tower::ServiceExt;
use web::themes::{Draft, OwnershipScope};

use crate::helpers::{
    body_string, create_operator_and_session, create_user_and_session, make_app, post_server_fn,
};

const NO_STORE: &str = "private, no-store";

fn package_draft(css: &str) -> Draft {
    Draft {
        manifest: br#"{"schema":1,"name":"Paper","style_contract":1,"assets":{},"defaults":{}}"#
            .to_vec(),
        stylesheet: css.as_bytes().to_vec(),
        assets: Vec::new(),
    }
}

fn archive(css: &str) -> Vec<u8> {
    export_theme_package(
        br#"{"schema":1,"name":"Paper","style_contract":1,"assets":{},"defaults":{}}"#,
        css.as_bytes(),
        &BTreeMap::new(),
    )
    .expect("no-asset theme package exports")
}

async fn create_author_theme(
    state: &std::sync::Arc<storage::AppState>,
    cookie: &str,
    name: &str,
    css: &str,
) -> web::themes::CatalogEntry {
    let (status, body) = post_server_fn(
        state,
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
    state: &std::sync::Arc<storage::AppState>,
    storage: &TempDir,
    input: &F,
    cookie: Option<&str>,
) -> axum::response::Response
where
    F: serde::Serialize + ServerFn,
{
    make_app(state, storage)
        .oneshot(server_fn_request(input, cookie))
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
    state: &std::sync::Arc<storage::AppState>,
    storage: &TempDir,
    body: Vec<u8>,
    cookie: &str,
) -> axum::response::Response {
    make_app(state, storage)
        .oneshot(multipart_request(body, cookie))
        .await
        .expect("router accepts multipart request")
}

fn multipart_body(scope: &str, name: &str, archive: &[u8]) -> Vec<u8> {
    let boundary = "----jaunder-theme-test-boundary";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"scope\"\r\n\r\n{scope}\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"name\"\r\n\r\n{name}\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"archive\"; filename=\"theme.zip\"\r\nContent-Type: application/zip\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(archive);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

#[apply(backends)]
#[tokio::test]
async fn theme_catalog_enforces_authentication_and_scope(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let member = create_user_and_session(&state).await;
    let operator = create_operator_and_session(&state).await;

    let (anonymous_status, anonymous_body) = post_server_fn(
        &state,
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
        &state,
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
        &state,
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
        &state,
        &web::themes::List {
            scope: OwnershipScope::Site,
        },
        Some(&operator.cookie()),
    )
    .await;
    assert_eq!(site_status, StatusCode::OK, "body: {site_body}");
}

#[apply(backends)]
#[tokio::test]
async fn theme_drafts_mask_foreign_ids_and_never_cache_private_responses(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let owner = create_user_and_session(&state).await;
    let stranger = create_user_and_session(&state).await;
    let theme =
        create_author_theme(&state, &owner.cookie(), "Draft", "body { color: navy; }").await;
    let storage = TempDir::new().expect("temporary storage");

    let draft = server_fn_response(
        &state,
        &storage,
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
        &state,
        &storage,
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
    let TestEnv { state, base: _base } = backend.setup().await;
    let owner = create_user_and_session(&state).await;
    let theme = create_author_theme(
        &state,
        &owner.cookie(),
        "dangerous; filename=owned\".zip",
        "body { color: teal; }",
    )
    .await;
    let storage = TempDir::new().expect("temporary storage");
    let response = server_fn_response(
        &state,
        &storage,
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
    let stranger = create_user_and_session(&state).await;
    let foreign = server_fn_response(
        &state,
        &storage,
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
async fn theme_import_zip_creates_drafts_and_rejects_invalid_packages(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let owner = create_user_and_session(&state).await;
    let storage = TempDir::new().expect("temporary storage");

    let imported = multipart_response(
        &state,
        &storage,
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

    let malformed = multipart_response(
        &state,
        &storage,
        multipart_body("author", "Rejected", b"not a zip"),
        &owner.cookie(),
    )
    .await;
    assert_eq!(malformed.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let oversized = multipart_response(
        &state,
        &storage,
        multipart_body(
            "author",
            "Too Large",
            &vec![0_u8; ThemePackageLimits::default().max_archive_bytes + 1],
        ),
        &owner.cookie(),
    )
    .await;
    assert_eq!(oversized.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let catalog = state
        .themes
        .list_themes(ThemeOwner::Author(owner.user_id))
        .await
        .expect("catalog lookup");
    assert_eq!(catalog.len(), 1, "failed imports do not create drafts");
    let stored = state
        .themes
        .get_draft(ThemeOwner::Author(owner.user_id), imported.id)
        .await
        .expect("draft lookup")
        .expect("imported theme remains");
    assert_eq!(stored.stylesheet, b"body { color: green; }");
}

#[apply(backends)]
#[tokio::test]
async fn theme_import_admission_precedes_archive_parsing_and_is_principal_scoped(
    #[case] backend: Backend,
) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let owner = create_user_and_session(&state).await;
    let other = create_user_and_session(&state).await;
    let storage = TempDir::new().expect("temporary storage");
    let app = make_app(&state, &storage);
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
async fn theme_import_validation_releases_the_request_permit(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let owner = create_user_and_session(&state).await;
    let storage = TempDir::new().expect("temporary storage");
    let app = make_app(&state, &storage);

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
    let TestEnv { state, base: _base } = backend.setup().await;
    let owner = create_user_and_session(&state).await;
    let storage = TempDir::new().expect("temporary storage");
    let imported = server_fn_response(
        &state,
        &storage,
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
        &state,
        &storage,
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
async fn theme_selection_rejects_unpublished_custom_and_preview_isolated(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let owner = create_user_and_session(&state).await;
    let theme = create_author_theme(
        &state,
        &owner.cookie(),
        "Preview",
        "body { color: purple; }",
    )
    .await;
    let storage = TempDir::new().expect("temporary storage");

    let (select_status, select_body) = post_server_fn(
        &state,
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
        &state,
        &storage,
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

    assert_eq!(
        state
            .themes
            .selection(ThemeOwner::Author(owner.user_id))
            .await
            .expect("selection lookup"),
        None,
        "preview must not select the draft"
    );
}
