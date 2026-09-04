//! RFC 9110 conditional-request parsing for cached Syndication Feed responses.

use std::time::SystemTime;

use axum::http::{HeaderMap, header};

/// Limits ignored empty list members so malformed input cannot consume unbounded work.
const MAX_IGNORED_EMPTY_ELEMENTS: usize = 16;

/// Whether the request's cache validators select the current representation.
///
/// A present `If-None-Match` always decides the result, including when malformed;
/// `If-Modified-Since` is considered only when the entity-tag field is absent.
pub(crate) fn is_not_modified(
    headers: &HeaderMap,
    current_etag: &[u8],
    last_modified: SystemTime,
) -> bool {
    if headers.contains_key(header::IF_NONE_MATCH) {
        return if_none_match_matches(headers, current_etag);
    }

    if_modified_since(headers).is_some_and(|date| last_modified <= date)
}

fn if_none_match_matches(headers: &HeaderMap, current_etag: &[u8]) -> bool {
    let mut condition = EntityTagCondition::default();

    for value in &headers.get_all(header::IF_NONE_MATCH) {
        if parse_field_value(value.as_bytes(), current_etag, &mut condition).is_err() {
            return false;
        }
    }

    condition.matches()
}

#[derive(Default)]
struct EntityTagCondition {
    saw_wildcard: bool,
    saw_entity_tag: bool,
    matched: bool,
    ignored_empty_elements: usize,
}

impl EntityTagCondition {
    fn empty_element(&mut self) -> Result<(), ()> {
        if self.saw_wildcard {
            return Err(());
        }
        self.ignored_empty_elements += 1;
        (self.ignored_empty_elements <= MAX_IGNORED_EMPTY_ELEMENTS)
            .then_some(())
            .ok_or(())
    }

    fn wildcard(&mut self) -> Result<(), ()> {
        if self.saw_wildcard || self.saw_entity_tag || self.ignored_empty_elements != 0 {
            return Err(());
        }
        self.saw_wildcard = true;
        Ok(())
    }

    fn entity_tag(&mut self, opaque_tag: &[u8], current_etag: &[u8]) -> Result<(), ()> {
        if self.saw_wildcard {
            return Err(());
        }
        self.saw_entity_tag = true;
        self.matched |= opaque_tag == current_etag;
        Ok(())
    }

    fn matches(self) -> bool {
        self.saw_wildcard || self.matched
    }
}

fn parse_field_value(
    value: &[u8],
    current_etag: &[u8],
    condition: &mut EntityTagCondition,
) -> Result<(), ()> {
    let mut cursor = 0;

    loop {
        skip_ows(value, &mut cursor);
        if cursor == value.len() {
            return condition.empty_element();
        }

        if value[cursor] == b',' {
            condition.empty_element()?;
            cursor += 1;
            continue;
        }

        if value[cursor] == b'*' {
            cursor += 1;
            skip_ows(value, &mut cursor);
            if cursor != value.len() {
                return Err(());
            }
            return condition.wildcard();
        }

        let opaque_tag = parse_entity_tag(value, &mut cursor)?;
        condition.entity_tag(opaque_tag, current_etag)?;

        skip_ows(value, &mut cursor);
        if cursor == value.len() {
            return Ok(());
        }
        if value[cursor] != b',' {
            return Err(());
        }
        cursor += 1;
    }
}

fn parse_entity_tag<'a>(value: &'a [u8], cursor: &mut usize) -> Result<&'a [u8], ()> {
    if value.get(*cursor..(*cursor + 2)) == Some(b"W/") {
        *cursor += 2;
    }
    if value.get(*cursor) != Some(&b'"') {
        return Err(());
    }

    let start = *cursor;
    *cursor += 1;
    while let Some(&byte) = value.get(*cursor) {
        if byte == b'"' {
            *cursor += 1;
            return Ok(&value[start..*cursor]);
        }
        if !is_etagc(byte) {
            return Err(());
        }
        *cursor += 1;
    }

    Err(())
}

fn skip_ows(value: &[u8], cursor: &mut usize) {
    while matches!(value.get(*cursor), Some(b' ' | b'\t')) {
        *cursor += 1;
    }
}

fn is_etagc(byte: u8) -> bool {
    byte == 0x21 || (0x23..=0x7e).contains(&byte) || byte >= 0x80
}

fn if_modified_since(headers: &HeaderMap) -> Option<SystemTime> {
    let mut values = headers.get_all(header::IF_MODIFIED_SINCE).iter();
    let value = values.next()?;
    if values.next().is_some() {
        return None;
    }
    let value = std::str::from_utf8(value.as_bytes()).ok()?;
    httpdate::parse_http_date(value).ok()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use axum::http::{HeaderMap, HeaderValue, header};

    use super::*;

    fn headers(values: &[&[u8]]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for value in values {
            headers.append(
                header::IF_NONE_MATCH,
                HeaderValue::from_bytes(value).expect("valid header bytes"),
            );
        }
        headers
    }

    #[test]
    fn parses_weak_tags_lists_repeated_fields_ows_and_obs_text() {
        let headers = headers(&[b" \tW/\"other\" , \"match\" ", b"\"\x80\xff\""]);
        assert!(if_none_match_matches(&headers, b"\"match\""));
    }

    #[test]
    fn accepts_a_bounded_number_of_empty_list_elements() {
        let empty = b", ,\t,";
        let request_headers = headers(&[empty, empty, empty, empty, b"\"match\""]);
        assert!(if_none_match_matches(&request_headers, b"\"match\""));

        let too_many = [b','; MAX_IGNORED_EMPTY_ELEMENTS + 1];
        let headers = headers(&[&too_many]);
        assert!(!if_none_match_matches(&headers, b"\"match\""));
    }

    #[test]
    fn wildcard_must_be_the_only_member() {
        assert!(if_none_match_matches(&headers(&[b" *\t"]), b"\"match\""));
        for value in [b"*, \"match\"".as_slice(), b"\"match\", *", b",*"] {
            assert!(!if_none_match_matches(&headers(&[value]), b"\"match\""));
        }

        assert!(!if_none_match_matches(&headers(&[b"*", b""]), b"\"match\""));
    }

    #[test]
    fn malformed_or_nonmatching_conditions_never_match() {
        for value in [
            b"\"other\"".as_slice(),
            b"W/\"unterminated",
            b"\"bad space\"",
            b"w/\"match\"",
            b"\"match\" trailing",
            b"\"match\" \"other\"",
        ] {
            assert!(!if_none_match_matches(&headers(&[value]), b"\"match\""));
        }
    }

    #[test]
    fn if_none_match_presence_suppresses_if_modified_since() {
        let last_modified = UNIX_EPOCH + Duration::from_secs(1_000);
        let mut headers = headers(&[b"\"other\""]);
        headers.insert(
            header::IF_MODIFIED_SINCE,
            HeaderValue::from_static("Thu, 01 Jan 1970 00:33:20 GMT"),
        );
        assert!(!is_not_modified(&headers, b"\"match\"", last_modified));

        headers.insert(header::IF_NONE_MATCH, HeaderValue::from_static("broken"));
        assert!(!is_not_modified(&headers, b"\"match\"", last_modified));
    }

    #[test]
    fn parses_all_http_date_forms_and_rejects_multiple_values() {
        let expected = UNIX_EPOCH + Duration::from_secs(784_111_777);
        for value in [
            "Sun, 06 Nov 1994 08:49:37 GMT",
            "Sunday, 06-Nov-94 08:49:37 GMT",
            "Sun Nov  6 08:49:37 1994",
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(
                header::IF_MODIFIED_SINCE,
                HeaderValue::from_str(value).expect("valid date header"),
            );
            assert_eq!(if_modified_since(&headers), Some(expected));
        }

        let mut headers = HeaderMap::new();
        headers.append(
            header::IF_MODIFIED_SINCE,
            HeaderValue::from_static("Sun, 06 Nov 1994 08:49:37 GMT"),
        );
        headers.append(
            header::IF_MODIFIED_SINCE,
            HeaderValue::from_static("Sun, 06 Nov 1994 08:49:37 GMT"),
        );
        assert_eq!(if_modified_since(&headers), None);
    }
}
