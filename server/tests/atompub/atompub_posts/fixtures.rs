use axum::http::header;

pub(super) fn entry_xml(title: &str, content_type: &str, content: &str) -> String {
    format!(
        r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>{title}</title>
  <content type="{content_type}">{content}</content>
  <category term="rust"/>
</entry>"#
    )
}

/// Extracts the created post's id from a `POST` response's `Location` header.
pub(super) fn location_post_id(response: &axum::response::Response) -> i64 {
    response
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|p| p.rsplit('/').next())
        .and_then(|id| id.parse::<i64>().ok())
        .expect("Location header should carry the new post id")
}

pub(super) fn etag_of(response: &axum::response::Response) -> String {
    response
        .headers()
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("response has an ETag header")
        .to_string()
}
