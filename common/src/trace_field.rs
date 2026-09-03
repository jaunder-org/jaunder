//! Type-owned projections for values admitted to tracing fields.

/// A value that may be projected into a tracing field.
///
/// Implementing this trait is the admission decision. The associated value type
/// makes the exact recorded representation reviewable and prevents callers from
/// falling back to the source value's generic [`Debug`](std::fmt::Debug).
pub trait TraceField {
    /// The exact value presented to tracing.
    type Value<'a>: std::fmt::Debug
    where
        Self: 'a;

    /// Project this value into its tracing-safe representation.
    fn trace_value(&self) -> Self::Value<'_>;
}

use crate::backup::{BackupMode, BackupSchedule, DestinationPath, RetentionCount};
use crate::ids::{AudienceId, PostId, RevisionId, SubscriptionId};
use crate::invite::InviteTtlHours;
use crate::media::{hash::ContentHash, storage::MediaSource};
use crate::pagination::{PageOffset, PageSize};
use crate::render::PostFormat;
use crate::seed::PageCursor;
use crate::site::SiteTitle;
use crate::slug::Slug;
use crate::tag::Tag;
use crate::tagged_url::BaseUrl;
use crate::time::{PermalinkDate, UtcInstant};
use crate::username::Username;

impl TraceField for bool {
    type Value<'a> = Self;

    fn trace_value(&self) -> Self::Value<'_> {
        *self
    }
}

impl TraceField for u32 {
    type Value<'a> = Self;

    fn trace_value(&self) -> Self::Value<'_> {
        *self
    }
}

impl<T: TraceField> TraceField for Option<T> {
    type Value<'a>
        = Option<T::Value<'a>>
    where
        Self: 'a;

    fn trace_value(&self) -> Self::Value<'_> {
        self.as_ref().map(TraceField::trace_value)
    }
}

impl<T: TraceField + ?Sized> TraceField for &T {
    type Value<'a>
        = T::Value<'a>
    where
        Self: 'a;

    fn trace_value(&self) -> Self::Value<'_> {
        (*self).trace_value()
    }
}

macro_rules! impl_borrowed_trace_field {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl TraceField for $ty {
                type Value<'a> = &'a Self;

                fn trace_value(&self) -> Self::Value<'_> {
                    self
                }
            }
        )+
    };
}

impl_borrowed_trace_field!(
    PostId,
    RevisionId,
    AudienceId,
    SubscriptionId,
    ContentHash,
    PageSize,
    PageOffset,
    RetentionCount,
    InviteTtlHours,
    UtcInstant,
    PageCursor,
    PostFormat,
    MediaSource,
    BackupMode,
    DestinationPath,
    SiteTitle,
    BaseUrl,
    BackupSchedule,
    Slug,
    PermalinkDate,
    Tag,
    Username,
);

#[cfg(test)]
mod tests {
    use super::TraceField;
    use crate::backup::{BackupMode, BackupSchedule, DestinationPath, RetentionCount};
    use crate::ids::{AudienceId, PostId, SubscriptionId};
    use crate::invite::InviteTtlHours;
    use crate::media::{hash::ContentHash, storage::MediaSource};
    use crate::pagination::{PageOffset, PageSize};
    use crate::render::PostFormat;
    use crate::seed::PageCursor;
    use crate::site::SiteTitle;
    use crate::slug::Slug;
    use crate::tag::Tag;
    use crate::tagged_url::BaseUrl;
    use crate::time::{PermalinkDate, UtcInstant};
    use crate::username::Username;

    fn assert_borrowed_projection<T>()
    where
        for<'a> T: TraceField<Value<'a> = &'a T>,
    {
    }

    fn assert_copy_projection<T>()
    where
        T: Copy,
        for<'a> T: TraceField<Value<'a> = T>,
    {
    }

    #[test]
    fn approved_types_lock_their_zero_allocation_projection_shape() {
        assert_copy_projection::<bool>();
        assert_copy_projection::<u32>();

        assert_borrowed_projection::<PostId>();
        assert_borrowed_projection::<AudienceId>();
        assert_borrowed_projection::<SubscriptionId>();
        assert_borrowed_projection::<ContentHash>();
        assert_borrowed_projection::<PageSize>();
        assert_borrowed_projection::<PageOffset>();
        assert_borrowed_projection::<RetentionCount>();
        assert_borrowed_projection::<InviteTtlHours>();
        assert_borrowed_projection::<UtcInstant>();
        assert_borrowed_projection::<PageCursor>();
        assert_borrowed_projection::<PostFormat>();
        assert_borrowed_projection::<MediaSource>();
        assert_borrowed_projection::<BackupMode>();
        assert_borrowed_projection::<DestinationPath>();
        assert_borrowed_projection::<SiteTitle>();
        assert_borrowed_projection::<BaseUrl>();
        assert_borrowed_projection::<BackupSchedule>();
        assert_borrowed_projection::<Slug>();
        assert_borrowed_projection::<PermalinkDate>();
        assert_borrowed_projection::<Tag>();
        assert_borrowed_projection::<Username>();
    }

    #[test]
    fn recursive_projections_preserve_option_and_reference_shape() {
        let value = 7_u32;
        assert!(true.trace_value());
        assert_eq!(value.trace_value(), 7);
        assert_eq!(Some(value).trace_value(), Some(7));
        assert_eq!(None::<u32>.trace_value(), None);
        assert_eq!(<&u32 as TraceField>::trace_value(&&value), 7);
    }
}
