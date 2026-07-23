//! Reusable `sqlx` bridge for `strum` string enums stored as a TEXT token.
//!
//! `strum` has no `sqlx` integration and `sqlx`'s own `#[derive(Type)]` maps a plain
//! enum to a *native* DB enum type (not the TEXT token these are stored as, dual
//! backend). So this declarative macro lifts the `String`-delegating bridge shape
//! (`RenderedHtml`, [`crate::render`]) into one reusable definition: given a type that
//! is `AsRef<str>` (strum `AsRefStr`) + `FromStr` (strum `EnumString`), it
//! binds/decodes the token as a typed value — like the `StrNewtype` newtypes (#438) —
//! instead of a stringly `.as_str()`/`.to_string()` strip. Introduced with `PostFormat`
//! (#572); reused for the other stored enums in #607.

/// Emit `sqlx::Type`/`Encode`/`Decode` for a `strum` string enum stored as TEXT.
///
/// Requires `$ty: AsRef<str> + FromStr` where the `FromStr::Err` is
/// `std::error::Error + Send + Sync + 'static` (so a malformed column surfaces as a
/// column-decode error).
macro_rules! impl_text_column_enum {
    ($ty:ty) => {
        #[cfg(feature = "sqlx")]
        const _: () = {
            impl<DB: sqlx::Database> sqlx::Type<DB> for $ty
            where
                String: sqlx::Type<DB>,
            {
                fn type_info() -> <DB as sqlx::Database>::TypeInfo {
                    <String as sqlx::Type<DB>>::type_info()
                }
                fn compatible(ty: &<DB as sqlx::Database>::TypeInfo) -> bool {
                    <String as sqlx::Type<DB>>::compatible(ty)
                }
            }

            impl<'q, DB: sqlx::Database> sqlx::Encode<'q, DB> for $ty
            where
                String: sqlx::Encode<'q, DB>,
            {
                fn encode_by_ref(
                    &self,
                    buf: &mut <DB as sqlx::Database>::ArgumentBuffer<'q>,
                ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
                    // Encode an owned `String` (delegating like `RenderedHtml`): the
                    // token is a `&'static str`, but `&str: Encode` ties the borrow to
                    // the buffer's `'q`, so a String value is the portable choice.
                    let token: String = self.as_ref().to_owned();
                    <String as sqlx::Encode<'q, DB>>::encode_by_ref(&token, buf)
                }
            }

            impl<'r, DB: sqlx::Database> sqlx::Decode<'r, DB> for $ty
            where
                &'r str: sqlx::Decode<'r, DB>,
            {
                fn decode(
                    value: <DB as sqlx::Database>::ValueRef<'r>,
                ) -> Result<Self, sqlx::error::BoxDynError> {
                    let s = <&str as sqlx::Decode<'r, DB>>::decode(value)?;
                    Ok(s.parse()?)
                }
            }
        };
    };
}

pub(crate) use impl_text_column_enum;
