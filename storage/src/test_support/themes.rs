use std::sync::Arc;

use common::ids::ThemeId;
use host::theme_package::CompiledThemeRevision;

use crate::{ThemeOwner, ThemeStorage, WriteScope};

pub use crate::seed_theme_fixture::theme_quota_limits;

/// Returns the validated custom-theme fixture.
///
/// # Panics
///
/// Panics if the static fixture violates the Theme Package contract.
#[must_use]
pub fn compiled_theme_fixture() -> CompiledThemeRevision {
    crate::seed_theme_fixture::try_compiled_theme_fixture().expect("valid compiled theme fixture")
}

/// Creates one site theme from the validated fixture.
///
/// # Panics
///
/// Panics if the fixture cannot be created or its commit is indeterminate.
pub async fn create_site_theme(
    themes: Arc<dyn ThemeStorage>,
    scope: WriteScope,
    compiled: &CompiledThemeRevision,
) -> ThemeId {
    crate::seed_theme_fixture::try_create_theme(themes, scope, ThemeOwner::Site, compiled)
        .await
        .expect("create site theme fixture")
}
