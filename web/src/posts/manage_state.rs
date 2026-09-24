//! Host-tested Manage Posts selection and confirmation state.

use std::collections::BTreeMap;

use common::ids::PostId;
use common::visibility::{AudienceBase, AudienceSelection};
use leptos::prelude::RwSignal;

use super::{
    BulkManageResult, BulkSelectionSnapshot, BulkSelectionTarget, ManageAudienceFilter,
    ManagePostsCursor, ManagePostsPage, ManagePublicationState, ManagedPost,
};

/// Which bulk confirmation the page is presenting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManageConfirmationKind {
    Audience,
    Delete,
}

/// Shared reactive state for the thin routed component and its subcomponents.
#[derive(Clone, Copy)]
pub struct ManagePageState {
    pub state: RwSignal<ManagePublicationState>,
    pub audience: RwSignal<ManageAudienceFilter>,
    pub search_input: RwSignal<String>,
    pub search: RwSignal<String>,
    pub cursor: RwSignal<Option<ManagePostsCursor>>,
    pub reload: RwSignal<u64>,
    pub page: RwSignal<Option<ManagePostsPage>>,
    pub loading: RwSignal<bool>,
    pub error: RwSignal<Option<String>>,
    pub selection: RwSignal<ManageSelectionState>,
    pub confirmation: RwSignal<Option<(ManageConfirmationKind, BulkSelectionSnapshot)>>,
    pub pending: RwSignal<bool>,
    pub success: RwSignal<Option<String>>,
    pub delete_count: RwSignal<String>,
    pub named_audiences: RwSignal<Vec<crate::audiences::Summary>>,
    pub replacement: RwSignal<AudienceSelection>,
}

impl Default for ManagePageState {
    fn default() -> Self {
        Self {
            state: RwSignal::new(ManagePublicationState::All),
            audience: RwSignal::new(ManageAudienceFilter::All),
            search_input: RwSignal::new(String::new()),
            search: RwSignal::new(String::new()),
            cursor: RwSignal::new(None),
            reload: RwSignal::new(0),
            page: RwSignal::new(None),
            loading: RwSignal::new(true),
            error: RwSignal::new(None),
            selection: RwSignal::new(ManageSelectionState::default()),
            confirmation: RwSignal::new(None),
            pending: RwSignal::new(false),
            success: RwSignal::new(None),
            delete_count: RwSignal::new(String::new()),
            named_audiences: RwSignal::new(Vec::new()),
            replacement: RwSignal::new(AudienceSelection {
                base: AudienceBase::Private,
                named: Vec::new(),
            }),
        }
    }
}

/// Stable exact selection retained while management pages change.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ManageSelectionState {
    targets: BTreeMap<PostId, i64>,
}

impl ManageSelectionState {
    #[must_use]
    pub fn selected_count(&self) -> usize {
        self.targets.len()
    }

    #[must_use]
    pub fn is_selected(&self, post_id: PostId) -> bool {
        self.targets.contains_key(&post_id)
    }

    pub fn toggle(&mut self, post: &ManagedPost) {
        if self.targets.remove(&post.post_id).is_none() {
            self.targets.insert(post.post_id, post.mutation_version);
        }
    }

    pub fn replace_with_snapshot(&mut self, snapshot: &BulkSelectionSnapshot) {
        self.targets = snapshot
            .targets
            .iter()
            .map(|target| (target.post_id, target.mutation_version))
            .collect();
    }

    pub fn clear(&mut self) {
        self.targets.clear();
    }

    #[must_use]
    pub fn post_ids(&self) -> Vec<PostId> {
        self.targets.keys().copied().collect()
    }

    #[must_use]
    pub fn snapshot(&self) -> BulkSelectionSnapshot {
        let targets = self
            .targets
            .iter()
            .map(|(post_id, mutation_version)| BulkSelectionTarget {
                post_id: *post_id,
                mutation_version: *mutation_version,
            })
            .collect::<Vec<_>>();
        BulkSelectionSnapshot {
            selected_count: targets.len(),
            targets,
        }
    }
}

/// Large destructive selections require an exact count acknowledgement.
#[must_use]
pub const fn delete_requires_count(selected_count: usize) -> bool {
    selected_count >= 10
}

/// Whether a delete confirmation satisfies the count safeguard.
#[must_use]
pub fn delete_count_matches(selected_count: usize, entered: &str) -> bool {
    !delete_requires_count(selected_count) || entered.trim() == selected_count.to_string()
}

/// User-facing committed result text.
#[must_use]
pub fn bulk_result_message(result: BulkManageResult) -> String {
    format!(
        "Selected {} Posts; changed {}.",
        result.selected_count, result.changed_count
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::slug::Slug;
    use common::test_support::{parse_post_title, parse_utc_instant};
    use leptos::prelude::{Get, Owner};

    fn post(id: i64, version: i64) -> ManagedPost {
        ManagedPost {
            post_id: PostId::from(id),
            mutation_version: version,
            title: Some(parse_post_title("Title")),
            fallback_label: super::super::UnpublishedPostLabel::Slug(
                "title".parse::<Slug>().unwrap(),
            ),
            slug: "title".parse().unwrap(),
            lifecycle: super::super::ManagedPostLifecycle::Draft,
            audiences: Vec::new(),
            updated_at: parse_utc_instant("2026-09-23T12:00:00Z"),
        }
    }

    #[test]
    fn page_state_defaults_to_a_bounded_idle_management_query() {
        Owner::new().with(|| {
            let state = ManagePageState::default();
            assert_eq!(state.state.get(), ManagePublicationState::All);
            assert_eq!(state.audience.get(), ManageAudienceFilter::All);
            assert!(state.cursor.get().is_none());
            assert!(state.loading.get());
        });
    }

    #[test]
    fn selection_survives_page_changes_and_snapshot_replacement() {
        let mut selection = ManageSelectionState::default();
        selection.toggle(&post(1, 2));
        selection.toggle(&post(9, 4));
        assert_eq!(selection.post_ids(), vec![PostId::from(1), PostId::from(9)]);
        assert_eq!(selection.snapshot().selected_count, 2);

        selection.replace_with_snapshot(&BulkSelectionSnapshot {
            targets: vec![BulkSelectionTarget {
                post_id: PostId::from(7),
                mutation_version: 3,
            }],
            selected_count: 1,
        });
        assert!(!selection.is_selected(PostId::from(1)));
        assert!(selection.is_selected(PostId::from(7)));
    }

    #[test]
    fn delete_safeguard_starts_at_ten_and_requires_exact_count() {
        assert!(!delete_requires_count(9));
        assert!(delete_count_matches(9, ""));
        assert!(delete_requires_count(10));
        assert!(!delete_count_matches(10, "9"));
        assert!(delete_count_matches(10, " 10 "));
    }

    #[test]
    fn result_message_reports_selected_and_materially_changed_counts() {
        assert_eq!(
            bulk_result_message(BulkManageResult {
                selected_count: 4,
                changed_count: 2,
            }),
            "Selected 4 Posts; changed 2."
        );
    }
}
