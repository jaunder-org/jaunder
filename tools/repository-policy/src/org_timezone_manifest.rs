//! Read-only local-original evidence, deliberately outside checkout/bootstrap lifecycle.
//! Only exact Post IDs join sources; this is not an importer or a coverage claim.
use std::{collections::BTreeMap, fs, path::PathBuf};

use common::{
    ids::PostId,
    org::{OrgTimezoneProperty, org_timezone_properties},
    time::PublicationTimezone,
};
use serde::Serialize;

#[derive(Serialize)]
pub struct Mapping {
    post_id: String,
    timezone: PublicationTimezone,
}

#[derive(Serialize)]
pub struct Diagnostic {
    source: PathBuf,
    code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    post_id: Option<String>,
}

#[derive(Serialize)]
pub struct Manifest {
    pub mappings: Vec<Mapping>,
    pub diagnostics: Vec<Diagnostic>,
    server_coverage: &'static str,
}

/// Inspect explicit files only. Ambiguous IDs are excluded even when their zones
/// agree, or one of their sources has an invalid/missing zone. Valid independent
/// pairs remain useful, but diagnostics make the partial result non-successful.
#[must_use]
pub fn extract(files: &[PathBuf]) -> Manifest {
    let mut report = Manifest {
        mappings: Vec::new(),
        diagnostics: Vec::new(),
        server_coverage: "unknown: no authorized server Post-ID inventory supplied; server IDs absent from originals cannot be determined",
    };
    let mut candidates: BTreeMap<i64, Vec<(PathBuf, Option<PublicationTimezone>)>> =
        BTreeMap::new();
    let mut files = files.to_vec();
    files.sort();
    for source in files {
        let mut codes = Vec::new();
        match fs::read_to_string(&source) {
            Err(_) => codes.push("unreadable_source"),
            Ok(text) => {
                let properties = org_timezone_properties(&text);
                let mut id_value = None;
                let mut duplicate_id_property = false;
                let mut timezone_value = None;
                let mut duplicate_zone_property = false;
                // Retain every valid ID so repeated or cross-file IDs block
                // acceptance, even when another property on this source is invalid.
                let mut ids = Vec::new();
                for property in &properties.properties {
                    match property {
                        OrgTimezoneProperty::PostId(value) => {
                            if id_value.is_some() {
                                duplicate_id_property = true;
                            } else {
                                id_value = Some(value.as_str());
                            }
                            if let Some(id) = decimal_post_id(value) {
                                ids.push(i64::from(id));
                            }
                        }
                        OrgTimezoneProperty::DateTimezone(value) => {
                            if timezone_value.is_some() {
                                duplicate_zone_property = true;
                            } else {
                                timezone_value = Some(value.as_str());
                            }
                        }
                    }
                }
                let id = if duplicate_id_property {
                    codes.push("duplicate_id_property");
                    None
                } else {
                    match id_value {
                        None => {
                            codes.push("missing_id");
                            None
                        }
                        Some(value) => match decimal_post_id(value) {
                            Some(id) => Some(id),
                            None => {
                                codes.push("invalid_id");
                                None
                            }
                        },
                    }
                };
                let timezone = if duplicate_zone_property {
                    codes.push("duplicate_zone_property");
                    None
                } else {
                    match timezone_value {
                        None => {
                            codes.push("missing_zone");
                            None
                        }
                        Some(value) => match value.parse::<PublicationTimezone>() {
                            Ok(zone) => Some(zone),
                            Err(_) => {
                                codes.push("invalid_zone");
                                None
                            }
                        },
                    }
                };
                // Even a source with repeated ID properties blocks all IDs it
                // names from being accepted through a different original.
                ids.sort_unstable();
                ids.dedup();
                for candidate in ids {
                    candidates
                        .entry(candidate)
                        .or_default()
                        .push((source.clone(), id.and(timezone.clone())));
                }
            }
        }
        if !codes.is_empty() {
            codes.push("unmapped_source");
        }
        for code in codes {
            report.diagnostics.push(Diagnostic {
                source: source.clone(),
                code,
                post_id: None,
            });
        }
    }
    for (id, sources) in candidates {
        if sources.len() > 1 {
            for (source, _) in sources {
                report.diagnostics.push(Diagnostic {
                    source: source.clone(),
                    code: "duplicate_id",
                    post_id: Some(id.to_string()),
                });
                if !report
                    .diagnostics
                    .iter()
                    .any(|d| d.source == source && d.code == "unmapped_source")
                {
                    report.diagnostics.push(Diagnostic {
                        source,
                        code: "unmapped_source",
                        post_id: None,
                    });
                }
            }
        } else if let Some((_, Some(timezone))) = sources.into_iter().next() {
            report.mappings.push(Mapping {
                post_id: id.to_string(),
                timezone,
            });
        }
    }
    report
        .diagnostics
        .sort_by(|a, b| (&a.source, a.code, &a.post_id).cmp(&(&b.source, b.code, &b.post_id)));
    report
        .diagnostics
        .dedup_by(|a, b| a.source == b.source && a.code == b.code && a.post_id == b.post_id);
    report
}

fn decimal_post_id(value: &str) -> Option<PostId> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let id = value.parse::<i64>().ok()?;
    (id > 0).then(|| PostId::from(id))
}
