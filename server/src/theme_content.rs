use std::{io, path::PathBuf, sync::Arc};

use axum::{
    Extension, Router,
    body::Body,
    extract::{OriginalUri, Path, rejection::PathRejection},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware,
    response::Response,
    routing,
};
use common::{media::ContentHash, theme::ThemeContentDigest};
use host::etag;
use storage::ThemeStorage;
use tokio::fs;
use tokio_util::io::ReaderStream;
use web::error::InternalError;

const CACHE_CONTROL: HeaderValue = HeaderValue::from_static("public, max-age=31536000, immutable");
const NOSNIFF: HeaderValue = HeaderValue::from_static("nosniff");

/// Registers immutable public custom-theme content routes.
///
/// The handler takes only its declared storage dependencies through extensions,
/// so this router remains composable with the application's state-free routes.
pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new().nest(
        "/theme",
        Router::new()
            .route(
                "/draft/{theme_id}/{*path}",
                routing::get(serve_draft).layer(middleware::map_response(private_no_store)),
            )
            .route("/{digest}", routing::get(serve))
            // Do not let malformed theme-content addresses fall through to the CSR
            // shell. The typed route above is the only public content address.
            .fallback(not_found),
    )
}

async fn not_found() -> StatusCode {
    StatusCode::NOT_FOUND
}
async fn private_no_store(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    response
}

/// Serves a package asset from an unselected draft only to its catalog owner.
async fn serve_draft(
    user: web::auth::User,
    Extension(themes): Extension<Arc<dyn ThemeStorage>>,
    Extension(users): Extension<Arc<dyn storage::UserStorage>>,
    Path((theme_id, path)): Path<(common::ids::ThemeId, String)>,
) -> Result<Response, StatusCode> {
    let author_owner = storage::ThemeOwner::Author(user.user_id);
    let site_owner = users
        .get_user(user.user_id)
        .await
        .map_err(storage_failure)?
        .filter(|record| record.is_operator.is_operator())
        .map(|_| storage::ThemeOwner::Site);
    let draft = themes
        .get_draft(author_owner, theme_id)
        .await
        .map_err(storage_failure)?
        .or(match site_owner {
            Some(owner) => themes
                .get_draft(owner, theme_id)
                .await
                .map_err(storage_failure)?,
            None => None,
        })
        .ok_or(StatusCode::NOT_FOUND)?;
    let asset = draft
        .assets
        .into_iter()
        .find(|asset| asset.path == path)
        .ok_or(StatusCode::NOT_FOUND)?;
    let content_type =
        HeaderValue::from_str(&asset.mime).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut response = Response::new(Body::from(asset.bytes));
    *response.status_mut() = StatusCode::OK;
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, content_type);
    response
        .headers_mut()
        .insert(header::X_CONTENT_TYPE_OPTIONS, NOSNIFF);
    Ok(response)
}

/// Serves immutable custom-theme content only after durable eligibility admits it.
#[tracing::instrument(name = "theme_content.serve", skip_all)]
async fn serve(
    Extension(themes): Extension<Arc<dyn ThemeStorage>>,
    Extension(storage_path): Extension<Arc<PathBuf>>,
    digest: Result<Path<ThemeContentDigest>, PathRejection>,
    OriginalUri(uri): OriginalUri,
    request_headers: HeaderMap,
) -> Result<Response, StatusCode> {
    let Path(digest) = digest.map_err(|_| StatusCode::NOT_FOUND)?;
    if uri.path().strip_prefix("/theme/") != Some(digest.as_ref()) {
        return Err(StatusCode::NOT_FOUND);
    }

    let Some(eligibility) = themes
        .content_eligibility(&digest)
        .await
        .map_err(storage_failure)?
    else {
        return Err(StatusCode::NOT_FOUND);
    };

    let etag = theme_etag(&digest)?;
    if crate::feed::conditional::if_none_match_matches(&request_headers, etag.as_ref().as_bytes()) {
        return response(
            StatusCode::NOT_MODIFIED,
            Body::empty(),
            &eligibility.mime,
            &etag,
        );
    }

    let path = content_path(&storage_path, &digest);
    let file = match fs::File::open(path).await {
        Ok(file) => file,
        Err(error) => return Err(file_failure(error)),
    };
    let body = Body::from_stream(ReaderStream::new(file));

    response(StatusCode::OK, body, &eligibility.mime, &etag)
}

fn response(
    status: StatusCode,
    body: Body,
    mime: &str,
    etag: &common::etag::ETag,
) -> Result<Response, StatusCode> {
    let content_type = HeaderValue::from_str(mime).map_err(|_| {
        InternalError::server(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid stored theme MIME",
        ))
        .with_context("boundary", "server.theme_content.response")
        .emit_boundary_failure();
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let etag =
        HeaderValue::from_str(etag.as_ref()).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut response = Response::new(body);
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, content_type);
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, NOSNIFF);
    headers.insert(header::CACHE_CONTROL, CACHE_CONTROL);
    headers.insert(header::ETAG, etag);
    Ok(response)
}

fn content_path(storage_path: &std::path::Path, digest: &ThemeContentDigest) -> PathBuf {
    let digest = digest.as_ref();
    storage_path
        .join("themes")
        .join(&digest[..2])
        .join(&digest[2..4])
        .join(digest)
}

fn theme_etag(digest: &ThemeContentDigest) -> Result<common::etag::ETag, StatusCode> {
    let hash = digest
        .as_ref()
        .parse::<ContentHash>()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(etag::from_content_hash(&hash))
}

fn storage_failure(error: sqlx::Error) -> StatusCode {
    InternalError::server(error)
        .with_context("boundary", "server.theme_content.eligibility")
        .emit_boundary_failure();
    StatusCode::INTERNAL_SERVER_ERROR
}

fn file_failure(error: io::Error) -> StatusCode {
    InternalError::server(error)
        .with_context("boundary", "server.theme_content.open")
        .emit_boundary_failure();
    StatusCode::INTERNAL_SERVER_ERROR
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(theme_id: common::ids::ThemeId) -> storage::ThemeDraft {
        let source_digest: common::theme::ThemeSourceDigest =
            DIGEST.parse().expect("valid source digest");
        let asset_digest: common::theme::ThemeAssetDigest =
            DIGEST.parse().expect("valid asset digest");
        storage::ThemeDraft {
            theme_id,
            manifest: Vec::new(),
            stylesheet: Vec::new(),
            source_digest,
            assets: vec![storage::ThemeDraftAsset {
                path: "assets/logo.png".to_owned(),
                mime: "image/png".to_owned(),
                bytes: b"draft asset".to_vec(),
                digest: asset_digest,
            }],
        }
    }

    fn operator(user_id: common::ids::UserId) -> storage::UserRecord {
        storage::UserRecord {
            user_id,
            username: "operator".parse().expect("valid username"),
            display_name: None,
            bio: None,
            created_at: common::time::UtcInstant::now(),
            last_authenticated_at: None,
            email: None,
            email_verified: storage::EmailVerified::UNVERIFIED,
            is_operator: storage::OperatorStatus::OPERATOR,
        }
    }

    #[tokio::test]
    async fn operator_draft_lookup_falls_back_to_the_site_catalog() {
        let user_id = common::ids::UserId::from(1);
        let theme_id = common::ids::ThemeId::from(7);
        let mut users = storage::MockUserStorage::new();
        users
            .expect_get_user()
            .withf(move |actual| *actual == user_id)
            .return_once(move |_| Ok(Some(operator(user_id))));
        let mut themes = storage::MockThemeStorage::new();
        themes
            .expect_get_draft()
            .withf(move |owner, actual| {
                *owner == storage::ThemeOwner::Author(user_id) && *actual == theme_id
            })
            .return_once(|_, _| Ok(None));
        themes
            .expect_get_draft()
            .withf(move |owner, actual| *owner == storage::ThemeOwner::Site && *actual == theme_id)
            .return_once(move |_, _| Ok(Some(draft(theme_id))));

        let response = serve_draft(
            web::auth::User {
                user_id,
                username: "operator".parse().expect("valid username"),
                token_hash: common::token::TokenHash::from_digest("test"),
            },
            Extension(Arc::new(themes) as Arc<dyn ThemeStorage>),
            Extension(Arc::new(users) as Arc<dyn storage::UserStorage>),
            Path((theme_id, "assets/logo.png".to_owned())),
        )
        .await
        .expect("operator may read a site draft asset");

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        assert_eq!(body.as_ref(), b"draft asset");
    }

    const DIGEST: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn response_rejects_invalid_stored_mime_without_serving_content() {
        let digest: ThemeContentDigest = DIGEST.parse().expect("valid content digest");
        let etag = theme_etag(&digest).expect("digest produces an ETag");

        assert!(matches!(
            response(StatusCode::OK, Body::empty(), "invalid\r\nmime", &etag),
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        ));
    }

    #[test]
    fn storage_failures_are_internal_errors() {
        assert_eq!(
            storage_failure(sqlx::Error::PoolClosed),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
