use std::{collections::BTreeMap, sync::Arc};

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use common::{
    MutationOutcome,
    theme::{PublicThemeSelection, ThemeImageRole},
};
use host::theme_package::{ThemePackageLimits, export_theme_package, validate_theme_package};
use rstest::*;
use rstest_reuse::*;
use server_fn::ServerFn;
use storage::test_support::{Backend, backends, confirmed_for, seed_media};
use storage::{ThemeManager, ThemeOwner, ThemePoolInput as StorageThemePoolInput};
use tempfile::TempDir;
use tower::ServiceExt;
use web::themes::{Draft, OwnershipScope, ThemeBindingInput, ThemeMediaInput, ThemePoolInput};

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

fn archive_with_logo(css: &str) -> Vec<u8> {
    export_theme_package(
        br#"{"schema":1,"name":"Bindings","style_contract":1,"assets":{"assets/logo.png":"image/png"},"defaults":{}}"#,
        css.as_bytes(),
        &BTreeMap::from([(
            "assets/logo.png".into(),
            b"\x89\x50\x4e\x47\x0d\x0a\x1a\x0a\x00\x00\x00\x0d\x49\x48\x44\x52\x00\x00\x00\x01\x00\x00\x00\x01\x08\x04\x00\x00\x00\xb5\x1c\x0c\x02\x00\x00\x00\x0b\x49\x44\x41\x54\x78\xda\x63\x64\xf8\x0f\x00\x01\x05\x01\x01\x27\x18\xe3\x66\x00\x00\x00\x00\x49\x45\x4e\x44\xae\x42\x60\x82".to_vec(),
        )]),
    )
    .expect("valid package asset exports")
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
    let boundary = "----jaunder-theme-test-boundary";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"scope\"\r\n\r\n{scope}\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"name\"\r\n\r\n{name}\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"archive\"; filename=\"theme.zip\"\r\nContent-Type: application/zip\r\n\r\n"
    )
    .into_bytes();
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
        multipart_body("site", "Site imported", &archive("body { color: navy; }")),
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
        multipart_fields(&[("scope", b"author"), ("archive", b"Wrong second")]),
        multipart_fields(&[
            ("scope", b"author"),
            ("name", b"Wrong third"),
            ("extra", b"not an archive"),
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
async fn theme_import_admission_precedes_archive_parsing_and_is_principal_scoped(
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
async fn theme_import_validation_releases_the_request_permit(#[case] backend: Backend) {
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
