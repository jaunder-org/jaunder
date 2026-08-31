//! Pure normalization of an Org post's leading Jaunder metadata block.

use std::str::FromStr;

use chrono::{LocalResult, NaiveDate, NaiveDateTime, TimeZone};
use chrono_tz::Tz;
use orgize::{
    Org,
    ast::{Document, Keyword},
    rowan::ast::AstNode,
};
use thiserror::Error;

use crate::{
    etag::ETag,
    ids::{AudienceId, PostId},
    post_body::PostBody,
    post_summary::PostSummary,
    post_title::PostTitle,
    render::{self, PostFormat},
    slug::Slug,
    tag::{self, TagLabel},
    time::UtcInstant,
    visibility::AudienceTarget,
};

/// Whether an ingress explicitly supplied a field. `Present(Vec::new())` is
/// deliberately distinct from [`Absent`](Self::Absent): an empty collection is
/// an instruction, not an omitted field.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Presence<T> {
    #[default]
    Absent,
    Present(T),
}

/// The lifecycle is a single input unit: callers cannot accidentally combine a
/// transport status with an Org publication time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicationState {
    Draft,
    Scheduled(UtcInstant),
    Published(UtcInstant),
}

impl PublicationState {
    /// Return the persistence timestamp represented by this present lifecycle state.
    #[must_use]
    pub const fn published_at(self) -> Option<UtcInstant> {
        match self {
            Self::Draft => None,
            Self::Scheduled(at) | Self::Published(at) => Some(at),
        }
    }
}

/// Structured values supplied by an ingress before Org headers are considered.
///
/// An absent scalar is omitted; scalar clearing is a surface concern after
/// normalization rather than a second `Option` state here.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OrgStructuredMetadata {
    pub title: Presence<PostTitle>,
    pub summary: Presence<PostSummary>,
    pub tags: Presence<Vec<TagLabel>>,
    pub audiences: Presence<Vec<AudienceTarget>>,
    pub lifecycle: Presence<PublicationState>,
}

/// The operation-specific identity available to pure metadata validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrgOperation {
    Create,
    Update { post_id: PostId },
}

/// Bookkeeping parsed from non-authoritative `JAUNDER_*` properties. Final
/// comparisons against storage intentionally happen outside this module.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OrgBookkeeping {
    pub slug: Option<Slug>,
    pub format: Option<PostFormat>,
    pub post_id: Option<PostId>,
    pub synced: Option<ETag>,
    pub synced_at: Option<UtcInstant>,
    pub date_utc: Option<UtcInstant>,
}

/// The effective fields after per-field structured/header precedence is applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrgEffectiveMetadata {
    pub title: Presence<PostTitle>,
    pub summary: Presence<PostSummary>,
    pub tags: Presence<Vec<TagLabel>>,
    pub audiences: Presence<Vec<AudienceTarget>>,
    pub lifecycle: Presence<PublicationState>,
}

/// Successful pure Org normalization.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrgNormalization {
    pub body: PostBody,
    pub metadata: OrgEffectiveMetadata,
    pub bookkeeping: OrgBookkeeping,
}

/// A rejected Org metadata block. The caller maps this single semantic failure
/// into its transport's error vocabulary without re-parsing source text.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum OrgMetadataError {
    #[error("invalid Org metadata: {0}")]
    Invalid(String),
    #[error("Org body contains metadata but no content")]
    MetadataOnly,
}

/// Normalize source and resolve its recognized leading metadata block.
///
/// `request_clock` is injected exactly once by the orchestration layer. Audience
/// ownership and comparisons against final stored values are intentionally not
/// dependencies of this pure function.
///
/// # Errors
///
/// Returns [`OrgMetadataError`] when recognized metadata is malformed,
/// contradictory, invalid for the operation, or leaves no Post body.
pub fn normalize_org(
    source: &str,
    structured: OrgStructuredMetadata,
    operation: OrgOperation,
    request_clock: UtcInstant,
) -> Result<OrgNormalization, OrgMetadataError> {
    validate_lifecycle(&structured.lifecycle, request_clock)?;
    let document = Org::parse(source);
    let parsed = parse_leading_block(source, &document, request_clock)?;
    validate_operation(&parsed.bookkeeping, operation)?;

    let (body, heading_title) =
        canonical_body(&parsed.body, matches!(&parsed.title, Presence::Present(_)))?;
    let title = match choose(structured.title, parsed.title) {
        Presence::Absent => heading_title.map_or(Presence::Absent, Presence::Present),
        title @ Presence::Present(_) => title,
    };
    let summary = choose(structured.summary, parsed.summary);
    let tags = choose(structured.tags, parsed.tags);
    let audiences = choose(structured.audiences, parsed.audiences);
    validate_audiences(&audiences)?;
    let lifecycle = choose(structured.lifecycle, parsed.lifecycle);

    Ok(OrgNormalization {
        body,
        metadata: OrgEffectiveMetadata {
            title,
            summary,
            tags,
            audiences,
            lifecycle,
        },
        bookkeeping: parsed.bookkeeping,
    })
}

#[derive(Default)]
struct ParsedBlock {
    body: String,
    title: Presence<PostTitle>,
    summary: Presence<PostSummary>,
    tags: Presence<Vec<TagLabel>>,
    audiences: Presence<Vec<AudienceTarget>>,
    lifecycle: Presence<PublicationState>,
    bookkeeping: OrgBookkeeping,
}
#[derive(Default)]
struct HeaderFields {
    titles: Vec<String>,
    summaries: Vec<String>,
    tags: Vec<TagLabel>,
    audiences: Vec<AudienceTarget>,
    date: Option<String>,
    timezone: Option<String>,
    status: Option<String>,
    bookkeeping: OrgBookkeeping,
}

enum HeaderKeyword<'a> {
    Title(&'a str),
    Description(&'a str),
    Keywords(&'a str),
    Date(&'a str),
    Property(&'a str),
}

#[derive(Clone, Copy)]
enum PropertyName {
    DateTimezone,
    Status,
    Audience,
    Slug,
    Format,
    Id,
    Synced,
    SyncedAt,
    DateUtc,
}

enum PropertyLine<'a> {
    Unknown,
    Recognized(PropertyName, &'a str),
}

fn parse_leading_block(
    source: &str,
    document: &Org,
    request_clock: UtcInstant,
) -> Result<ParsedBlock, OrgMetadataError> {
    let header_end = leading_keyword_end(document);
    let mut body = Vec::new();
    let mut offset = 0;
    let mut fields = HeaderFields::default();

    for source_line in source.split_inclusive('\n') {
        let line = source_line.strip_suffix('\n').unwrap_or(source_line);
        let handled = offset < header_end && parse_header_keyword(line, &mut fields)?;
        if !handled {
            body.push(line);
        }
        offset += source_line.len();
    }

    let mut parsed = ParsedBlock {
        body: body.join("\n"),
        bookkeeping: fields.bookkeeping,
        ..ParsedBlock::default()
    };
    if !fields.titles.is_empty() {
        parsed.title = Presence::Present(
            fields
                .titles
                .join("\n")
                .parse()
                .map_err(|_| OrgMetadataError::Invalid("invalid TITLE".into()))?,
        );
    }
    if !fields.summaries.is_empty() {
        parsed.summary = Presence::Present(
            fields
                .summaries
                .join("\n")
                .parse()
                .map_err(|_| OrgMetadataError::Invalid("invalid DESCRIPTION".into()))?,
        );
    }
    if !fields.tags.is_empty() {
        parsed.tags = Presence::Present(
            tag::parse_and_validate_tags(fields.tags)
                .map_err(|_| OrgMetadataError::Invalid("invalid KEYWORDS".into()))?,
        );
    }
    if !fields.audiences.is_empty() {
        parsed.audiences = Presence::Present(fields.audiences);
    }
    parsed.lifecycle = parse_lifecycle(fields.status, fields.date, fields.timezone, request_clock)?;
    Ok(parsed)
}

fn parse_header_keyword(line: &str, fields: &mut HeaderFields) -> Result<bool, OrgMetadataError> {
    let Some(keyword) = header_keyword(line) else {
        return Ok(false);
    };
    match keyword {
        HeaderKeyword::Title(value) => fields.titles.push(nonblank(value, "TITLE")?.to_owned()),
        HeaderKeyword::Description(value) => fields
            .summaries
            .push(nonblank(value, "DESCRIPTION")?.to_owned()),
        HeaderKeyword::Keywords(value) => fields.tags.extend(parse_keywords(value)?),
        HeaderKeyword::Date(value) => set_once(
            &mut fields.date,
            nonblank(value, "DATE")?.to_owned(),
            "DATE",
        )?,
        HeaderKeyword::Property(value) => match property_line(value)? {
            PropertyLine::Unknown => return Ok(false),
            PropertyLine::Recognized(name, value) => {
                parse_property(name, value, fields)?;
            }
        },
    }
    Ok(true)
}

fn header_keyword(line: &str) -> Option<HeaderKeyword<'_>> {
    let (name, value) = keyword(line)?;
    match name.as_str() {
        "title" => Some(HeaderKeyword::Title(value)),
        "description" => Some(HeaderKeyword::Description(value)),
        "keywords" => Some(HeaderKeyword::Keywords(value)),
        "date" => Some(HeaderKeyword::Date(value)),
        "property" => Some(HeaderKeyword::Property(value)),
        _ => None,
    }
}

fn parse_keywords(value: &str) -> Result<Vec<TagLabel>, OrgMetadataError> {
    let terms: Vec<_> = value
        .split(',')
        .map(str::trim)
        .filter(|term| !term.is_empty())
        .collect();
    if terms.is_empty() {
        return invalid("KEYWORDS must contain a tag");
    }
    terms
        .into_iter()
        .map(|term| {
            term.parse()
                .map_err(|_| OrgMetadataError::Invalid("invalid KEYWORDS tag".into()))
        })
        .collect()
}

fn leading_keyword_end(document: &Org) -> usize {
    let Some(section) = document
        .first_node::<Document>()
        .and_then(|doc| doc.section())
    else {
        return 0;
    };

    section
        .syntax()
        .children()
        .take_while(|node| Keyword::cast(node.clone()).is_some())
        .filter_map(Keyword::cast)
        .last()
        .map_or(0, |keyword| {
            u32::from(keyword.syntax().text_range().end()) as usize
        })
}

fn keyword(line: &str) -> Option<(String, &str)> {
    let (key, value) = line.trim_start().split_once(':')?;
    let key = key.strip_prefix("#+")?;
    Some((key.to_ascii_lowercase(), value.trim()))
}

fn property_line(value: &str) -> Result<PropertyLine<'_>, OrgMetadataError> {
    let Some((name, value)) = value.split_once(char::is_whitespace) else {
        return if property_name(value).is_some() {
            invalid("PROPERTY must name a value")
        } else {
            Ok(PropertyLine::Unknown)
        };
    };
    let Some(name) = property_name(name) else {
        return Ok(PropertyLine::Unknown);
    };
    Ok(PropertyLine::Recognized(
        name,
        nonblank(value.trim(), "PROPERTY")?,
    ))
}

fn property_name(name: &str) -> Option<PropertyName> {
    match name.to_ascii_lowercase().as_str() {
        "jaunder_date_tz" => Some(PropertyName::DateTimezone),
        "jaunder_status" => Some(PropertyName::Status),
        "jaunder_audience" => Some(PropertyName::Audience),
        "jaunder_slug" => Some(PropertyName::Slug),
        "jaunder_format" => Some(PropertyName::Format),
        "jaunder_id" => Some(PropertyName::Id),
        "jaunder_synced" => Some(PropertyName::Synced),
        "jaunder_synced_at" => Some(PropertyName::SyncedAt),
        "jaunder_date_utc" => Some(PropertyName::DateUtc),
        _ => None,
    }
}

fn nonblank<'a>(value: &'a str, field: &str) -> Result<&'a str, OrgMetadataError> {
    if value.is_empty() {
        invalid(&format!("{field} must not be blank"))
    } else {
        Ok(value)
    }
}
fn invalid<T>(message: &str) -> Result<T, OrgMetadataError> {
    Err(OrgMetadataError::Invalid(message.into()))
}
fn set_once(slot: &mut Option<String>, value: String, name: &str) -> Result<(), OrgMetadataError> {
    if slot.replace(value).is_some() {
        invalid(&format!("duplicate {name}"))
    } else {
        Ok(())
    }
}

fn parse_property(
    name: PropertyName,
    value: &str,
    fields: &mut HeaderFields,
) -> Result<(), OrgMetadataError> {
    match name {
        PropertyName::DateTimezone => {
            set_once(&mut fields.timezone, value.to_owned(), "JAUNDER_DATE_TZ")
        }
        PropertyName::Status => set_once(&mut fields.status, value.to_owned(), "JAUNDER_STATUS"),
        PropertyName::Audience => {
            fields.audiences.push(parse_audience(value)?);
            Ok(())
        }
        PropertyName::Slug => set_bookkeeping(
            &mut fields.bookkeeping.slug,
            value
                .parse()
                .map_err(|_| OrgMetadataError::Invalid("invalid Jaunder slug".into()))?,
            "JAUNDER_SLUG",
        ),
        PropertyName::Format => set_bookkeeping(
            &mut fields.bookkeeping.format,
            value
                .parse()
                .map_err(|_| OrgMetadataError::Invalid("invalid Jaunder format".into()))?,
            "JAUNDER_FORMAT",
        ),
        PropertyName::Id => set_bookkeeping(
            &mut fields.bookkeeping.post_id,
            value
                .parse()
                .map_err(|_| OrgMetadataError::Invalid("invalid Jaunder ID".into()))?,
            "JAUNDER_ID",
        ),
        PropertyName::Synced => set_bookkeeping(
            &mut fields.bookkeeping.synced,
            ETag::from_str(value)
                .map_err(|_| OrgMetadataError::Invalid("invalid Jaunder sync ETag".into()))?,
            "JAUNDER_SYNCED",
        ),
        PropertyName::SyncedAt => set_bookkeeping(
            &mut fields.bookkeeping.synced_at,
            value
                .parse()
                .map_err(|_| OrgMetadataError::Invalid("invalid Jaunder sync time".into()))?,
            "JAUNDER_SYNCED_AT",
        ),
        PropertyName::DateUtc => set_bookkeeping(
            &mut fields.bookkeeping.date_utc,
            value.parse().map_err(|_| {
                OrgMetadataError::Invalid("invalid Jaunder publication time".into())
            })?,
            "JAUNDER_DATE_UTC",
        ),
    }
}
fn set_bookkeeping<T>(slot: &mut Option<T>, value: T, name: &str) -> Result<(), OrgMetadataError> {
    if slot.replace(value).is_some() {
        invalid(&format!("duplicate {name}"))
    } else {
        Ok(())
    }
}

fn parse_audience(value: &str) -> Result<AudienceTarget, OrgMetadataError> {
    match value {
        "public" => Ok(AudienceTarget::Public),
        "subscribers" => Ok(AudienceTarget::Subscribers),
        "private" => Ok(AudienceTarget::Private),
        _ => value
            .strip_prefix("named:")
            .and_then(|id| id.parse::<AudienceId>().ok())
            .map(AudienceTarget::Named)
            .ok_or_else(|| OrgMetadataError::Invalid("invalid JAUNDER_AUDIENCE".into())),
    }
}

fn parse_lifecycle(
    status: Option<String>,
    date: Option<String>,
    timezone: Option<String>,
    clock: UtcInstant,
) -> Result<Presence<PublicationState>, OrgMetadataError> {
    let Some(status) = status else {
        if date.is_some() || timezone.is_some() {
            return invalid("DATE and JAUNDER_DATE_TZ require JAUNDER_STATUS");
        }
        return Ok(Presence::Absent);
    };
    let instant = match (date, timezone) {
        (None, None) => None,
        (Some(date), Some(tz)) => Some(parse_org_date(&date, &tz)?),
        _ => return invalid("DATE and JAUNDER_DATE_TZ must occur together"),
    };
    let state = match (status.to_ascii_lowercase().as_str(), instant) {
        ("draft", None) => PublicationState::Draft,
        ("scheduled", Some(at)) if at.value() > clock.value() => PublicationState::Scheduled(at),
        ("published", at) => PublicationState::Published(at.unwrap_or(clock)),
        _ => return invalid("invalid JAUNDER_STATUS lifecycle"),
    };
    let lifecycle = Presence::Present(state);
    validate_lifecycle(&lifecycle, clock)?;
    Ok(lifecycle)
}

fn parse_org_date(value: &str, timezone: &str) -> Result<UtcInstant, OrgMetadataError> {
    let Some(value) = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) else {
        return invalid("DATE must be an inactive Org timestamp");
    };
    let parts: Vec<_> = value.split_whitespace().collect();
    if parts.len() != 3 {
        return invalid("invalid DATE");
    }
    let date = NaiveDate::parse_from_str(parts[0], "%Y-%m-%d")
        .map_err(|_| OrgMetadataError::Invalid("invalid DATE".into()))?;
    if date.format("%a").to_string() != parts[1] {
        return invalid("DATE weekday does not match date");
    }
    let time =
        NaiveDateTime::parse_from_str(&format!("{} {}", parts[0], parts[2]), "%Y-%m-%d %H:%M")
            .map_err(|_| OrgMetadataError::Invalid("invalid DATE".into()))?;
    let tz: Tz = timezone
        .parse()
        .map_err(|_| OrgMetadataError::Invalid("invalid Jaunder timezone".into()))?;
    match tz.from_local_datetime(&time) {
        LocalResult::Single(at) => Ok(at.with_timezone(&chrono::Utc).into()),
        LocalResult::Ambiguous(earlier, _) => Ok(earlier.with_timezone(&chrono::Utc).into()),
        LocalResult::None => invalid("DATE is in a DST gap"),
    }
}

fn choose<T>(structured: Presence<T>, header: Presence<T>) -> Presence<T> {
    match structured {
        Presence::Present(value) => Presence::Present(value),
        Presence::Absent => header,
    }
}
fn validate_lifecycle(
    lifecycle: &Presence<PublicationState>,
    clock: UtcInstant,
) -> Result<(), OrgMetadataError> {
    match lifecycle {
        Presence::Present(PublicationState::Published(at)) if at.value() > clock.value() => {
            invalid("published instant must not be future")
        }
        _ => Ok(()),
    }
}
fn validate_audiences(audiences: &Presence<Vec<AudienceTarget>>) -> Result<(), OrgMetadataError> {
    let Presence::Present(audiences) = audiences else {
        return Ok(());
    };
    if audiences
        .iter()
        .any(|audience| matches!(audience, AudienceTarget::Private))
        && audiences.len() != 1
    {
        return invalid("private audience cannot be combined");
    }
    Ok(())
}

fn canonical_body(
    body: &str,
    header_supplies_title: bool,
) -> Result<(PostBody, Option<PostTitle>), OrgMetadataError> {
    // The canonicalizer's title-header state decides whether a following level-one
    // heading is content. Retain that state while the parser removes recognized
    // metadata by presenting an otherwise-discarded marker to the one canonical
    // format-aware body door.
    let source = if header_supplies_title {
        format!("#+TITLE:\n{body}")
    } else {
        body.to_owned()
    };
    let body: PostBody = source.parse().map_err(|_| OrgMetadataError::MetadataOnly)?;
    let heading_title = render::derive_post_naming(None, &body, &PostFormat::Org).0;
    render::canonicalize_body(&body, &PostFormat::Org)
        .map(|body| (body, heading_title))
        .map_err(|_| OrgMetadataError::MetadataOnly)
}
fn validate_operation(
    bookkeeping: &OrgBookkeeping,
    operation: OrgOperation,
) -> Result<(), OrgMetadataError> {
    match operation {
        OrgOperation::Create
            if bookkeeping.post_id.is_some()
                || bookkeeping.synced.is_some()
                || bookkeeping.synced_at.is_some() =>
        {
            invalid("create cannot include ID or sync bookkeeping")
        }
        OrgOperation::Update { post_id } if bookkeeping.post_id.is_some_and(|id| id != post_id) => {
            invalid("JAUNDER_ID does not match update target")
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock() -> UtcInstant {
        "2026-08-26T12:00:00Z".parse().expect("valid fixed clock")
    }

    #[test]
    fn publication_state_projects_published_at() {
        let at: UtcInstant = "2026-11-01T05:30:00Z".parse().expect("valid instant");

        for (state, expected) in [
            (PublicationState::Draft, None),
            (PublicationState::Scheduled(at), Some(at)),
            (PublicationState::Published(at), Some(at)),
        ] {
            assert_eq!(state.published_at(), expected);
        }
    }

    fn normalize(source: &str) -> OrgNormalization {
        normalize_org(
            source,
            OrgStructuredMetadata::default(),
            OrgOperation::Create,
            clock(),
        )
        .expect("valid Org metadata")
    }

    fn invalid(source: &str) {
        assert!(matches!(
            normalize_org(
                source,
                OrgStructuredMetadata::default(),
                OrgOperation::Create,
                clock(),
            ),
            Err(OrgMetadataError::Invalid(_))
        ));
    }

    #[test]
    fn honors_the_ast_header_boundary_and_preserves_unknown_whitespace() {
        let normalized = normalize(
            "\n#+tItLe: A title\n\n#+AUTHOR: Author\n    indented content\n#+KEYWORDS: later\n",
        );

        assert_eq!(
            normalized.metadata.title,
            Presence::Present("A title".parse().unwrap())
        );
        assert_eq!(
            normalized.body.to_string(),
            "#+AUTHOR: Author\n    indented content\n#+KEYWORDS: later\n"
        );
        assert_eq!(normalized.metadata.tags, Presence::Absent);
    }

    #[test]
    fn preserves_unrecognized_header_keywords_and_properties() {
        let normalized = normalize(
            "#+AUTHOR: Author\n#+PROPERTY: EXPORT_FILE_NAME example\n#+PROPERTY: EXPORT_FILE_NAME\n#+NOT_A_KEYWORD: retained\nBody",
        );

        assert_eq!(
            normalized.body.to_string(),
            "#+AUTHOR: Author\n#+PROPERTY: EXPORT_FILE_NAME example\n#+PROPERTY: EXPORT_FILE_NAME\n#+NOT_A_KEYWORD: retained\nBody\n"
        );
    }

    #[test]
    fn composes_repeated_text_and_keywords_with_tag_identity_order_and_cap() {
        let normalized = normalize(
            "#+TITLE: First\n#+TITLE: Second\n#+DESCRIPTION: One\n#+DESCRIPTION: Two\n#+KEYWORDS: Rust, , Emacs\n#+KEYWORDS: rust, Lisp\nBody",
        );

        assert_eq!(
            normalized.metadata.title,
            Presence::Present("First\nSecond".parse().unwrap())
        );
        assert_eq!(
            normalized.metadata.summary,
            Presence::Present("One\nTwo".parse().unwrap())
        );
        assert_eq!(
            normalized.metadata.tags,
            Presence::Present(vec![
                "Rust".parse().unwrap(),
                "Emacs".parse().unwrap(),
                "Lisp".parse().unwrap()
            ])
        );
        invalid("#+KEYWORDS: , ,\nBody");
        invalid("#+KEYWORDS: rust/lang\nBody");
    }

    #[test]
    fn structured_presence_wins_without_an_explicit_clear_state() {
        let normalized = normalize_org(
            "#+TITLE: Header\n#+DESCRIPTION: Header summary\n#+KEYWORDS: rust\nBody",
            OrgStructuredMetadata {
                title: Presence::Present("Structured".parse().unwrap()),
                summary: Presence::Present("Structured summary".parse().unwrap()),
                tags: Presence::Present(vec![]),
                ..OrgStructuredMetadata::default()
            },
            OrgOperation::Create,
            clock(),
        )
        .expect("explicit structured metadata wins");

        assert_eq!(
            normalized.metadata.title,
            Presence::Present("Structured".parse().unwrap())
        );
        assert_eq!(
            normalized.metadata.summary,
            Presence::Present("Structured summary".parse().unwrap())
        );
        assert_eq!(normalized.metadata.tags, Presence::Present(vec![]));
        assert_eq!(normalized.body.to_string(), "Body\n");
    }

    #[test]
    fn preserves_heading_title_behavior_and_strips_once() {
        let normalized = normalize("* Heading\n\n    content\n");
        assert_eq!(
            normalized.metadata.title,
            Presence::Present("Heading".parse().unwrap())
        );
        assert_eq!(normalized.body.to_string(), "    content\n");

        let header_wins = normalize("#+TITLE: Header\n* Heading\nBody");
        assert_eq!(
            header_wins.metadata.title,
            Presence::Present("Header".parse().unwrap())
        );
        assert_eq!(header_wins.body.to_string(), "* Heading\nBody\n");

        let normalized_twice = normalize(normalized.body.as_ref());
        assert_eq!(normalized_twice.body, normalized.body);
        assert_eq!(normalized_twice.metadata.title, Presence::Absent);
    }

    #[test]
    fn validates_lifecycle_combinations_clock_and_civil_time() {
        assert_eq!(
            normalize("#+PROPERTY: jaunder_status draft\nBody")
                .metadata
                .lifecycle,
            Presence::Present(PublicationState::Draft)
        );
        assert_eq!(
            normalize(
                "#+DATE: [2026-08-26 Wed 12:00]\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n#+PROPERTY: JAUNDER_STATUS published\nBody"
            )
            .metadata
            .lifecycle,
            Presence::Present(PublicationState::Published(clock()))
        );
        assert_eq!(
            normalize(
                "#+DATE: [2026-11-01 Sun 01:30]\n#+PROPERTY: JAUNDER_DATE_TZ America/New_York\n#+PROPERTY: JAUNDER_STATUS scheduled\nBody"
            )
            .metadata
            .lifecycle,
            Presence::Present(PublicationState::Scheduled(
                "2026-11-01T05:30:00Z".parse().unwrap()
            ))
        );
        invalid("#+PROPERTY: JAUNDER_STATUS scheduled\nBody");
        invalid(
            "#+DATE: [2026-08-26 Tue 12:00]\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n#+PROPERTY: JAUNDER_STATUS published\nBody",
        );
        invalid(
            "#+DATE: [2026-03-08 Sun 02:30]\n#+PROPERTY: JAUNDER_DATE_TZ America/New_York\n#+PROPERTY: JAUNDER_STATUS published\nBody",
        );
        invalid("#+DATE: [2026-08-26 Wed 12:00]\nBody");
        invalid("#+DATE: [2026-08-26 Wed 12:00]\n#+PROPERTY: JAUNDER_STATUS published\nBody");
        invalid("#+PROPERTY: JAUNDER_DATE_TZ UTC\n#+PROPERTY: JAUNDER_STATUS published\nBody");
        invalid(
            "#+DATE: <2026-08-26 Wed 12:00>\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n#+PROPERTY: JAUNDER_STATUS published\nBody",
        );
        invalid(
            "#+DATE: [2026-08-26 Wed]\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n#+PROPERTY: JAUNDER_STATUS published\nBody",
        );
    }

    #[test]
    fn validates_displaced_header_lifecycle_before_precedence() {
        let future_published = "#+DATE: [2026-08-27 Thu 12:00]\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n#+PROPERTY: JAUNDER_STATUS published\nBody";

        for lifecycle in [
            PublicationState::Draft,
            PublicationState::Published(clock()),
        ] {
            assert!(matches!(
                normalize_org(
                    future_published,
                    OrgStructuredMetadata {
                        lifecycle: Presence::Present(lifecycle),
                        ..OrgStructuredMetadata::default()
                    },
                    OrgOperation::Create,
                    clock(),
                ),
                Err(OrgMetadataError::Invalid(_))
            ));
        }

        let normalized = normalize_org(
            "#+DATE: [2026-08-25 Tue 12:00]\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n#+PROPERTY: JAUNDER_STATUS published\nBody",
            OrgStructuredMetadata {
                lifecycle: Presence::Present(PublicationState::Draft),
                ..OrgStructuredMetadata::default()
            },
            OrgOperation::Create,
            clock(),
        )
        .expect("a valid displaced header lifecycle is accepted");
        assert_eq!(
            normalized.metadata.lifecycle,
            Presence::Present(PublicationState::Draft)
        );
    }

    #[test]
    fn validates_audience_and_singleton_metadata() {
        assert_eq!(
            normalize(
                "#+PROPERTY: jaunder_audience named:42\n#+PROPERTY: JAUNDER_AUDIENCE subscribers\nBody"
            )
            .metadata
            .audiences,
            Presence::Present(vec![
                AudienceTarget::Named(AudienceId::from(42)),
                AudienceTarget::Subscribers
            ])
        );
        invalid("#+PROPERTY: JAUNDER_AUDIENCE private\n#+PROPERTY: JAUNDER_AUDIENCE public\nBody");
        invalid("#+PROPERTY: JAUNDER_AUDIENCE named:invalid\nBody");
        invalid("#+PROPERTY: JAUNDER_STATUS draft\n#+PROPERTY: Jaunder_Status draft\nBody");
        invalid("#+DATE: [2026-08-26 Wed 12:00]\n#+DATE: [2026-08-26 Wed 12:00]\nBody");
        invalid("#+PROPERTY: JAUNDER_STATUS\nBody");
    }

    #[test]
    fn validates_bookkeeping_grammar_duplicates_and_operation_identity() {
        let normalized = normalize(
            "#+PROPERTY: JAUNDER_FORMAT org\n#+PROPERTY: JAUNDER_SLUG example\n#+PROPERTY: JAUNDER_DATE_UTC 2026-08-26T12:00:00+00:00\nBody",
        );
        assert_eq!(normalized.bookkeeping.format, Some(PostFormat::Org));
        assert_eq!(
            normalized.bookkeeping.date_utc,
            Some("2026-08-26T12:00:00Z".parse().unwrap())
        );
        invalid("#+PROPERTY: JAUNDER_FORMAT org\n#+PROPERTY: JAUNDER_FORMAT org\nBody");
        invalid("#+PROPERTY: JAUNDER_SYNCED weak\nBody");
        invalid("#+PROPERTY: JAUNDER_ID 7\nBody");
        invalid("#+PROPERTY: JAUNDER_DATE_UTC not-a-time\nBody");
        assert!(matches!(
            normalize_org(
                "#+PROPERTY: JAUNDER_ID 7\nBody",
                OrgStructuredMetadata::default(),
                OrgOperation::Update {
                    post_id: PostId::from(8)
                },
                clock(),
            ),
            Err(OrgMetadataError::Invalid(_))
        ));
    }

    #[test]
    fn rejects_blank_text_and_more_than_the_tag_cap() {
        invalid("#+TITLE:   \nBody");
        invalid("#+DESCRIPTION:   \nBody");

        let tags = (0..=crate::tag::MAX_TAGS_PER_POST)
            .map(|index| format!("tag{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        invalid(&format!("#+KEYWORDS: {tags}\nBody"));
    }

    #[test]
    fn rejects_every_invalid_lifecycle_presence_combination() {
        assert_eq!(
            normalize("#+PROPERTY: JAUNDER_STATUS published\nBody")
                .metadata
                .lifecycle,
            Presence::Present(PublicationState::Published(clock()))
        );
        invalid(
            "#+DATE: [2026-08-27 Thu 12:00]\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n#+PROPERTY: JAUNDER_STATUS draft\nBody",
        );
        invalid(
            "#+DATE: [2026-08-27 Thu 12:00]\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n#+PROPERTY: JAUNDER_STATUS published\nBody",
        );
        invalid(
            "#+DATE: [2026-08-26 Wed 12:00]\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n#+PROPERTY: JAUNDER_STATUS scheduled\nBody",
        );
        invalid("#+PROPERTY: JAUNDER_DATE_TZ UTC\nBody");
    }

    #[test]
    fn parses_sync_bookkeeping_without_changing_instant_identity() {
        let normalized = normalize_org(
            "#+PROPERTY: JAUNDER_SYNCED \"sha256-abc\"\n#+PROPERTY: JAUNDER_SYNCED_AT 2026-08-26T08:00:00-04:00\nBody",
            OrgStructuredMetadata::default(),
            OrgOperation::Update {
                post_id: PostId::from(7),
            },
            clock(),
        )
        .expect("valid update sync bookkeeping");
        assert_eq!(
            normalized.bookkeeping.synced,
            Some("\"sha256-abc\"".parse().unwrap())
        );
        assert_eq!(normalized.bookkeeping.synced_at, Some(clock()));
        assert!(matches!(
            normalize_org(
                "#+PROPERTY: JAUNDER_SYNCED_AT 2026-08-26T12:00:00Z\n#+PROPERTY: JAUNDER_SYNCED_AT 2026-08-26T12:00:00Z\nBody",
                OrgStructuredMetadata::default(),
                OrgOperation::Update {
                    post_id: PostId::from(7),
                },
                clock(),
            ),
            Err(OrgMetadataError::Invalid(_))
        ));

        let matching = normalize_org(
            "#+PROPERTY: JAUNDER_ID 7\nBody",
            OrgStructuredMetadata::default(),
            OrgOperation::Update {
                post_id: PostId::from(7),
            },
            clock(),
        )
        .expect("matching update identity is valid");
        assert_eq!(matching.bookkeeping.post_id, Some(PostId::from(7)));
    }

    #[test]
    fn rejects_metadata_only_source() {
        assert_eq!(
            normalize_org(
                "#+TITLE: Only metadata",
                OrgStructuredMetadata::default(),
                OrgOperation::Create,
                clock(),
            ),
            Err(OrgMetadataError::MetadataOnly)
        );
    }
}
