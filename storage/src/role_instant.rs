//! Shared implementation macro for seam-local timestamp role wrappers.

macro_rules! impl_role_instant {
    ($name:ident, $inner:ty) => {
        impl $name {
            /// The wrapped UTC instant.
            #[must_use]
            fn value(self) -> $inner {
                self.0
            }
        }

        impl From<$inner> for $name {
            fn from(instant: $inner) -> Self {
                Self(instant)
            }
        }
    };
}

pub(crate) use impl_role_instant;
