//! Host-tested Manage Posts selection and confirmation state.

use std::collections::BTreeMap;

use common::ids::PostId;
use common::visibility::AudienceSelection;
use leptos::prelude::{GetUntracked, RwSignal, Set, Update};

use super::{
    BulkManageResult, BulkSelectionTarget, ManageAudienceFilter, ManagePostsCursor,
    ManagePostsPage, ManagePublicationState, ManagedPost, ManagementSelectionSnapshot,
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
    pub confirmation: RwSignal<Option<(ManageConfirmationKind, ManagementSelectionSnapshot)>>,
    pub pending: RwSignal<bool>,
    pub success: RwSignal<Option<String>>,
    pub delete_count: RwSignal<String>,
    pub named_audiences: RwSignal<Vec<crate::audiences::Summary>>,
    pub replacement: RwSignal<AudienceSelection>,
    list_generation: RwSignal<u64>,
}

/// Binds one list load's completion to the request that started it.
///
/// Each new load advances the generation, so a superseded load's late
/// completion can no longer settle the page (#1988).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ManageListClaim(u64);

impl ManagePageState {
    /// Start a list load, superseding any load still in flight.
    #[must_use]
    pub fn begin_list_load(self) -> ManageListClaim {
        self.list_generation.update(|generation| *generation += 1);
        self.loading.set(true);
        self.error.set(None);
        ManageListClaim(self.list_generation.get_untracked())
    }

    /// Settle a list load unless a later load has superseded it.
    pub fn finish_list_load(
        self,
        claim: ManageListClaim,
        outcome: Result<ManagePostsPage, String>,
    ) {
        if claim.0 != self.list_generation.get_untracked() {
            return;
        }
        match outcome {
            Ok(page) => self.page.set(Some(page)),
            Err(error) => self.error.set(Some(error)),
        }
        self.loading.set(false);
    }
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
            replacement: RwSignal::new(AudienceSelection::default()),
            list_generation: RwSignal::new(0),
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

    pub fn replace_with_snapshot(&mut self, snapshot: &ManagementSelectionSnapshot) {
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
    pub fn snapshot(&self) -> ManagementSelectionSnapshot {
        let targets = self
            .targets
            .iter()
            .map(|(post_id, mutation_version)| BulkSelectionTarget {
                post_id: *post_id,
                mutation_version: *mutation_version,
            })
            .collect::<Vec<_>>();
        ManagementSelectionSnapshot {
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

    fn list_page(id: i64) -> ManagePostsPage {
        ManagePostsPage {
            posts: vec![post(id, 1)],
            next_cursor: None,
            has_more: false,
        }
    }

    #[test]
    fn only_the_latest_list_load_settles_the_page() {
        Owner::new().with(|| {
            let state = ManagePageState::default();
            let superseded = state.begin_list_load();
            let current = state.begin_list_load();

            state.finish_list_load(superseded, Ok(list_page(1)));
            state.finish_list_load(superseded, Err("stale failure".to_owned()));
            assert!(state.page.get().is_none());
            assert!(state.error.get().is_none());
            assert!(state.loading.get());

            state.finish_list_load(current, Ok(list_page(2)));
            assert_eq!(state.page.get(), Some(list_page(2)));
            assert!(!state.loading.get());

            state.finish_list_load(superseded, Ok(list_page(1)));
            state.finish_list_load(superseded, Err("stale failure".to_owned()));
            assert_eq!(state.page.get(), Some(list_page(2)));
            assert!(state.error.get().is_none());
            assert!(!state.loading.get());

            let failing = state.begin_list_load();
            assert!(state.loading.get());
            state.finish_list_load(failing, Err("Sign in to manage Posts.".to_owned()));
            assert_eq!(
                state.error.get().as_deref(),
                Some("Sign in to manage Posts.")
            );
            assert!(!state.loading.get());
        });
    }

    #[test]
    fn selection_survives_page_changes_and_snapshot_replacement() {
        let mut selection = ManageSelectionState::default();
        selection.toggle(&post(1, 2));
        selection.toggle(&post(9, 4));
        assert_eq!(selection.selected_count(), 2);
        assert_eq!(selection.post_ids(), vec![PostId::from(1), PostId::from(9)]);
        assert_eq!(selection.snapshot().selected_count, 2);

        selection.replace_with_snapshot(&ManagementSelectionSnapshot {
            targets: vec![BulkSelectionTarget {
                post_id: PostId::from(7),
                mutation_version: 3,
            }],
            selected_count: 1,
        });
        assert!(!selection.is_selected(PostId::from(1)));
        assert!(selection.is_selected(PostId::from(7)));
        selection.clear();
        assert_eq!(selection.selected_count(), 0);
        assert!(selection.post_ids().is_empty());
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
