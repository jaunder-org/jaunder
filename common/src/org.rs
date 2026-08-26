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
    render::{PostFormat, canonicalize_body, derive_post_naming},
    slug::Slug,
    tag::{TagLabel, parse_and_validate_tags},
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
    // rendered-html-from-trusted:allow generic presence holds caller-validated values and never mints or decodes HTML (#77)
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
    let lifecycle = choose_lifecycle(structured.lifecycle, parsed.lifecycle, request_clock)?;

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
fn parse_leading_block(
    source: &str,
    document: &Org,
    request_clock: UtcInstant,
) -> Result<ParsedBlock, OrgMetadataError> {
    let mut parsed = ParsedBlock::default();
    let header_end = leading_keyword_end(document);
    let mut body = Vec::new();
    let mut offset = 0;
    let mut titles = Vec::new();
    let mut summaries = Vec::new();
    let mut tags = Vec::new();
    let mut audiences = Vec::new();
    let mut date = None;
    let mut timezone = None;
    let mut status = None;

    for source_line in source.split_inclusive('\n') {
        let line = source_line.strip_suffix('\n').unwrap_or(source_line);
        if offset < header_end && line.trim_start().starts_with("#+") {
            match keyword(line) {
                Some((name, value)) if name == "property" && !recognized_property(value) => {
                    body.push(line);
                }
                Some((name, value)) if recognized(&name) => match name.as_str() {
                    "title" => titles.push(nonblank(value, "TITLE")?.to_owned()),
                    "description" => summaries.push(nonblank(value, "DESCRIPTION")?.to_owned()),
                    "keywords" => {
                        let terms: Vec<_> = value
                            .split(',')
                            .map(str::trim)
                            .filter(|term| !term.is_empty())
                            .collect();
                        if terms.is_empty() {
                            return invalid("KEYWORDS must contain a tag");
                        }
                        for term in terms {
                            tags.push(term.parse().map_err(|_| {
                                OrgMetadataError::Invalid("invalid KEYWORDS tag".into())
                            })?);
                        }
                    }
                    "date" => set_once(&mut date, nonblank(value, "DATE")?.to_owned(), "DATE")?,
                    "property" => parse_property(
                        value,
                        &mut timezone,
                        &mut status,
                        &mut audiences,
                        &mut parsed.bookkeeping,
                    )?,
                    _ => unreachable!(),
                },
                _ => body.push(line),
            }
        } else {
            body.push(line);
        }
        offset += source_line.len();
    }

    if !titles.is_empty() {
        parsed.title = Presence::Present(
            titles
                .join("\n")
                .parse()
                .map_err(|_| OrgMetadataError::Invalid("invalid TITLE".into()))?,
        );
    }
    if !summaries.is_empty() {
        parsed.summary = Presence::Present(
            summaries
                .join("\n")
                .parse()
                .map_err(|_| OrgMetadataError::Invalid("invalid DESCRIPTION".into()))?,
        );
    }
    if !tags.is_empty() {
        parsed.tags = Presence::Present(
            parse_and_validate_tags(tags)
                .map_err(|_| OrgMetadataError::Invalid("invalid KEYWORDS".into()))?,
        );
    }
    if !audiences.is_empty() {
        parsed.audiences = Presence::Present(audiences);
    }
    parsed.lifecycle = parse_lifecycle(status, date, timezone, request_clock)?;
    parsed.body = body.join("\n");
    Ok(parsed)
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

fn recognized(name: &str) -> bool {
    matches!(
        name,
        "title" | "description" | "keywords" | "date" | "property"
    )
}

fn recognized_property(value: &str) -> bool {
    value.split_whitespace().next().is_some_and(|name| {
        matches!(
            name.to_ascii_lowercase().as_str(),
            "jaunder_date_tz"
                | "jaunder_status"
                | "jaunder_audience"
                | "jaunder_slug"
                | "jaunder_format"
                | "jaunder_id"
                | "jaunder_synced"
                | "jaunder_synced_at"
                | "jaunder_date_utc"
        )
    })
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
    value: &str,
    timezone: &mut Option<String>,
    status: &mut Option<String>,
    audiences: &mut Vec<AudienceTarget>,
    bookkeeping: &mut OrgBookkeeping,
) -> Result<(), OrgMetadataError> {
    let Some((name, value)) = value.split_once(char::is_whitespace) else {
        return invalid("PROPERTY must name a value");
    };
    let name = name.to_ascii_lowercase();
    let value = nonblank(value.trim(), "PROPERTY")?;
    match name.as_str() {
        "jaunder_date_tz" => set_once(timezone, value.to_owned(), "JAUNDER_DATE_TZ"),
        "jaunder_status" => set_once(status, value.to_owned(), "JAUNDER_STATUS"),
        "jaunder_audience" => {
            audiences.push(parse_audience(value)?);
            Ok(())
        }
        "jaunder_slug" => set_bookkeeping(
            &mut bookkeeping.slug,
            value
                .parse()
                .map_err(|_| OrgMetadataError::Invalid("invalid Jaunder slug".into()))?,
            "JAUNDER_SLUG",
        ),
        "jaunder_format" => set_bookkeeping(
            &mut bookkeeping.format,
            value
                .parse()
                .map_err(|_| OrgMetadataError::Invalid("invalid Jaunder format".into()))?,
            "JAUNDER_FORMAT",
        ),
        "jaunder_id" => set_bookkeeping(
            &mut bookkeeping.post_id,
            value
                .parse()
                .map_err(|_| OrgMetadataError::Invalid("invalid Jaunder ID".into()))?,
            "JAUNDER_ID",
        ),
        "jaunder_synced" => set_bookkeeping(
            &mut bookkeeping.synced,
            ETag::from_str(value)
                .map_err(|_| OrgMetadataError::Invalid("invalid Jaunder sync ETag".into()))?,
            "JAUNDER_SYNCED",
        ),
        "jaunder_synced_at" => set_bookkeeping(
            &mut bookkeeping.synced_at,
            value
                .parse()
                .map_err(|_| OrgMetadataError::Invalid("invalid Jaunder sync time".into()))?,
            "JAUNDER_SYNCED_AT",
        ),
        "jaunder_date_utc" => set_bookkeeping(
            &mut bookkeeping.date_utc,
            value.parse().map_err(|_| {
                OrgMetadataError::Invalid("invalid Jaunder publication time".into())
            })?,
            "JAUNDER_DATE_UTC",
        ),
        _ => Ok(()),
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
    Ok(Presence::Present(state))
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
fn choose_lifecycle(
    structured: Presence<PublicationState>,
    header: Presence<PublicationState>,
    clock: UtcInstant,
) -> Result<Presence<PublicationState>, OrgMetadataError> {
    let chosen = choose(structured, header);
    match chosen {
        Presence::Present(PublicationState::Published(at)) if at.value() > clock.value() => {
            invalid("published instant must not be future")
        }
        value => Ok(value),
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
    let heading_title = derive_post_naming(None, &body, &PostFormat::Org).0;
    canonicalize_body(&body, &PostFormat::Org)
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

        let tags = (0..26)
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
