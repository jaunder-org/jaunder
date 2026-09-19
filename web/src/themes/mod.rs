//! Owner-authorized Theme Package management API and private Studio journey.
//!
//! Draft CSS is returned only to the browser component and previewed in a
//! sandboxed document; it never joins the Studio cascade.

mod api;
mod page_state;
mod presentation_state;
#[cfg(feature = "server")]
mod thumbnail;

#[cfg(target_arch = "wasm32")]
mod component;

pub use api::{
    CatalogEntry, Create, Draft, Export, ExportedPackage, GetDraft, GetPresentation, GetSelection,
    ImportCss, ImportPackage, ImportZip, List, OwnershipScope, Preview, Publish, Remove, Rename,
    ReplaceBinding, ReplaceCss, ReplacePool, Select, Shuffle, ThemeAssetInput, ThemeBindingInput,
    ThemeMediaInput, ThemePoolInput, ThemePresentation, ThemePreview, create, export, get_draft,
    get_presentation, get_selection, import_css, import_package, import_zip, list, preview,
    publish, remove, rename, replace_binding, replace_css, replace_pool, select, shuffle,
};

#[cfg(feature = "server")]
pub use thumbnail::thumbnail_document;

pub use page_state::{
    Revalidation, ScopeAvailability, ThemePageState, draft_from_editor, revalidation,
};
pub use presentation_state::{
    HeaderMediaDraft, LogoFallbackOption, MEDIA_PAGE_SIZE, logo_fallback_option, media_page_offset,
};

#[cfg(target_arch = "wasm32")]
pub use component::ThemesPage;
