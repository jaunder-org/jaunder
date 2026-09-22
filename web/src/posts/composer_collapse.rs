//! Host-tested scroll policy for the collapsible Home Post composer.

const COLLAPSE_Y: f64 = 96.0;
const REARM_Y: f64 = 24.0;

/// Action exposed by the single Home composer toggle row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HomeComposerToggleAction {
    Collapse,
    Expand,
}

/// Accessible attributes and label for the current Home composer state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HomeComposerTogglePresentation {
    pub id: &'static str,
    pub expanded: bool,
    pub label: &'static str,
    pub action: HomeComposerToggleAction,
}

/// Present one toggle row without duplicating expanded and collapsed markup.
#[must_use]
pub fn home_composer_toggle_presentation(collapsed: bool) -> HomeComposerTogglePresentation {
    if collapsed {
        HomeComposerTogglePresentation {
            id: "home-composer-expand",
            expanded: false,
            label: "Expand composer",
            action: HomeComposerToggleAction::Expand,
        }
    } else {
        HomeComposerTogglePresentation {
            id: "home-composer-collapse",
            expanded: true,
            label: "Collapse composer",
            action: HomeComposerToggleAction::Collapse,
        }
    }
}

/// Whether expanding at this offset preserves automatic-collapse eligibility.
///
/// Returning to the top zone earns the next automatic collapse even when the
/// composer is still collapsed. Expanding down-page remains stable instead.
#[must_use]
pub fn home_composer_armed_after_expansion(scroll_y: f64) -> bool {
    scroll_y <= REARM_Y
}

/// State changes produced by one Home document-scroll observation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HomeComposerScrollDecision {
    /// Whether a later downward threshold crossing may collapse the composer.
    pub armed: bool,
    /// Whether this observation should collapse the composer now.
    pub collapse: bool,
}

/// Decide whether one document-scroll observation re-arms or collapses the composer.
#[must_use]
pub fn home_composer_scroll_decision(
    scroll_y: f64,
    previous_scroll_y: f64,
    armed: bool,
    eligible: bool,
) -> HomeComposerScrollDecision {
    if scroll_y <= REARM_Y {
        return HomeComposerScrollDecision {
            armed: true,
            collapse: false,
        };
    }

    let collapse = armed && eligible && scroll_y >= COLLAPSE_Y && scroll_y > previous_scroll_y;
    HomeComposerScrollDecision {
        armed: armed && !collapse,
        collapse,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_presentation_tracks_the_composer_state() {
        assert_eq!(
            home_composer_toggle_presentation(false),
            HomeComposerTogglePresentation {
                id: "home-composer-collapse",
                expanded: true,
                label: "Collapse composer",
                action: HomeComposerToggleAction::Collapse,
            }
        );
        assert_eq!(
            home_composer_toggle_presentation(true),
            HomeComposerTogglePresentation {
                id: "home-composer-expand",
                expanded: false,
                label: "Expand composer",
                action: HomeComposerToggleAction::Expand,
            }
        );
    }

    #[test]
    fn expansion_preserves_only_top_zone_rearming() {
        assert!(home_composer_armed_after_expansion(24.0));
        assert!(!home_composer_armed_after_expansion(24.1));
        assert!(!home_composer_armed_after_expansion(96.0));
    }

    #[test]
    fn downward_threshold_crossing_collapses_once() {
        assert_eq!(
            home_composer_scroll_decision(96.0, 95.0, true, true),
            HomeComposerScrollDecision {
                armed: false,
                collapse: true,
            }
        );
        assert_eq!(
            home_composer_scroll_decision(200.0, 96.0, false, true),
            HomeComposerScrollDecision {
                armed: false,
                collapse: false,
            }
        );
    }

    #[test]
    fn top_zone_rearms_without_collapsing() {
        assert_eq!(
            home_composer_scroll_decision(24.0, 200.0, false, true),
            HomeComposerScrollDecision {
                armed: true,
                collapse: false,
            }
        );
    }

    #[test]
    fn ineligible_or_upward_scroll_does_not_collapse() {
        assert!(!home_composer_scroll_decision(120.0, 95.0, true, false).collapse);
        assert!(!home_composer_scroll_decision(120.0, 130.0, true, true).collapse);
        assert!(!home_composer_scroll_decision(95.0, 24.0, true, true).collapse);
    }
}
