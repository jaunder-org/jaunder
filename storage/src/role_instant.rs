//! Shared implementation macro for seam-local timestamp role wrappers.

macro_rules! impl_role_instant {
    ($name:ident) => {
        impl $name {
            /// The wrapped UTC instant.
            #[must_use]
            fn value(self) -> chrono::DateTime<chrono::Utc> {
                self.0
            }
        }

        impl From<chrono::DateTime<chrono::Utc>> for $name {
            fn from(instant: chrono::DateTime<chrono::Utc>) -> Self {
                Self(instant)
            }
        }
    };
}

pub(crate) use impl_role_instant;
