//! Typed revalidation scopes for the authenticated shell's persisted warnings.
//!
//! A warning reads its authoritative persisted predicate after a relevant settings
//! mutation settles. The two distinct context types prevent a site save from
//! accidentally refreshing the backup predicate (or vice versa).

use crate::reactive::invalidator_scope;
use common::MutationOutcome;

invalidator_scope! {
    /// Revalidates only the shell's site-base-URL warning.
    pub(crate) struct SiteBaseUrlWarning
}

invalidator_scope! {
    /// Revalidates only the shell's backup-destination warning.
    pub(crate) struct BackupWarning
}

/// Whether a settings settlement may have changed its persisted warning predicate.
///
/// Both acknowledged commits and commit-indeterminate writes require a fresh
/// authoritative read. An outer error is rollback-confirmed for this client
/// mutation, so it does not notify the warning scope.
pub(crate) fn revalidates_warning<T, E>(settlement: &Result<MutationOutcome<T>, E>) -> bool {
    matches!(
        settlement,
        Ok(MutationOutcome::Confirmed(_) | MutationOutcome::CommitIndeterminate(_))
    )
}

#[cfg(test)]
mod tests {
    use super::{BackupWarning, SiteBaseUrlWarning, revalidates_warning};
    use crate::reactive::Invalidator;
    use common::MutationOutcome;
    use leptos::reactive::owner::Owner;

    #[test]
    fn mutation_settlements_gate_warning_revalidation() {
        assert!(revalidates_warning::<(), ()>(&Ok(
            MutationOutcome::Confirmed(())
        )));
        assert!(revalidates_warning::<(), ()>(&Ok(
            MutationOutcome::CommitIndeterminate(())
        )));
        assert!(!revalidates_warning::<(), ()>(&Err(())));
    }

    #[test]
    fn site_and_backup_warning_scopes_are_independent() {
        Owner::new().with(|| {
            let site = SiteBaseUrlWarning(Invalidator::new());
            let backup = BackupWarning(Invalidator::new());
            let site_before = site.track();
            let backup_before = backup.track();

            site.notify();
            assert_ne!(site.track(), site_before);
            assert_eq!(backup.track(), backup_before);

            backup.notify();
            assert_eq!(site.track(), site_before.wrapping_add(1));
            assert_ne!(backup.track(), backup_before);
        });
    }
}
