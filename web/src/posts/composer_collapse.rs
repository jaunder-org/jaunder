//! Host-tested scroll policy for the collapsible Home Post composer.

const COLLAPSE_Y: f64 = 96.0;
const REARM_Y: f64 = 24.0;

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
