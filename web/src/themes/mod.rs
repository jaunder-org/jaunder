//! Owner-authorized Theme Package management API and private Studio journey.
//!
//! Draft CSS is returned only to the browser component and previewed in a
//! sandboxed document; it never joins the Studio cascade.

mod api;
// Pure editor-input projections are consumed by the CSR UI and native regression tests.
#[cfg(any(target_arch = "wasm32", test))]
mod editor_inputs;
mod page_state;
mod presentation_state;
#[cfg(feature = "server")]
mod thumbnail;

#[cfg(target_arch = "wasm32")]
mod component;

pub use api::{
    CatalogEntry, CopyTheme, Create, Draft, Export, ExportedPackage, GetDraft, GetPresentation,
    GetSelection, ImportCss, ImportPackage, ImportZip, List, OwnershipScope, Preview,
    PreviewEditor, PreviewInstalled, Publish, Remove, Rename, ReplaceBinding, ReplaceCss,
    ReplacePool, SaveEditor, Select, Shuffle, ThemeAssetInput, ThemeBindingInput, ThemeEditorInput,
    ThemeEditorPatch, ThemeEditorSaveResult, ThemeMediaInput, ThemePoolInput, ThemePresentation,
    ThemePreview, copy_theme, create, export, get_draft, get_presentation, get_selection,
    import_css, import_package, import_zip, list, preview, preview_editor, preview_installed,
    publish, remove, rename, replace_binding, replace_css, replace_pool, save_editor, select,
    shuffle,
};

#[cfg(feature = "server")]
pub use thumbnail::thumbnail_document;

pub use page_state::{
    Feedback, MutationStage, PreviewTarget, Receipt, Revalidation, ScopeAvailability, ThemeChoice,
    ThemePageState, choice_is_current, current_theme_summary, draft_from_editor, mutation_feedback,
    mutation_message, mutation_receipt, preview_is_current, preview_target, publication_confirmed,
    reconciliation_message, revalidation, review_choice, saved_version_control_token,
    selection_control_token, selection_from_token, selection_token,
};
pub use presentation_state::{
    HeaderMediaDraft, LogoFallbackOption, MEDIA_PAGE_SIZE, logo_fallback_option, media_page_offset,
};

#[cfg(target_arch = "wasm32")]
pub use component::ThemesPage;
