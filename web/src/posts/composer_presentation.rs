//! Host-tested presentation choices for the shared creation composer.

/// CSS classes selected by the creation surface's surrounding page context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CreationComposerPresentation {
    /// Grid class selected for the surrounding creation surface.
    pub layout_class: &'static str,
    /// Textarea class selected for that surface's body treatment.
    pub textarea_class: &'static str,
}

/// Choose the presentation classes without duplicating composer behavior.
#[must_use]
pub fn creation_composer_presentation(compact: bool) -> CreationComposerPresentation {
    if compact {
        CreationComposerPresentation {
            layout_class: "j-composer-layout",
            textarea_class: "",
        }
    } else {
        CreationComposerPresentation {
            layout_class: "j-compose-grid",
            textarea_class: "j-edit-form-textarea",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_surface_uses_inline_layout_classes() {
        assert_eq!(
            creation_composer_presentation(true),
            CreationComposerPresentation {
                layout_class: "j-composer-layout",
                textarea_class: "",
            }
        );
    }

    #[test]
    fn dedicated_surface_uses_full_page_layout_classes() {
        assert_eq!(
            creation_composer_presentation(false),
            CreationComposerPresentation {
                layout_class: "j-compose-grid",
                textarea_class: "j-edit-form-textarea",
            }
        );
    }
}
