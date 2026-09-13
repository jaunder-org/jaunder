use crate::model::{
    CountOverrides, CursorRequirement, DatasetManifest, DatasetPlan, DatasetProfile, Distribution,
    GENERATOR_SEED, GENERATOR_VERSION, GeneratorIdentity, PlanError, Workload,
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

fn canonical_counts(profile: DatasetProfile) -> (u64, u64, u64) {
    match profile {
        DatasetProfile::Small => (100, 10, 500),
        DatasetProfile::Medium => (5_000, 100, 25_000),
        DatasetProfile::Large => (50_000, 1_000, 250_000),
    }
}

/// Builds a deterministic allocation from a profile and optional count changes.
///
/// Every accepted plan has positive post and author counts, distributes posts
/// evenly across authors, and assigns at least one revision to every post.
/// Noncanonical plans must also provide more than 50 history rows per author.
/// These constraints make arbitrary revision totals deterministic: the canonical
/// shape is retained without changes; otherwise each post receives
/// `revisions / posts` revisions; the first `revisions % posts` posts receive
/// one additional revision.
///
/// # Errors
///
/// Returns [`PlanError`] when the requested counts cannot produce that
/// deterministic fixture.
pub fn plan(profile: DatasetProfile, overrides: CountOverrides) -> Result<DatasetPlan, PlanError> {
    let (canonical_posts, canonical_authors, canonical_revisions) = canonical_counts(profile);
    let posts = overrides.posts.unwrap_or(canonical_posts);
    let authors = overrides.authors.unwrap_or(canonical_authors);
    let revisions = overrides.revisions.unwrap_or(canonical_revisions);
    if posts == 0 {
        return Err(PlanError::Posts);
    }
    if authors == 0 {
        return Err(PlanError::Authors);
    }
    if !posts.is_multiple_of(authors) {
        return Err(PlanError::AuthorDistribution);
    }
    if revisions < posts {
        return Err(PlanError::Revisions);
    }
    if !overrides.is_empty() && revisions / authors <= 50 {
        return Err(PlanError::HistoryCursor);
    }

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
    Ok(DatasetPlan {
        profile,
        generator: GeneratorIdentity {
            version: GENERATOR_VERSION,
            seed: GENERATOR_SEED,
        },
        posts,
        authors,
        revisions,
        lifecycle,
        revision_distribution: if overrides.is_empty() {
            allocate(posts, &[("one", 70), ("five", 20), ("thirty_three", 10)])
        } else {
            let remainder = revisions % posts;
            vec![
                Distribution {
                    bucket: "one_extra".into(),
                    count: remainder,
                },
                Distribution {
                    bucket: "base".into(),
                    count: posts - remainder,
                },
            ]
        },
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
    })
}

/// Returns the deterministic fixture plan for a built-in profile.
///
/// # Panics
///
/// Panics only if a built-in canonical profile violates the fixture invariants.
#[must_use]
pub fn canonical_plan(profile: DatasetProfile) -> DatasetPlan {
    match plan(profile, CountOverrides::default()) {
        Ok(plan) => plan,
        Err(error) => unreachable!("built-in canonical counts must be valid: {error}"),
    }
}

/// Validates that a dataset manifest matches its deterministic profile plan and cursor contract.
///
/// # Errors
///
/// Returns [`ManifestError`] when the schema, deterministic plan, workload subjects,
/// or resolved cursors are invalid.
pub fn validate_manifest(manifest: &DatasetManifest) -> Result<(), ManifestError> {
    if manifest.schema_version != crate::DATASET_SCHEMA_VERSION {
        return Err(ManifestError::Schema);
    }
    let canonical = canonical_plan(manifest.plan.profile);
    let overrides = CountOverrides {
        posts: (manifest.plan.posts != canonical.posts).then_some(manifest.plan.posts),
        authors: (manifest.plan.authors != canonical.authors).then_some(manifest.plan.authors),
        revisions: (manifest.plan.revisions != canonical.revisions)
            .then_some(manifest.plan.revisions),
    };
    let expected = plan(manifest.plan.profile, overrides);
    let canonical_count_override = plan(
        manifest.plan.profile,
        CountOverrides {
            posts: Some(manifest.plan.posts),
            authors: Some(manifest.plan.authors),
            revisions: Some(manifest.plan.revisions),
        },
    );
    if expected.ok().as_ref() != Some(&manifest.plan)
        && canonical_count_override.ok().as_ref() != Some(&manifest.plan)
    {
        return Err(ManifestError::Plan);
    }
    let browser_rows = &manifest.subjects.browser_initial_rows;
    if manifest.cursors.len() != manifest.plan.cursor_requirements.len()
        || manifest.subjects.username.trim().is_empty()
        || manifest.subjects.history_post_id == 0
        || manifest.subjects.revision_id == 0
        || browser_rows.home == 0
        || browser_rows.app == 0
        || browser_rows.global_history <= 50
        || browser_rows.post_history == 0
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
    let cursor_count = |workload| {
        manifest
            .cursors
            .iter()
            .find(|cursor| cursor.workload == workload)
            .map(|cursor| cursor.matching_result_count)
    };
    if cursor_count(Workload::PublicTimeline) != Some(browser_rows.home)
        || cursor_count(Workload::OwnerHistory) != Some(browser_rows.global_history)
        || cursor_count(Workload::PostHistory) != Some(browser_rows.post_history)
    {
        return Err(ManifestError::Cursor);
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
            count: total / d * w + total % d * w / d,
        })
        .collect::<Vec<_>>();
    let assigned = r.iter().map(|b| b.count).sum::<u64>();
    let mut rem = buckets
        .iter()
        .enumerate()
        .map(|(i, (_, w))| (i, total % d * w % d))
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
        assert_eq!(counts(&s.revision_distribution), [70, 20, 10]);
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

    #[test]
    fn plans_exact_overridden_totals_deterministically() {
        let overrides = CountOverrides {
            posts: Some(120),
            authors: Some(12),
            revisions: Some(777),
        };
        let first = plan(DatasetProfile::Small, overrides).expect("valid explicit counts");
        let second = plan(DatasetProfile::Small, overrides).expect("valid explicit counts");
        assert_eq!(
            (first.posts, first.authors, first.revisions),
            (120, 12, 777)
        );
        assert_eq!(first, second);
        assert_ne!(first, canonical_plan(DatasetProfile::Small));
        assert_eq!(
            first.revision_distribution,
            vec![
                Distribution {
                    bucket: "one_extra".into(),
                    count: 57,
                },
                Distribution {
                    bucket: "base".into(),
                    count: 63,
                },
            ]
        );
        assert_eq!(
            first
                .revision_distribution
                .iter()
                .map(|item| match item.bucket.as_str() {
                    "base" => item.count * (first.revisions / first.posts),
                    "one_extra" => item.count * (first.revisions / first.posts + 1),
                    _ => unreachable!("uniform revision bucket"), // cov:ignore: the assertion iterates the planner's closed bucket set
                })
                .sum::<u64>(),
            first.revisions
        );
    }

    #[test]
    fn rejects_tampered_overridden_plan() {
        let mut tampered = plan(
            DatasetProfile::Small,
            CountOverrides {
                posts: Some(120),
                authors: Some(12),
                revisions: Some(777),
            },
        )
        .expect("valid explicit counts");
        tampered.lifecycle[0].count += 1;
        let manifest = DatasetManifest {
            schema_version: crate::DATASET_SCHEMA_VERSION,
            plan: tampered,
            subjects: crate::WorkloadSubjects {
                username: "fixture".into(),
                history_post_id: 1,
                revision_id: 1,
                browser_initial_rows: crate::BrowserInitialRows {
                    home: 1,
                    app: 1,
                    global_history: 51,
                    post_history: 1,
                },
            },
            cursors: vec![],
        };
        assert_eq!(validate_manifest(&manifest), Err(ManifestError::Plan));
    }

    #[test]
    fn rejects_tampered_override_revision_distribution() {
        let mut tampered = plan(
            DatasetProfile::Small,
            CountOverrides {
                posts: Some(120),
                authors: Some(12),
                revisions: Some(777),
            },
        )
        .expect("valid explicit counts");
        tampered.revision_distribution[0].count -= 1;
        let manifest = DatasetManifest {
            schema_version: crate::DATASET_SCHEMA_VERSION,
            plan: tampered,
            subjects: crate::WorkloadSubjects {
                username: "fixture".into(),
                history_post_id: 1,
                revision_id: 1,
                browser_initial_rows: crate::BrowserInitialRows {
                    home: 1,
                    app: 1,
                    global_history: 51,
                    post_history: 1,
                },
            },
            cursors: vec![],
        };
        assert_eq!(validate_manifest(&manifest), Err(ManifestError::Plan));
    }

    #[test]
    fn rejects_count_relationships_that_cannot_seed_deterministically() {
        assert_eq!(
            plan(
                DatasetProfile::Small,
                CountOverrides {
                    posts: Some(121),
                    authors: Some(12),
                    revisions: Some(777),
                },
            ),
            Err(PlanError::AuthorDistribution)
        );
        assert_eq!(
            plan(
                DatasetProfile::Small,
                CountOverrides {
                    posts: Some(120),
                    authors: Some(12),
                    revisions: Some(119),
                },
            ),
            Err(PlanError::Revisions)
        );
    }
}
