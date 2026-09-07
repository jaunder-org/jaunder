//! Owner-authorized Theme Package management API.
//!
//! This vertical contains server functions and wire DTOs only. Studio routes and
//! components intentionally belong to the following UI task.

mod api;

pub use api::{
    CatalogEntry, Create, Draft, Export, ExportedPackage, GetDraft, GetPresentation, GetSelection,
    ImportCss, ImportPackage, ImportZip, List, OwnershipScope, Preview, Publish, Remove, Rename,
    ReplaceBinding, ReplaceCss, ReplacePool, Select, Shuffle, ThemeAssetInput, ThemeBindingInput,
    ThemeMediaInput, ThemePoolInput, ThemePresentation, ThemePreview, create, export, get_draft,
    get_presentation, get_selection, import_css, import_package, import_zip, list, preview,
    publish, remove, rename, replace_binding, replace_css, replace_pool, select, shuffle,
};
