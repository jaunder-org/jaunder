//! Cross-crate test fixtures for `common`'s domain types, gated by the
//! `test-support` feature (mirroring `storage::test_support`, ADR-0033): `common`'s
//! own tests reach it under `cfg(test)`; `storage`, `server`, and `web` reach it via
//! the `test-support` feature. Kept out of shipped binaries.

mod identity;

pub use identity::{
    parse_audience_name, parse_bio, parse_display_name, parse_email, parse_raw_token,
    parse_session_label, parse_smtp_password, parse_smtp_username, parse_token_hash,
    parse_username,
};

mod content;

pub use content::{
    parse_post_body, parse_post_summary, parse_post_title, parse_site_title, parse_slug, parse_tag,
    parse_tag_label,
};

mod media;

pub use media::{
    MEDIA_TEST_SHA256, parse_byte_size, parse_content_hash, parse_content_type, parse_filename,
    parse_max_file_size, parse_user_quota,
};

mod urls_time;

pub use urls_time::{
    parse_etag, parse_root_relative_url, parse_url, parse_utc_instant, permalink_date,
};

mod numbers;

pub use numbers::{
    parse_destination_path, parse_invite_ttl_hours, parse_page_offset, parse_page_size,
    parse_retention_count, parse_row_limit,
};
