//! Workspace proc-macros: a target-agnostic, host-compiled build-time crate — the home
//! for the workspace's proc-macros — distinct from the `common`/`host`/`client` runtime
//! trio.

use proc_macro::TokenStream;
use syn::{Data, DeriveInput, Fields, parse_macro_input};

mod id_newtype;
mod num_newtype;
mod server_fn;
mod sqlx_bridge;
mod sqlx_bridge_derive;
mod str_newtype;
mod text_enum;

/// Derives the ADR-0063 **string-newtype trailer** for a `struct X(String)`: `Display`,
/// a serde bridge (deserialize routed through `FromStr`, so invalid input is rejected on
/// the wire), `AsRef`/`Borrow`/`Deref<str>`, `TryFrom<String>`, `From<Self> for String`,
/// `PartialEq<str>`/`<&str>`, and `PartialOrd`/`Ord` on the inner value. `FromStr` stays
/// hand-written — it is the single validating/normalizing chokepoint — as do the
/// remaining std `#[derive]`s. Because `Ord: Eq`, `PartialEq`/`Eq` are required unless
/// the type takes `#[str_newtype(no_ord)]` (#761).
///
/// `#[str_newtype(secret)]` selects the tight secret surface (redacting `Debug`,
/// `AsRef` + `TryFrom` only; no `Display`/serde/`Deref`/`Borrow`/owned-`String`/`PartialEq`).
/// `#[str_newtype(secret, serde)]` adds the validating serde bridge back onto that surface
/// for a secret that must cross the wire *inbound* (client→server) — still no `Display`/`Deref`:
///
/// ```
/// # use macros::StrNewtype;
/// # use std::str::FromStr;
/// # #[derive(Clone, StrNewtype)]
/// # #[str_newtype(secret, serde)]
/// # struct Wire(String);
/// # impl FromStr for Wire { type Err = std::convert::Infallible; fn from_str(s: &str) -> Result<Self, Self::Err> { Ok(Wire(s.to_owned())) } }
/// let w = Wire("x".to_owned());
/// assert_eq!(serde_json::to_string(&w).unwrap(), "\"x\""); // serde is back
/// let _back: Wire = serde_json::from_str("\"x\"").unwrap();
/// ```
///
/// Applying the derive to anything but a single-field tuple struct is a compile error:
///
/// ```compile_fail
/// # use macros::StrNewtype;
/// #[derive(StrNewtype)]
/// struct NotATuple { s: String }
/// ```
///
/// A single-field tuple struct with a hand-written `FromStr` compiles:
///
/// ```
/// use macros::StrNewtype;
/// use std::str::FromStr;
/// // `PartialEq, Eq` are required: the trailer emits `Ord`, and `Ord: Eq` (#761).
/// // A type that genuinely cannot have them takes `#[str_newtype(no_ord)]`.
/// #[derive(Clone, PartialEq, Eq, StrNewtype)]
/// struct Ok1(String);
/// impl FromStr for Ok1 {
///     type Err = std::convert::Infallible;
///     fn from_str(s: &str) -> Result<Self, Self::Err> { Ok(Ok1(s.to_owned())) }
/// }
/// ```
///
/// The **secret** surface omits `Display`, serde, owned-`String` extraction, value
/// `PartialEq`, and `Deref` coercion. The positive companion shows the identical fixture
/// compiles — and that `serde_json` resolves, so the serde `compile_fail` below fails for
/// the missing `Serialize`, not an unresolved crate. (Fixture lines are hidden with `#`.)
///
/// ```
/// # use macros::StrNewtype;
/// # use std::str::FromStr;
/// # use std::borrow::Borrow;
/// # #[derive(Clone, StrNewtype)]
/// # #[str_newtype(secret)]
/// # struct Sec(String);
/// # impl FromStr for Sec { type Err = std::convert::Infallible; fn from_str(s: &str) -> Result<Self, Self::Err> { Ok(Sec(s.to_owned())) } }
/// let s = Sec("x".to_owned());
/// let _read: &str = s.as_ref();                    // explicit borrowed read is allowed
/// let _ = serde_json::to_string(s.as_ref());       // serde_json resolves (a &str serializes)
/// let _b: &str = Borrow::borrow(s.as_ref());       // Borrow resolves, on a &str
/// ```
///
/// No `Display`:
/// ```compile_fail
/// # use macros::StrNewtype;
/// # use std::str::FromStr;
/// # #[derive(Clone, StrNewtype)]
/// # #[str_newtype(secret)]
/// # struct Sec(String);
/// # impl FromStr for Sec { type Err = std::convert::Infallible; fn from_str(s: &str) -> Result<Self, Self::Err> { Ok(Sec(s.to_owned())) } }
/// # let s = Sec("x".to_owned());
/// let _ = format!("{}", s);
/// ```
///
/// No serde:
/// ```compile_fail
/// # use macros::StrNewtype;
/// # use std::str::FromStr;
/// # #[derive(Clone, StrNewtype)]
/// # #[str_newtype(secret)]
/// # struct Sec(String);
/// # impl FromStr for Sec { type Err = std::convert::Infallible; fn from_str(s: &str) -> Result<Self, Self::Err> { Ok(Sec(s.to_owned())) } }
/// # let s = Sec("x".to_owned());
/// let _ = serde_json::to_string(&s);
/// ```
///
/// No owned-`String` extraction:
/// ```compile_fail
/// # use macros::StrNewtype;
/// # use std::str::FromStr;
/// # #[derive(Clone, StrNewtype)]
/// # #[str_newtype(secret)]
/// # struct Sec(String);
/// # impl FromStr for Sec { type Err = std::convert::Infallible; fn from_str(s: &str) -> Result<Self, Self::Err> { Ok(Sec(s.to_owned())) } }
/// # let s = Sec("x".to_owned());
/// let _ = String::from(s);
/// ```
///
/// No value `PartialEq`:
/// ```compile_fail
/// # use macros::StrNewtype;
/// # use std::str::FromStr;
/// # #[derive(Clone, StrNewtype)]
/// # #[str_newtype(secret)]
/// # struct Sec(String);
/// # impl FromStr for Sec { type Err = std::convert::Infallible; fn from_str(s: &str) -> Result<Self, Self::Err> { Ok(Sec(s.to_owned())) } }
/// # let s = Sec("x".to_owned());
/// let _ = s == "x";
/// ```
///
/// No `Deref` coercion to `&str`:
/// ```compile_fail
/// # use macros::StrNewtype;
/// # use std::str::FromStr;
/// # #[derive(Clone, StrNewtype)]
/// # #[str_newtype(secret)]
/// # struct Sec(String);
/// # impl FromStr for Sec { type Err = std::convert::Infallible; fn from_str(s: &str) -> Result<Self, Self::Err> { Ok(Sec(s.to_owned())) } }
/// # let s = Sec("x".to_owned());
/// let _: &str = &s;
/// ```
///
/// No `Borrow<str>` — the other omission ADR-0063 names alongside `Deref`, and the
/// reason the companion above resolves `Borrow`: without that import in scope, this
/// block would fail for an unresolved name rather than a missing impl.
/// ```compile_fail
/// # use macros::StrNewtype;
/// # use std::str::FromStr;
/// # use std::borrow::Borrow;
/// # #[derive(Clone, StrNewtype)]
/// # #[str_newtype(secret)]
/// # struct Sec(String);
/// # impl FromStr for Sec { type Err = std::convert::Infallible; fn from_str(s: &str) -> Result<Self, Self::Err> { Ok(Sec(s.to_owned())) } }
/// # let s = Sec("x".to_owned());
/// let _: &str = Borrow::borrow(&s);
/// ```
///
/// # No ordering, and the control that makes that a proof
///
/// A secret is never sorted or used as a `BTreeMap` key, and `#[str_newtype(no_ord)]`
/// suppresses the ordering half for a type that deliberately has none (`RawToken`).
///
/// These three proofs used to derive no `PartialEq`, which made them **vacuous**:
/// `a < b` would have failed to compile even if the macro *did* emit ordering, so
/// they documented intent rather than discriminating. (Their prose said so, and
/// pointed at a unit test as "the actual guard" — while that test pointed back at
/// these doctests. Neither guarded anything.)
///
/// Each fixture now derives `PartialEq, Eq`, so `a < b` can only fail for the missing
/// `PartialOrd`. The control below is what makes that argument checkable: the same
/// shape *without* a suppressing option orders, so the failures are about ordering
/// and not about the fixture.
///
/// ```
/// use macros::StrNewtype;
/// use std::str::FromStr;
/// #[derive(Clone, PartialEq, Eq, StrNewtype)]
/// struct Ordered(String);
/// impl FromStr for Ordered { type Err = std::convert::Infallible; fn from_str(s: &str) -> Result<Self, Self::Err> { Ok(Ordered(s.to_owned())) } }
/// // CONTROL: an un-suppressed newtype with the same derives DOES order.
/// assert!(Ordered("a".to_owned()) < Ordered("b".to_owned()));
/// ```
///
/// The three suppressed fixtures, which compile and do carry `PartialEq`:
/// ```
/// # use macros::StrNewtype;
/// # use std::str::FromStr;
/// #[derive(Clone, PartialEq, Eq, StrNewtype)]
/// #[str_newtype(secret)]
/// struct SecOrd(String);
/// impl FromStr for SecOrd { type Err = std::convert::Infallible; fn from_str(s: &str) -> Result<Self, Self::Err> { Ok(SecOrd(s.to_owned())) } }
/// #[derive(Clone, PartialEq, Eq, StrNewtype)]
/// #[str_newtype(secret, serde)]
/// struct SecSerdeOrd(String);
/// impl FromStr for SecSerdeOrd { type Err = std::convert::Infallible; fn from_str(s: &str) -> Result<Self, Self::Err> { Ok(SecSerdeOrd(s.to_owned())) } }
/// #[derive(Clone, PartialEq, Eq, StrNewtype)]
/// #[str_newtype(no_ord)]
/// struct Unordered(String);
/// impl FromStr for Unordered { type Err = std::convert::Infallible; fn from_str(s: &str) -> Result<Self, Self::Err> { Ok(Unordered(s.to_owned())) } }
/// // `PartialEq` is present on all three — so `<` below cannot be failing for it.
/// assert!(SecOrd("a".to_owned()) == SecOrd("a".to_owned()));
/// ```
///
/// A secret does not order:
/// ```compile_fail
/// # use macros::StrNewtype;
/// # use std::str::FromStr;
/// # #[derive(Clone, PartialEq, Eq, StrNewtype)]
/// # #[str_newtype(secret)]
/// # struct SecOrd(String);
/// # impl FromStr for SecOrd { type Err = std::convert::Infallible; fn from_str(s: &str) -> Result<Self, Self::Err> { Ok(SecOrd(s.to_owned())) } }
/// let a = SecOrd("a".to_owned());
/// let b = SecOrd("b".to_owned());
/// let _ = a < b;
/// ```
///
/// …nor the inbound `secret, serde` variant:
/// ```compile_fail
/// # use macros::StrNewtype;
/// # use std::str::FromStr;
/// # #[derive(Clone, PartialEq, Eq, StrNewtype)]
/// # #[str_newtype(secret, serde)]
/// # struct SecSerdeOrd(String);
/// # impl FromStr for SecSerdeOrd { type Err = std::convert::Infallible; fn from_str(s: &str) -> Result<Self, Self::Err> { Ok(SecSerdeOrd(s.to_owned())) } }
/// let a = SecSerdeOrd("a".to_owned());
/// let b = SecSerdeOrd("b".to_owned());
/// let _ = a < b;
/// ```
///
/// …nor a `no_ord` newtype, which keeps the rest of the trailer
/// (`str_newtype::no_ord_keeps_the_rest_of_the_trailer` covers that half):
/// ```compile_fail
/// # use macros::StrNewtype;
/// # use std::str::FromStr;
/// # #[derive(Clone, PartialEq, Eq, StrNewtype)]
/// # #[str_newtype(no_ord)]
/// # struct Unordered(String);
/// # impl FromStr for Unordered { type Err = std::convert::Infallible; fn from_str(s: &str) -> Result<Self, Self::Err> { Ok(Unordered(s.to_owned())) } }
/// let a = Unordered("a".to_owned());
/// let b = Unordered("b".to_owned());
/// let _ = a < b;
/// ```
///
/// `#[str_newtype(infallible)]` selects the **infallible** trailer for a newtype whose
/// invariant never rejects: construction is a hand-written `From<String>` (the single
/// pure-wrap or normalizing chokepoint) rather than `FromStr`, so there is no
/// `TryFrom<String>`/`FromStr`. The derive also emits a `From<&str>` alias that routes
/// through that `From<String>` — so a literal constructs with one `.into()`, no
/// `.to_owned()` — and a `Deserialize` that routes wire input through it too, so it cannot
/// fail and normalizes identically. Exclusive with `secret`/`serde` (the infallible
/// trailer already includes the serde bridge):
///
/// ```
/// use macros::StrNewtype;
/// #[derive(Clone, PartialEq, Eq, StrNewtype)]
/// #[str_newtype(infallible)]
/// struct Inf(String);
/// impl From<String> for Inf {
///     fn from(s: String) -> Self { Inf(s) }
/// }
/// let v: Inf = "x".into();                                        // From<&str>, one hop
/// assert_eq!(serde_json::to_string(&v).unwrap(), "\"x\"");        // serde bridge
/// let _back: Inf = serde_json::from_str("\"x\"").unwrap();        // deserialize via From<String>
/// ```
///
/// # The phantom-tagged form (#875)
///
/// Alone among the newtype derives, `StrNewtype` accepts a **generic** struct carrying a
/// zero-sized role marker — `struct X<T: Bound>(String, PhantomData<_>)` — and threads the
/// user's generics through every impl it emits. One trailer is then written once and
/// inherited by every role, and two roles are distinct types that cannot be transposed.
///
/// The marker field never appears in the emitted code: nothing the derive writes
/// constructs `Self`. `TryFrom`, `Deserialize`, and the sqlx `Decode` all route through the
/// author's `FromStr`, which is where the `PhantomData` is supplied — so the derive needs
/// to know only that the field is there, not what to put in it.
///
/// The std `#[derive]`s must be hand-written for such a type: `#[derive(Clone)]` would add
/// a `T: Clone` bound on a marker that is never stored.
///
/// ```
/// use macros::StrNewtype;
/// use std::marker::PhantomData;
/// use std::str::FromStr;
/// pub trait Role {}
/// pub struct Hub;
/// impl Role for Hub {}
///
/// #[derive(StrNewtype)]
/// pub struct Tagged<T: Role>(String, PhantomData<fn() -> T>);
///
/// impl<T: Role> FromStr for Tagged<T> {
///     type Err = std::convert::Infallible;
///     fn from_str(s: &str) -> Result<Self, Self::Err> { Ok(Self(s.to_owned(), PhantomData)) }
/// }
/// // `Ord: Eq`, and the trailer emits `Ord` — so these two are required, hand-written.
/// impl<T: Role> PartialEq for Tagged<T> {
///     fn eq(&self, other: &Self) -> bool { self.0 == other.0 }
/// }
/// impl<T: Role> Eq for Tagged<T> {}
///
/// let t: Tagged<Hub> = "https://hub.example.com/".parse().unwrap();
/// assert_eq!(t.to_string(), "https://hub.example.com/");
/// assert_eq!(serde_json::to_string(&t).unwrap(), "\"https://hub.example.com/\"");
/// ```
#[proc_macro_derive(StrNewtype, attributes(str_newtype))]
pub fn str_newtype_derive(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as DeriveInput);
    str_newtype::expand(&input).into()
}

/// Derives the ADR-0063 **numeric-ID trailer** for a `struct X(i64)`: `From<i64>`,
/// `From<Self> for i64`, `Display`, and a transparent-i64 serde bridge (wire form is a
/// bare integer), and `PartialOrd`/`Ord` on the inner `i64` (#761). `Copy` and the
/// remaining std traits stay in the user's `#[derive]` list; `PartialEq`/`Eq` are
/// required, since `Ord: Eq`.
///
/// Applying the derive to anything but a single-field tuple struct is a compile error:
///
/// ```compile_fail
/// # use macros::IdNewtype;
/// #[derive(IdNewtype)]
/// struct NotATuple { n: i64 }
/// ```
///
/// A single-field tuple struct compiles:
///
/// ```
/// use macros::IdNewtype;
/// // `PartialEq, Eq` are required: the trailer emits `Ord`, and `Ord: Eq` (#761).
/// #[derive(Clone, Copy, PartialEq, Eq, IdNewtype)]
/// struct Id(i64);
/// ```
///
/// # No generic form
///
/// `StrNewtype` learned the phantom-tagged shape `X<T: Bound>(String, PhantomData<_>)`
/// (#875); `IdNewtype` deliberately did not, because an id carries no role to tag. A
/// generic struct is rejected by the shape check:
///
/// ```compile_fail
/// # use std::marker::PhantomData;
/// # use macros::IdNewtype;
/// # pub trait Role {}
/// #[derive(IdNewtype)]
/// pub struct Generic<T: Role>(i64, PhantomData<fn() -> T>);
/// ```
///
/// The non-generic form is what it accepts — the same fixture lines, so the negative
/// above can only be failing for the generics:
///
/// ```
/// # use std::marker::PhantomData;
/// # use macros::IdNewtype;
/// # pub trait Role {}
/// #[derive(Clone, Copy, PartialEq, Eq, IdNewtype)]
/// pub struct PostId(i64);
/// ```
#[proc_macro_derive(IdNewtype)]
pub fn id_newtype_derive(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as DeriveInput);
    id_newtype::expand(&input).into()
}

/// Derives the ADR-0063 **numeric-value trailer** for a `struct X(I)` over an integer `I`
/// with a declarative bound. Unlike `StrNewtype` (whose `FromStr` is hand-written) and
/// `IdNewtype` (which enforces no value invariant), a numeric bound is declarative, so this
/// derive *generates* the whole trailer from `#[num_newtype(...)]`: a self-contained error
/// type, `value()`, a validating `FromStr`, `Display`, an optional compile-checked `Default`,
/// a validating transparent-integer serde bridge (out-of-range rejected on the wire), and
/// `PartialOrd`/`Ord` on the inner integer (#761). The remaining std `#[derive]`s
/// (`Clone`/`Copy`/`Debug`/`PartialEq`/`Eq`/`Hash`) stay in the user's list, with
/// `PartialEq`/`Eq` required since `Ord: Eq`.
///
/// Options: `inner = <ty>` (**required**, the wrapped integer type; the tuple field must be
/// exactly this type), `min` / `max` (inclusive bounds, each optional — the check is emitted
/// only for a declared side), `default = <int>` (generates a `Default` guarded so an
/// out-of-range default is a compile error), `error = "…"` (overrides the generated
/// `Display` message), and `clamp` (a bare flag, requires both `min` and `max`: emits
/// `MIN`/`MAX` consts and an infallible `const fn clamped(inner) -> Self` coercing into range).
///
/// ```
/// use macros::NumNewtype;
/// use std::str::FromStr;
/// #[derive(Clone, Copy, Debug, PartialEq, Eq, NumNewtype)]
/// #[num_newtype(inner = u32, min = 1, default = 20)]
/// struct MinItems(u32);
///
/// assert_eq!("7".parse::<MinItems>().unwrap().value(), 7);
/// assert!("0".parse::<MinItems>().is_err());          // below `min`
/// assert_eq!(MinItems::default().value(), 20);        // compile-checked default
/// assert_eq!(u32::from(MinItems::default()), 20);     // From<Self> for the inner
/// assert_eq!(serde_json::to_string(&MinItems::default()).unwrap(), "20"); // bare integer
/// assert!(serde_json::from_str::<MinItems>("0").is_err());                // wire rejection
/// ```
///
/// Applying the derive to anything but a single-field tuple struct is a compile error:
///
/// ```compile_fail
/// # use macros::NumNewtype;
/// #[derive(NumNewtype)]
/// #[num_newtype(inner = u32)]
/// struct NotATuple { n: u32 }
/// ```
#[proc_macro_derive(NumNewtype, attributes(num_newtype))]
pub fn num_newtype_derive(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as DeriveInput);
    num_newtype::expand(&input).into()
}

/// Declares a jaunder `#[server]` fn, deriving everything about it that the source
/// already determines (#714).
///
/// From the fn's file path and identifier it derives the wire endpoint
/// (`/<vertical>/<ident>`) and the ADR-0011 span name (`web.<vertical>.<ident>`),
/// and it wraps the body in `error::server_boundary` so operator detail cannot
/// reach the client. None of the three can drift from the fn, because none of them
/// is written down.
///
/// Write it fully qualified — `#[macros::server]`, never `use`d — so it cannot be
/// confused with leptos's `#[server]`, which this expands to.
///
/// Illustration, not a test: the fn body is an ellipsis, `AudienceId`/`AudienceName`/
/// `WebResult` live in `common`/`web` rather than this crate's dev-dependencies, and
/// the macro's first act is `Span::call_site().file()` with a hard placement check —
/// so a doctest's synthetic path fails it by construction (see the `cov:ignore` note
/// below).
///
/// ```text
/// #[macros::server(skip(name))]
/// pub async fn rename(audience_id: AudienceId, name: AudienceName) -> WebResult<()> {
///     …
/// }
/// ```
///
/// Accepts `input = …` (forwarded to `#[server]`) and `skip(...)` / `skip_all`
/// (forwarded to `#[tracing::instrument]`). Everything else — including `endpoint`,
/// `name`, and `fields(...)` — is a hard error; see [`server_fn::derive`].
///
/// **Placement is enforced.** The fn must live in `web/src/<vertical>/api.rs`; a
/// submodule is a compile error, because `(vertical, ident)` would stop being
/// unique and two fns could silently derive one wire URL (#358).
// cov:ignore-start — the only proc-macro-context code in this crate.
// `Span::call_site().file()` panics outside a live expansion, so nothing in this
// fn is reachable from `cargo test`. Every decision lives in `server_fn::expand` /
// `::derive`, which take the path as a plain parameter and are unit-tested branch
// by branch; this shell only fetches the path and renders the error.
#[proc_macro_attribute]
pub fn server(args: TokenStream, item: TokenStream) -> TokenStream {
    let file = proc_macro::Span::call_site().file();
    let args = parse_macro_input!(args with
        syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated);
    let item = parse_macro_input!(item as syn::ItemFn);
    match server_fn::expand(&file, &args.into_iter().collect::<Vec<_>>(), item) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}
// cov:ignore-stop

/// Emits **only** the `sqlx` storage bridge (`Type`/`Encode`/`Decode`) for a
/// `struct X(Inner)` — no trailer, no serde, no constructor. For a type that is stored as
/// its inner but whose construction is deliberately not the derive's business.
///
/// # The generated `Decode` is an inbound door that re-establishes no invariant
///
/// It wraps whatever the column holds, straight into `Self(..)`, with no validation and no
/// normalization. That is correct only for a type whose invariant is *inherited* from the
/// fact that we wrote the row ourselves. A type whose invariant must hold for arbitrary
/// bytes — anything reachable from outside — must either establish it on the way in through
/// its own door, or not use this derive.
///
/// `RenderedHtml` (`common/src/render.rs`) is the motivating case, and its module documents
/// exactly why a sanitizing decode was rejected there and when to revisit that.
#[proc_macro_derive(SqlxBridge, attributes(sqlx_bridge))]
pub fn sqlx_bridge_derive(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as DeriveInput);
    sqlx_bridge_derive::expand(&input).into()
}

/// The standard shape for a **closed string enum** (ADR-0075 as amended by #746): one
/// attribute that owns the whole convention.
///
/// ```
/// use std::str::FromStr;
/// #[macros::text_enum(
///     error = InvalidPostFormat,
///     message = "post format must be \"markdown\", \"org\", or \"html\"",
/// )]
/// #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, strum::VariantArray)]
/// #[strum(serialize_all = "snake_case")]
/// pub enum PostFormat {
///     #[default]
///     Markdown,
///     Org,
///     Html,
/// }
///
/// assert_eq!(PostFormat::Markdown.to_string(), "markdown");
/// assert_eq!(PostFormat::from_str("org"), Ok(PostFormat::Org));
/// assert!(PostFormat::from_str("rtf").is_err());
/// assert_eq!(serde_json::to_string(&PostFormat::Html).unwrap(), "\"html\"");
/// ```
///
/// A production call site adds `sqlx` as the first argument to also emit the storage
/// bridge. It is omitted here because this crate's `sqlx` feature carries no
/// dependency — it exists only so the emitted `#[cfg(feature = "sqlx")]` is a
/// recognized value — so the bridge cannot expand in a doctest of this crate.
///
/// It **injects** `strum`'s `AsRefStr`/`Display`/`EnumString`/`IntoStaticStr` and the
/// `#[strum(parse_err_ty, parse_err_fn)]` pair, and **generates** the named parse error,
/// its parse fn, `Serialize`/`Deserialize`, and — with `sqlx` — the storage bridge. The
/// author keeps the non-uniform derives (`VariantArray`, `EnumMessage`, `Default`, …) and
/// `serialize_all`. `strum` does all the actual token/`Display`/`FromStr` work.
///
/// # It must be the item's first *active* attribute
///
/// An attribute macro only receives the attributes written **below** it; anything above
/// has already been expanded and stripped. So a uniform derive written above this
/// attribute is invisible here, gets injected a second time, and fails to compile with
/// `E0119` (conflicting implementations) or `E0592` (duplicate definitions). Put
/// `#[text_enum(…)]` first and that cannot happen.
///
/// A `///` doc comment above it is fine, and is how the adopting enums are written: doc
/// attributes are inert, so they neither expand nor collide — they simply stack onto the
/// enum along with the injected derives.
///
/// # The adopting crate must depend on `strum` and `serde`
///
/// The injected derives are emitted as `::strum::…` and the serde bridge as `::serde::…`,
/// so both must be dependencies of the crate under exactly those names. The serde bridge
/// is unconditional — there is no opt-out — because every closed string enum in this repo
/// either crosses the wire already or is one field away from doing so. Without the
/// dependencies the error is "cannot find derive macro in this scope" or an unresolved
/// `::serde` path, neither of which points back here.
///
/// The generated error type is always `pub`, regardless of the enum's own visibility: it
/// is registered by name across crate boundaries (`host`'s `validation_from!`), so a
/// private one would be useless.
#[proc_macro_attribute]
pub fn text_enum(attr: TokenStream, item: TokenStream) -> TokenStream {
    let item = proc_macro2::TokenStream::from(item);
    text_enum::expand(attr.into(), &item).into()
}

/// The field shapes a newtype derive will accept.
#[derive(Clone, Copy)]
pub(crate) enum NewtypeShape {
    /// Exactly one field, and no generics. `IdNewtype`, `NumNewtype`, `SqlxBridge`.
    Plain,
    /// One data field, optionally followed by a `PhantomData<_>` marker field, with
    /// generics permitted. `StrNewtype` only (#875).
    PhantomTagged,
}

/// Validates that `input` is a single-field tuple struct (`struct X(T)`) — the shape every
/// newtype derive requires — returning its **data field** (the first one), or a spanned
/// error (rendered as `compile_error!`) otherwise. `macro_name`/`example` shape the
/// diagnostic.
///
/// [`NewtypeShape::Plain`] additionally rejects generics, rather than letting them be
/// silently mis-handled: those derives emit `impl … for #name` with no
/// `impl_generics`/`where_clause`, so a generic newtype would otherwise produce a confusing
/// "missing generics" error at the user's site instead of this clear one.
///
/// [`NewtypeShape::PhantomTagged`] permits generics *and* a trailing `PhantomData<_>`
/// field, because `StrNewtype` threads the user's generics through every impl it emits
/// (#875). The marker field is not the derive's business — nothing it emits constructs
/// `Self`, so it never has to name it.
pub(crate) fn require_newtype_shape<'a>(
    input: &'a DeriveInput,
    shape: NewtypeShape,
    macro_name: &str,
    example: &str,
) -> syn::Result<&'a syn::Field> {
    let generics_ok = match shape {
        NewtypeShape::Plain => {
            input.generics.params.is_empty() && input.generics.where_clause.is_none()
        }
        NewtypeShape::PhantomTagged => true,
    };
    let data_field = match &input.data {
        Data::Struct(s) => match &s.fields {
            Fields::Unnamed(f) if f.unnamed.len() == 1 => Some(&f.unnamed[0]),
            // The second field is admitted only under `PhantomTagged`, and only if it
            // really is a `PhantomData<_>`: an accidental second data field must still be
            // the clear shape error, not a silently ignored one.
            Fields::Unnamed(f)
                if matches!(shape, NewtypeShape::PhantomTagged)
                    && f.unnamed.len() == 2
                    && is_phantom_data(&f.unnamed[1].ty) =>
            {
                Some(&f.unnamed[0])
            }
            _ => None,
        },
        _ => None,
    };
    match (data_field, generics_ok) {
        (Some(field), true) => Ok(field),
        _ => Err(syn::Error::new_spanned(
            input,
            match shape {
                NewtypeShape::Plain => format!(
                    "{macro_name} requires a non-generic single-field tuple struct like `{example}`"
                ),
                NewtypeShape::PhantomTagged => format!(
                    "{macro_name} requires a single-field tuple struct like `{example}`, \
                     optionally with a trailing `PhantomData<_>` marker field"
                ),
            },
        )),
    }
}

/// Whether `ty` names `PhantomData` — keyed on the last path segment, so `PhantomData<T>`,
/// `std::marker::PhantomData<T>`, and any alias-free spelling in between all match.
fn is_phantom_data(ty: &syn::Type) -> bool {
    matches!(
        ty,
        syn::Type::Path(p) if p.path.segments.last().is_some_and(|s| s.ident == "PhantomData"),
    )
}

/// The user's generics with `extra` prepended as the first parameter — the merge an impl
/// that introduces a parameter of its own needs (serde's `'de`, the sqlx bridge's `'q`/`'r`).
/// Prepended rather than pushed because Rust requires lifetime parameters to precede type
/// parameters, and every parameter threaded in this way is a lifetime.
pub(crate) fn with_leading_param(base: &syn::Generics, extra: syn::GenericParam) -> syn::Generics {
    let mut merged = base.clone();
    merged.params.insert(0, extra);
    merged
}

/// Validates that `input` is a **non-generic** enum whose variants are all unit variants —
/// the shape `#[text_enum]` requires, since every variant must map to exactly one token.
/// Mirrors [`require_newtype_shape`], including its rejection of generics: the emitted
/// impls carry no `impl_generics`/`where_clause`, so a generic enum would otherwise fail
/// confusingly at the user's site instead of clearly here.
pub(crate) fn require_enum_shape(
    input: &DeriveInput,
    macro_name: &str,
    example: &str,
) -> syn::Result<()> {
    let unit_enum = matches!(
        &input.data,
        Data::Enum(e) if e.variants.iter().all(|v| matches!(v.fields, Fields::Unit)),
    );
    let non_generic = input.generics.params.is_empty() && input.generics.where_clause.is_none();
    if unit_enum && non_generic {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(
            input,
            format!(
                "{macro_name} requires a non-generic enum with only unit variants like `{example}`"
            ),
        ))
    }
}

/// The **ordering half** of the ADR-0063 trailer (#761), shared by all three newtype
/// derives: `PartialOrd` + `Ord` delegating to the wrapped value. The emitted code is
/// identical for a `String` and an integer inner, so there is one copy rather than three.
///
/// Two details are load-bearing and easy to "simplify" wrongly:
///
/// - `partial_cmp` is written as `Some(self.cmp(other))` — clippy's canonical form for
///   `non_canonical_partial_ord_impl`. It resolves to `<#name as Ord>::cmp`, not `str`'s.
///   The probe walks the deref chain `&#name` → `#name` → `str`, and at the `#name` step
///   autoref finds a method taking `&#name`, which is our own `Ord::cmp`. It stops there,
///   so a `str`-backed newtype never reaches `str`'s impl — and never recurses.
/// - `cmp` delegates to `self.0`, the wrapped value, **not** to a `str` view. That is what
///   keeps the order consistent with the derived `PartialEq` and with `Borrow<str>`, whose
///   contract requires `Ord`/`Eq`/`Hash` to agree with the borrowed form.
///
/// Neither impl introduces a parameter of its own, so `generics` splits straight through
/// (`IdNewtype`/`NumNewtype` pass an empty one; `StrNewtype` may pass a phantom tag's).
pub(crate) fn ord_impls(name: &syn::Ident, generics: &syn::Generics) -> proc_macro2::TokenStream {
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    quote::quote! {
        #[automatically_derived]
        impl #impl_generics ::core::cmp::PartialOrd for #name #ty_generics #where_clause {
            fn partial_cmp(&self, other: &Self) -> ::core::option::Option<::core::cmp::Ordering> {
                ::core::option::Option::Some(self.cmp(other))
            }
        }

        #[automatically_derived]
        impl #impl_generics ::core::cmp::Ord for #name #ty_generics #where_clause {
            fn cmp(&self, other: &Self) -> ::core::cmp::Ordering {
                ::core::cmp::Ord::cmp(&self.0, &other.0)
            }
        }
    }
}

// The derives' *error* paths (wrong shape, unknown `str_newtype` option) can only be
// reached through a compile error, which the `compile_fail` doctests exercise — but
// doctest compilation is invisible to coverage instrumentation. These unit tests drive
// the same branches at runtime by calling the codegen entry points directly with
// malformed input and asserting a `compile_error!` is emitted.
#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn require_newtype_shape_rejects_named_struct() {
        let input: DeriveInput = parse_quote! { struct X { a: String } };
        assert!(
            require_newtype_shape(
                &input,
                NewtypeShape::PhantomTagged,
                "StrNewtype",
                "struct X(String)"
            )
            .is_err()
        );
    }

    #[test]
    fn require_newtype_shape_plain_rejects_generic_struct() {
        let input: DeriveInput = parse_quote! { struct X<T>(T); };
        assert!(
            require_newtype_shape(&input, NewtypeShape::Plain, "IdNewtype", "struct X(i64)")
                .is_err()
        );
    }

    #[test]
    fn require_newtype_shape_plain_rejects_a_where_clause() {
        // The `where`-only spelling of the same rejection: `params` is empty here, so only
        // the `where_clause` half of the guard can fire.
        let input: DeriveInput = parse_quote! { struct X(i64) where i64: Copy; };
        assert!(
            require_newtype_shape(&input, NewtypeShape::Plain, "IdNewtype", "struct X(i64)")
                .is_err()
        );
    }

    #[test]
    fn require_newtype_shape_accepts_tuple_struct() {
        let input: DeriveInput = parse_quote! { struct X(String); };
        let field = require_newtype_shape(
            &input,
            NewtypeShape::PhantomTagged,
            "StrNewtype",
            "struct X(String)",
        )
        .expect("a plain tuple struct is accepted");
        assert_eq!(quote::quote!(#field).to_string(), "String");
    }

    #[test]
    fn require_newtype_shape_phantom_tagged_accepts_a_generic_marker_struct() {
        let input: DeriveInput = parse_quote! {
            struct X<T: Role>(String, PhantomData<fn() -> T>);
        };
        let field = require_newtype_shape(
            &input,
            NewtypeShape::PhantomTagged,
            "StrNewtype",
            "struct X(String)",
        )
        .expect("the phantom-tagged shape is accepted");
        // The *data* field is returned, never the marker.
        assert_eq!(quote::quote!(#field).to_string(), "String");
    }

    #[test]
    fn require_newtype_shape_plain_rejects_the_phantom_tagged_shape() {
        let input: DeriveInput = parse_quote! {
            struct X<T: Role>(i64, PhantomData<fn() -> T>);
        };
        assert!(
            require_newtype_shape(&input, NewtypeShape::Plain, "IdNewtype", "struct X(i64)")
                .is_err()
        );
    }

    #[test]
    fn require_newtype_shape_phantom_tagged_rejects_a_second_data_field() {
        // The relaxation admits a *marker*, not a second value: `PhantomData` is the whole
        // admission criterion, so a two-field struct is still the clear shape error.
        let input: DeriveInput = parse_quote! { struct X(String, String); };
        assert!(
            require_newtype_shape(
                &input,
                NewtypeShape::PhantomTagged,
                "StrNewtype",
                "struct X(String)"
            )
            .is_err()
        );
    }

    #[test]
    fn require_newtype_shape_names_the_macro_in_both_shapes() {
        let named: DeriveInput = parse_quote! { struct X { a: String } };
        for (shape, macro_name) in [
            (NewtypeShape::Plain, "IdNewtype"),
            (NewtypeShape::PhantomTagged, "StrNewtype"),
        ] {
            // `syn::Field` is not `Debug`, so `expect_err` is unavailable here.
            let Err(err) = require_newtype_shape(&named, shape, macro_name, "struct X(String)")
            else {
                // cov:ignore-start unreachable: this is the test's own assertion-failure
                // path, reached only if the shape check wrongly accepts named fields
                panic!("a named-field struct is rejected under either shape")
                // cov:ignore-stop
            };
            assert!(
                err.to_string().contains(macro_name),
                "the diagnostic must name the macro, got: {err}"
            );
        }
    }

    /// Asserts the rejection *and* that its message names the macro — deliberately
    /// stronger than `require_newtype_shape`'s tests above, which assert only `is_err()`
    /// and so would pass on an anonymous diagnostic.
    fn assert_rejected_naming_macro(input: &DeriveInput) {
        let err = require_enum_shape(input, "text_enum", "enum X { A }")
            .expect_err("this shape must be rejected");
        assert!(
            err.to_string().contains("text_enum"),
            "the diagnostic must name the macro, got: {err}"
        );
    }

    #[test]
    fn require_enum_shape_rejects_a_struct() {
        assert_rejected_naming_macro(&parse_quote! { struct S(String); });
    }

    #[test]
    fn require_enum_shape_rejects_a_non_unit_variant() {
        assert_rejected_naming_macro(&parse_quote! { enum X { A(u8) } });
    }

    #[test]
    fn require_enum_shape_rejects_a_generic_enum() {
        assert_rejected_naming_macro(&parse_quote! { enum X<T> { A(T) } });
    }

    #[test]
    fn require_enum_shape_accepts_a_unit_enum() {
        let input: DeriveInput = parse_quote! { enum X { A, B } };
        assert!(require_enum_shape(&input, "text_enum", "enum X { A }").is_ok());
    }

    #[test]
    fn str_newtype_wrong_shape_emits_compile_error() {
        let input: DeriveInput = parse_quote! { struct X { a: String } };
        assert!(
            str_newtype::expand(&input)
                .to_string()
                .contains("compile_error")
        );
    }

    #[test]
    fn str_newtype_unknown_option_emits_compile_error() {
        let input: DeriveInput = parse_quote! {
            #[str_newtype(bogus)]
            struct X(String);
        };
        assert!(
            str_newtype::expand(&input)
                .to_string()
                .contains("compile_error")
        );
    }

    #[test]
    fn str_newtype_secret_selects_redacting_trailer() {
        // Drives `parse_opts`'s success path and the secret branch of `expand`: a redacting
        // Debug is emitted and the serde bridge is not.
        let input: DeriveInput = parse_quote! {
            #[str_newtype(secret)]
            struct X(String);
        };
        let out = str_newtype::expand(&input).to_string();
        assert!(out.contains("redacted"));
        assert!(!out.contains("Serialize"));
    }

    #[test]
    fn str_newtype_secret_serde_adds_the_serde_bridge_to_the_redacting_trailer() {
        // `secret, serde`: the redacting Debug AND the serde bridge, but not the full
        // trailer (no Display).
        let input: DeriveInput = parse_quote! {
            #[str_newtype(secret, serde)]
            struct X(String);
        };
        let out = str_newtype::expand(&input).to_string();
        assert!(out.contains("redacted"));
        assert!(out.contains("Serialize"));
        assert!(!out.contains("Display"));
    }

    #[test]
    fn str_newtype_serde_without_secret_emits_compile_error() {
        // A bare `serde` is invalid — the default trailer already has the serde bridge.
        let input: DeriveInput = parse_quote! {
            #[str_newtype(serde)]
            struct X(String);
        };
        assert!(
            str_newtype::expand(&input)
                .to_string()
                .contains("compile_error")
        );
    }

    #[test]
    fn str_newtype_infallible_emits_from_string_serde_and_omits_fallible_door() {
        // Infallible mode: Display/AsRef/Deref/Serialize/Deserialize present; the
        // fallible door (TryFrom / FromStr routing) is absent — the author writes
        // From<String> and Deserialize routes through it.
        let input: DeriveInput = parse_quote! {
            #[str_newtype(infallible)]
            struct X(String);
        };
        let out = str_newtype::expand(&input).to_string();
        assert!(out.contains("Display"));
        assert!(out.contains("AsRef"));
        assert!(out.contains("Deref"));
        assert!(out.contains("Serialize"));
        assert!(out.contains("Deserialize"));
        // The fallible door (TryFrom / FromStr routing) is absent — the author writes
        // From<String> and Deserialize routes through it.
        assert!(!out.contains("TryFrom"));
        assert!(!out.contains("FromStr"));
    }

    #[test]
    fn str_newtype_infallible_with_secret_emits_compile_error() {
        let input: DeriveInput = parse_quote! {
            #[str_newtype(infallible, secret)]
            struct X(String);
        };
        assert!(
            str_newtype::expand(&input)
                .to_string()
                .contains("compile_error")
        );
    }

    #[test]
    fn str_newtype_infallible_with_serde_emits_compile_error() {
        let input: DeriveInput = parse_quote! {
            #[str_newtype(infallible, serde)]
            struct X(String);
        };
        assert!(
            str_newtype::expand(&input)
                .to_string()
                .contains("compile_error")
        );
    }

    /// The generic form's headers, at the token level (#875). The integration fixture
    /// `macros/tests/str_newtype.rs` proves the emitted code *works*; this pins the two
    /// merged-generics spellings that a plain `split_for_impl` would get wrong — the
    /// `Deserialize` lifetime must precede the type parameter, and `Self` must be spelled
    /// with its type arguments wherever it appears as a type.
    #[test]
    fn str_newtype_generic_threads_the_users_generics_through_every_impl() {
        let input: DeriveInput = parse_quote! {
            struct X<T: Role>(String, PhantomData<fn() -> T>);
        };
        let out = sqlx_bridge::tests::norm(&str_newtype::expand(&input));
        assert!(!out.contains("compile_error"), "{out}");
        assert!(out.contains("impl<T:Role>::core::fmt::DisplayforX<T>"));
        assert!(out.contains("impl<T:Role>::core::convert::From<X<T>>for::std::string::String"));
        assert!(out.contains("impl<T:Role>::core::cmp::OrdforX<T>"));
        assert!(
            out.contains("impl<'de,T:Role>::serde::Deserialize<'de>forX<T>"),
            "the `'de` must be merged in FRONT of the type parameter: {out}"
        );
        assert!(
            out.contains("<X<T>as::core::str::FromStr>::from_str"),
            "a qualified `Self` must carry its type arguments: {out}"
        );
    }

    #[test]
    fn str_newtype_rejects_a_second_non_phantom_field() {
        let input: DeriveInput = parse_quote! { struct X(String, String); };
        assert!(
            str_newtype::expand(&input)
                .to_string()
                .contains("compile_error")
        );
    }

    #[test]
    fn id_newtype_generic_emits_compile_error() {
        // The runtime counterpart of the `compile_fail` doctest on the derive: only
        // `StrNewtype` learned the phantom-tagged shape (#875).
        let input: DeriveInput = parse_quote! {
            struct X<T: Role>(i64, PhantomData<fn() -> T>);
        };
        assert!(
            id_newtype::expand(&input)
                .to_string()
                .contains("compile_error")
        );
    }

    #[test]
    fn num_newtype_generic_emits_compile_error() {
        let input: DeriveInput = parse_quote! {
            #[num_newtype(inner = u32)]
            struct X<T: Role>(u32, PhantomData<fn() -> T>);
        };
        assert!(
            num_newtype::expand(&input)
                .to_string()
                .contains("compile_error")
        );
    }

    #[test]
    fn id_newtype_wrong_shape_emits_compile_error() {
        let input: DeriveInput = parse_quote! { struct X { a: i64 } };
        assert!(
            id_newtype::expand(&input)
                .to_string()
                .contains("compile_error")
        );
    }

    #[test]
    fn num_newtype_wrong_shape_emits_compile_error() {
        let input: DeriveInput = parse_quote! {
            #[num_newtype(inner = u32)]
            struct X { a: u32 }
        };
        assert!(
            num_newtype::expand(&input)
                .to_string()
                .contains("compile_error")
        );
    }

    #[test]
    fn num_newtype_missing_inner_emits_compile_error() {
        let input: DeriveInput = parse_quote! {
            #[num_newtype(min = 1)]
            struct X(u32);
        };
        assert!(
            num_newtype::expand(&input)
                .to_string()
                .contains("compile_error")
        );
    }

    #[test]
    fn num_newtype_inner_type_mismatch_emits_compile_error() {
        // Field is `u32` but `inner = i64` — the declared inner disagrees with the field.
        let input: DeriveInput = parse_quote! {
            #[num_newtype(inner = i64, min = 1)]
            struct X(u32);
        };
        assert!(
            num_newtype::expand(&input)
                .to_string()
                .contains("compile_error")
        );
    }

    #[test]
    fn num_newtype_unknown_option_emits_compile_error() {
        let input: DeriveInput = parse_quote! {
            #[num_newtype(inner = u32, bogus = 1)]
            struct X(u32);
        };
        assert!(
            num_newtype::expand(&input)
                .to_string()
                .contains("compile_error")
        );
    }

    #[test]
    fn num_newtype_min_max_default_emit_full_trailer() {
        let input: DeriveInput = parse_quote! {
            #[num_newtype(inner = u32, min = 1, max = 100, default = 20)]
            struct X(u32);
        };
        let out = num_newtype::expand(&input).to_string();
        assert!(!out.contains("compile_error"));
        assert!(out.contains("FromStr"));
        // The validating `TryFrom<inner>` door (the checked integer constructor) is emitted,
        // reusing the same bound guards as `FromStr`.
        assert!(out.contains("TryFrom"));
        assert!(out.contains("Default"));
        assert!(out.contains("Serialize"));
        assert!(out.contains("Deserialize"));
        // Both bound checks present — `v < min` and `v > max` (the `v` prefix is unique to
        // the generated checks; a bare `> ` also occurs in generics).
        assert!(out.contains("v < 1"));
        assert!(out.contains("v > 100"));
    }

    #[test]
    fn num_newtype_min_only_omits_max_check_and_default() {
        let input: DeriveInput = parse_quote! {
            #[num_newtype(inner = usize, min = 1)]
            struct X(usize);
        };
        let out = num_newtype::expand(&input).to_string();
        assert!(!out.contains("compile_error"));
        assert!(out.contains("v < 1"));
        // No `max` side (`v > `), no `default` impl.
        assert!(!out.contains("v > "));
        assert!(!out.contains("impl :: core :: default :: Default"));
    }

    #[test]
    fn num_newtype_error_message_overrides_generated() {
        let input: DeriveInput = parse_quote! {
            #[num_newtype(inner = u32, min = 1, error = "must be a whole number of at least 1")]
            struct X(u32);
        };
        let out = num_newtype::expand(&input).to_string();
        assert!(out.contains("must be a whole number of at least 1"));
    }

    #[test]
    fn num_newtype_max_only_emits_max_check_and_at_most_message() {
        let input: DeriveInput = parse_quote! {
            #[num_newtype(inner = u32, max = 100)]
            struct X(u32);
        };
        let out = num_newtype::expand(&input).to_string();
        assert!(!out.contains("compile_error"));
        assert!(out.contains("v > 100"));
        assert!(!out.contains("v < ")); // no `min` side
        assert!(out.contains("at most 100"));
    }

    #[test]
    fn num_newtype_no_bounds_generates_valid_integer_message() {
        let input: DeriveInput = parse_quote! {
            #[num_newtype(inner = u32)]
            struct X(u32);
        };
        let out = num_newtype::expand(&input).to_string();
        assert!(!out.contains("compile_error"));
        assert!(out.contains("a valid integer"));
        assert!(!out.contains("v < "));
        assert!(!out.contains("v > "));
    }

    /// True iff the emitted stream carries the three always-emitted sqlx bridge impls.
    /// Deliberately does not check `PgHasArrayType`, which is opt-in per caller (#891).
    fn has_sqlx_bridge(out: &str) -> bool {
        out.contains("sqlx :: Type")
            && out.contains("sqlx :: Encode")
            && out.contains("sqlx :: Decode")
    }

    #[test]
    fn str_newtype_default_emits_sqlx_bridge() {
        // Default (non-secret) type: the validating sqlx bridge is on, feature-gated,
        // and routes Decode through FromStr.
        let input: DeriveInput = parse_quote! {
            struct X(String);
        };
        let out = str_newtype::expand(&input).to_string();
        assert!(has_sqlx_bridge(&out));
        assert!(out.contains("cfg (feature = \"sqlx\")"));
        assert!(out.contains("from_str"));
    }

    #[test]
    fn str_newtype_no_sqlx_omits_the_bridge() {
        // `no_sqlx` opts a non-secret must-not-store type (RawToken) out of the bridge.
        let input: DeriveInput = parse_quote! {
            #[str_newtype(no_sqlx)]
            struct X(String);
        };
        let out = str_newtype::expand(&input).to_string();
        assert!(!has_sqlx_bridge(&out));
        // The rest of the default trailer is still there.
        assert!(out.contains("Serialize"));
    }

    #[test]
    fn str_newtype_secret_omits_the_bridge() {
        // A secret is bridge-less by default (must not be storable).
        let input: DeriveInput = parse_quote! {
            #[str_newtype(secret)]
            struct X(String);
        };
        let out = str_newtype::expand(&input).to_string();
        assert!(!has_sqlx_bridge(&out));
    }

    #[test]
    fn str_newtype_secret_sqlx_readds_the_bridge() {
        // `secret, sqlx`: the redacting trailer plus the validating sqlx bridge
        // (InviteCode — a stored secret).
        let input: DeriveInput = parse_quote! {
            #[str_newtype(secret, sqlx)]
            struct X(String);
        };
        let out = str_newtype::expand(&input).to_string();
        assert!(out.contains("redacted"));
        assert!(has_sqlx_bridge(&out));
        assert!(out.contains("from_str"));
    }

    #[test]
    fn str_newtype_infallible_emits_the_infallible_sqlx_bridge() {
        // Infallible types are stored: the bridge is on by default and Decode wraps via
        // From<String> (no FromStr).
        let input: DeriveInput = parse_quote! {
            #[str_newtype(infallible)]
            struct X(String);
        };
        let out = str_newtype::expand(&input).to_string();
        assert!(has_sqlx_bridge(&out));
        assert!(!out.contains("from_str"));
    }

    #[test]
    fn str_newtype_infallible_no_sqlx_omits_the_bridge() {
        let input: DeriveInput = parse_quote! {
            #[str_newtype(infallible, no_sqlx)]
            struct X(String);
        };
        let out = str_newtype::expand(&input).to_string();
        assert!(!has_sqlx_bridge(&out));
    }

    #[test]
    fn str_newtype_no_sqlx_with_secret_emits_compile_error() {
        // A secret is already bridge-less — `no_sqlx` is redundant/invalid.
        let input: DeriveInput = parse_quote! {
            #[str_newtype(secret, no_sqlx)]
            struct X(String);
        };
        assert!(
            str_newtype::expand(&input)
                .to_string()
                .contains("compile_error")
        );
    }

    #[test]
    fn str_newtype_no_sqlx_with_sqlx_emits_compile_error() {
        // `sqlx, no_sqlx` (no `secret`, so the `no_sqlx && secret` guard is skipped and
        // the `no_sqlx && sqlx` exclusivity arm fires).
        let input: DeriveInput = parse_quote! {
            #[str_newtype(sqlx, no_sqlx)]
            struct X(String);
        };
        assert!(
            str_newtype::expand(&input)
                .to_string()
                .contains("compile_error")
        );
    }

    // --- ordering (#761) ---------------------------------------------------------------
    // Assertions key on the emitted *method* names, never the trait names: "Ord" is a
    // substring of "PartialOrd", so `contains("Ord")` could never fail independently.

    #[test]
    fn str_newtype_emits_ordering_by_default() {
        let input: DeriveInput = parse_quote! { struct X(String); };
        let out = str_newtype::expand(&input).to_string();
        assert!(out.contains("fn partial_cmp"));
        assert!(out.contains("fn cmp"));
    }

    #[test]
    fn str_newtype_infallible_emits_ordering() {
        let input: DeriveInput = parse_quote! {
            #[str_newtype(infallible)]
            struct X(String);
        };
        assert!(
            str_newtype::expand(&input)
                .to_string()
                .contains("fn partial_cmp")
        );
    }

    #[test]
    fn str_newtype_no_ord_omits_ordering() {
        // The real discriminator for `no_ord`. The companion `compile_fail` doctest
        // cannot tell the two states apart, because an *unknown* option also fails to
        // compile — it would pass before this feature existed.
        let input: DeriveInput = parse_quote! {
            #[str_newtype(no_ord)]
            struct X(String);
        };
        let out = str_newtype::expand(&input).to_string();
        assert!(!out.contains("fn partial_cmp"));
        assert!(!out.contains("fn cmp"));
        // Only the ordering half is suppressed; the rest of the trailer stands — including
        // the sqlx bridge, which no integration fixture can assert (the `macros` crate
        // declares the `sqlx` feature with no deps, so nothing enables it there).
        assert!(out.contains("Display"));
        assert!(has_sqlx_bridge(&out));
    }

    #[test]
    fn str_newtype_secret_omits_ordering() {
        let input: DeriveInput = parse_quote! {
            #[str_newtype(secret)]
            struct X(String);
        };
        assert!(
            !str_newtype::expand(&input)
                .to_string()
                .contains("fn partial_cmp")
        );
    }

    #[test]
    fn str_newtype_no_ord_with_secret_emits_compile_error() {
        // A secret is already unordered — `no_ord` is redundant/invalid, mirroring the
        // `no_sqlx` + `secret` guard above. The message is asserted, not just the presence
        // of a `compile_error!`: six guards can fire here, and only one of them is right.
        let input: DeriveInput = parse_quote! {
            #[str_newtype(secret, no_ord)]
            struct X(String);
        };
        let out = str_newtype::expand(&input).to_string();
        assert!(out.contains("compile_error"));
        assert!(out.contains("already unordered"));
    }

    #[test]
    fn str_newtype_infallible_no_ord_is_accepted() {
        // An infallible newtype can lack `Eq` for the same reasons a default one can,
        // so the pair is legal (unlike `infallible, secret`).
        let input: DeriveInput = parse_quote! {
            #[str_newtype(infallible, no_ord)]
            struct X(String);
        };
        let out = str_newtype::expand(&input).to_string();
        assert!(!out.contains("compile_error"));
        assert!(!out.contains("fn partial_cmp"));
    }

    #[test]
    fn str_newtype_bare_sqlx_without_secret_emits_compile_error() {
        // Bare `sqlx` is only meaningful on a secret; non-secret types get the bridge
        // by default.
        let input: DeriveInput = parse_quote! {
            #[str_newtype(sqlx)]
            struct X(String);
        };
        assert!(
            str_newtype::expand(&input)
                .to_string()
                .contains("compile_error")
        );
    }

    #[test]
    fn id_newtype_emits_sqlx_bridge() {
        // Unconditional: every id is stored, so `IdNewtype` has no opt-out attribute
        // (unlike `StrNewtype`'s `no_sqlx`/`sqlx` controls).
        let input: DeriveInput = parse_quote! {
            struct X(i64);
        };
        let out = id_newtype::expand(&input).to_string();
        assert!(has_sqlx_bridge(&out));
        assert!(out.contains("cfg (feature = \"sqlx\")"));
    }

    #[test]
    fn id_newtype_sqlx_decode_is_an_infallible_wrap() {
        // An id has no invariant beyond "is an integer" (ADR-0063 §2), so `Decode`
        // wraps directly — it must NOT route through the bound-checking
        // `TryFrom<inner>` that `NumNewtype`'s bridge uses, which would make an id
        // decode fallible for no reason.
        let input: DeriveInput = parse_quote! {
            struct X(i64);
        };
        let out = id_newtype::expand(&input).to_string();
        assert!(out.contains("sqlx :: Decode"));
        // Absence assertions are only worth anything against a needle known to be
        // spelled correctly — a typo'd one is vacuously absent and never bites. This is
        // the same needle `num_newtype_emits_bound_checking_sqlx_bridge` asserts is
        // PRESENT, so the pair fails if either family adopts the other's `Decode`.
        assert!(
            !out.contains("try_from (v) ?"),
            "an id's Decode must not re-run a bound it does not have"
        );
    }

    #[test]
    fn num_newtype_emits_bound_checking_sqlx_bridge() {
        // A numeric value DOES have an invariant, so `Decode` re-runs it via the
        // generated `TryFrom<inner>` — otherwise the column would be a hole in a
        // bound the serde bridge already enforces on the wire.
        //
        // The assertion must name the bridge's CALL, not just `try_from`: the derive
        // always emits the `TryFrom<inner>` door itself (`try_from_inner_impl`), whose
        // signature stringifies as `try_from (v : u32)`. So a bare `contains("try_from")`
        // holds even when the bridge's `Decode` is an infallible wrap that skips the
        // bound — i.e. it passes on the very regression it exists to catch. Only the
        // bridge applies it to the decoded local, as `try_from (v) ?`.
        let input: DeriveInput = parse_quote! {
            #[num_newtype(inner = u32, min = 1, max = 50, default = 50)]
            struct X(u32);
        };
        let out = num_newtype::expand(&input).to_string();
        assert!(has_sqlx_bridge(&out));
        assert!(out.contains("cfg (feature = \"sqlx\")"));
        assert!(
            out.contains("try_from (v) ?"),
            "bridge Decode must re-run the bound via TryFrom on the decoded value"
        );
    }

    #[test]
    fn num_newtype_sqlx_bridge_uses_the_declared_inner_type() {
        // The bridge is parameterized on `inner`, not hardcoded to i64. The inner MUST
        // be a non-i64 type for this to discriminate: with `inner = i64` the assertion
        // holds whether the bridge is parameterized or hardcoded, so the test would
        // pass on the very bug it exists to catch. `usize` is also the wrong choice —
        // it would not distinguish a hardcoded `usize` either, but `u32` here plus the
        // `i64` absence below pins the delegation to the declared type.
        let input: DeriveInput = parse_quote! {
            #[num_newtype(inner = u32, min = 1, default = 1)]
            struct X(u32);
        };
        let out = num_newtype::expand(&input).to_string();
        assert!(
            out.contains("u32 as :: sqlx :: Type"),
            "bridge must delegate to the declared inner type"
        );
        assert!(
            !out.contains("i64 as :: sqlx :: Type"),
            "bridge must not fall back to a hardcoded i64"
        );
    }

    #[test]
    fn num_newtype_clamp_emits_bounds_and_clamped_constructor() {
        let input: DeriveInput = parse_quote! {
            #[num_newtype(inner = u32, min = 1, max = 50, default = 50, clamp)]
            struct X(u32);
        };
        let out = num_newtype::expand(&input).to_string();
        assert!(!out.contains("compile_error"));
        assert!(out.contains("const MIN"));
        assert!(out.contains("const MAX"));
        assert!(out.contains("fn clamped"));
    }

    #[test]
    fn num_newtype_clamp_without_both_bounds_emits_compile_error() {
        let input: DeriveInput = parse_quote! {
            #[num_newtype(inner = u32, min = 1, clamp)]
            struct X(u32);
        };
        assert!(
            num_newtype::expand(&input)
                .to_string()
                .contains("compile_error")
        );
    }

    #[test]
    fn num_newtype_min_greater_than_max_emits_compile_error() {
        let input: DeriveInput = parse_quote! {
            #[num_newtype(inner = u32, min = 50, max = 1)]
            struct X(u32);
        };
        assert!(
            num_newtype::expand(&input)
                .to_string()
                .contains("compile_error")
        );
    }

    #[test]
    fn num_newtype_without_clamp_omits_clamped_constructor() {
        let input: DeriveInput = parse_quote! {
            #[num_newtype(inner = u32, min = 1, max = 50)]
            struct X(u32);
        };
        let out = num_newtype::expand(&input).to_string();
        assert!(!out.contains("fn clamped"));
        assert!(!out.contains("const MAX"));
    }
}
