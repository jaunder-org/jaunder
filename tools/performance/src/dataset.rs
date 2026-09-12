use crate::model::{
    CursorRequirement, DatasetManifest, DatasetPlan, DatasetProfile, Distribution, GENERATOR_SEED,
    GENERATOR_VERSION, GeneratorIdentity, Workload,
};
use thiserror::Error;

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ManifestError {
    #[error("manifest schema is unsupported")]
    Schema,
    #[error("manifest plan is not canonical")]
    Plan,
    #[error("cursor does not match its planned result set")]
    Cursor,
}

#[must_use]
pub fn canonical_plan(profile: DatasetProfile) -> DatasetPlan {
    let (posts, authors) = match profile {
        DatasetProfile::Small => (100, 10),
        DatasetProfile::Medium => (5_000, 100),
        DatasetProfile::Large => (50_000, 1_000),
    };
    let mut lifecycle = allocate(
        posts,
        &[
            ("live", 60),
            ("draft", 15),
            ("scheduled", 15),
            ("deleted", 10),
        ],
    );
    lifecycle.push(Distribution {
        bucket: "live_backdated".into(),
        count: lifecycle[0].count / 3,
    });
    DatasetPlan {
        profile,
        generator: GeneratorIdentity {
            version: GENERATOR_VERSION,
            seed: GENERATOR_SEED,
        },
        posts,
        authors,
        revisions: posts * 5,
        lifecycle,
        revision_distribution: allocate(posts, &[("one", 70), ("five", 20), ("thirty_three", 10)]),
        tag_distribution: allocate(posts, &[("none", 25), ("two", 50), ("eight", 25)]),
        audience_distribution: allocate(
            posts,
            &[
                ("public_only", 50),
                ("one_private", 25),
                ("five_private", 25),
            ],
        ),
        media_distribution: allocate(posts, &[("none", 75), ("one", 20), ("five", 5)]),
        body_distribution: evenly(
            posts,
            &[
                "markdown_256",
                "markdown_4096",
                "markdown_65536",
                "html_256",
                "html_4096",
                "html_65536",
                "plain_text_256",
                "plain_text_4096",
                "plain_text_65536",
            ],
        ),
        follows_per_author: (authors - 1).min(20),
        cursor_requirements: vec![
            CursorRequirement {
                workload: Workload::PublicTimeline,
            },
            CursorRequirement {
                workload: Workload::AuthenticatedTimeline,
            },
            CursorRequirement {
                workload: Workload::OwnerHistory,
            },
            CursorRequirement {
                workload: Workload::PostHistory,
            },
        ],
    }
}

/// Validates that a dataset manifest matches the canonical profile and cursor contract.
///
/// # Errors
///
/// Returns [`ManifestError`] when the schema, canonical plan, workload subjects,
/// or resolved cursors are invalid.
pub fn validate_manifest(manifest: &DatasetManifest) -> Result<(), ManifestError> {
    if manifest.schema_version != crate::DATASET_SCHEMA_VERSION {
        return Err(ManifestError::Schema);
    }
    if manifest.plan != canonical_plan(manifest.plan.profile) {
        return Err(ManifestError::Plan);
    }
    if manifest.cursors.len() != manifest.plan.cursor_requirements.len()
        || manifest.subjects.username.trim().is_empty()
        || manifest.subjects.history_post_id == 0
        || manifest.subjects.revision_id == 0
    {
        return Err(ManifestError::Cursor);
    }
    for requirement in &manifest.plan.cursor_requirements {
        let matches = manifest
            .cursors
            .iter()
            .filter(|cursor| cursor.workload == requirement.workload)
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(ManifestError::Cursor);
        }
        let cursor = matches[0];
        let expected_timeline = matches!(
            cursor.workload,
            Workload::PublicTimeline | Workload::AuthenticatedTimeline
        );
        if cursor.target_percent != 80
            || cursor.matching_result_count == 0
            || cursor.resolved_rank != rank(cursor.matching_result_count)
            || !valid_persisted_cursor(expected_timeline, &cursor.cursor)
        {
            return Err(ManifestError::Cursor);
        }
    }
    Ok(())
}
fn valid_persisted_cursor(expected_timeline: bool, cursor: &crate::PersistedCursor) -> bool {
    match cursor {
        crate::PersistedCursor::Timeline(timeline) => expected_timeline && timeline.post_id != 0,
        crate::PersistedCursor::History(history) => !expected_timeline && history.revision_id != 0,
    }
}

fn rank(count: u64) -> u64 {
    count / 100 * 80 + (count % 100 * 80).div_ceil(100)
}
fn allocate(total: u64, buckets: &[(&str, u64)]) -> Vec<Distribution> {
    let d = buckets.iter().map(|(_, w)| w).sum::<u64>();
    let mut r = buckets
        .iter()
        .map(|(n, w)| Distribution {
            bucket: (*n).into(),
            count: total * w / d,
        })
        .collect::<Vec<_>>();
    let assigned = r.iter().map(|b| b.count).sum::<u64>();
    let mut rem = buckets
        .iter()
        .enumerate()
        .map(|(i, (_, w))| (i, total * w % d))
        .collect::<Vec<_>>();
    rem.sort_by_key(|(i, r)| (std::cmp::Reverse(*r), *i));
    for ((i, _), _) in rem.into_iter().zip(0..(total - assigned)) {
        r[i].count += 1;
    }
    r
}
fn evenly(total: u64, buckets: &[&str]) -> Vec<Distribution> {
    let bucket_count = buckets.len() as u64;
    let mut r = buckets
        .iter()
        .map(|n| Distribution {
            bucket: (*n).into(),
            count: total / bucket_count,
        })
        .collect::<Vec<_>>();
    for (_, bucket) in (0..(total % bucket_count)).zip(&mut r) {
        bucket.count += 1;
    }
    r
}
#[cfg(test)]
mod tests {
    use super::*;
    fn counts(x: &[Distribution]) -> Vec<u64> {
        x.iter().map(|x| x.count).collect()
    }
    #[test]
    fn canonical_profiles_have_literal_totals_and_buckets() {
        let s = canonical_plan(DatasetProfile::Small);
        assert_eq!((s.posts, s.authors, s.revisions), (100, 10, 500));
        assert_eq!(counts(&s.lifecycle), [60, 15, 15, 10, 20]);
        assert_eq!(
            counts(&s.body_distribution),
            [12, 11, 11, 11, 11, 11, 11, 11, 11]
        );
        let m = canonical_plan(DatasetProfile::Medium);
        assert_eq!((m.posts, m.authors, m.revisions), (5_000, 100, 25_000));
        assert_eq!(counts(&m.tag_distribution), [1250, 2500, 1250]);
        let l = canonical_plan(DatasetProfile::Large);
        assert_eq!((l.posts, l.authors, l.revisions), (50_000, 1_000, 250_000));
        assert_eq!(counts(&l.media_distribution), [37500, 10000, 2500]);
        assert_eq!(l.cursor_requirements.len(), 4);
    }

    #[test]
    fn computes_the_cursor_rank_without_overflow() {
        assert_eq!(rank(u64::MAX), 14_757_395_258_967_641_292);
    }
}
