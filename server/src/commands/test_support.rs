use tempfile::TempDir;

use crate::cli::StorageArgs;

pub(super) fn sqlite_storage_args(temp: &TempDir) -> StorageArgs {
    StorageArgs {
        storage_path: temp.path().to_path_buf(),
        db: crate::test_support::sqlite_db_options(temp.path()),
    }
}

pub(super) fn assert_command_source<T: std::error::Error + 'static>(
    error: &anyhow::Error,
    context: &str,
) {
    assert_eq!(error.to_string(), context);
    assert!(
        error
            .chain()
            .any(|source| source.downcast_ref::<T>().is_some()),
        "typed source must remain downcastable: {error:#}"
    );
}
