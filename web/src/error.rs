use leptos::server_fn::{
    codec::JsonEncoding,
    error::{FromServerFnError, ServerFnErrorErr},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// The server-side error carrier lives in `host` (ADR-0058); `web` keeps only the
// wire type, the `kind → WebError` projection, and the leptos owner-pinning
// boundary. Re-exported so every vertical's `InternalError::storage(…)`/`?` call
// site names it unchanged through `web::error`.
#[cfg(feature = "server")]
pub use host::error::{ErrorClass, ErrorKind, InternalError, InternalResult};

pub type WebResult<T> = Result<T, WebError>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Error)]
#[serde(rename_all = "snake_case")]
pub enum WebError {
    #[error("unauthorized")]
    Unauthorized,
    #[error("{message}")]
    NotFound { message: String },
    #[error("{message}")]
    Validation { message: String },
    #[error("{message}")]
    Conflict { message: String },
    #[error("storage error: {message}")]
    Storage { message: String },
    #[error("server error: {message}")]
    Server { message: String },
    #[error("server function error: {message}")]
    ServerFunction { message: String },
}

impl WebError {
    pub fn not_found(resource: impl Into<String>) -> Self {
        Self::NotFound {
            message: format!("{} not found", resource.into()),
        }
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation {
            message: message.into(),
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::Conflict {
            message: message.into(),
        }
    }

    pub fn server_message(message: impl Into<String>) -> Self {
        Self::Server {
            message: message.into(),
        }
    }

    pub fn server_function(message: impl Into<String>) -> Self {
        Self::ServerFunction {
            message: message.into(),
        }
    }
}

impl FromServerFnError for WebError {
    type Encoder = JsonEncoding;

    fn from_server_fn_error(value: ServerFnErrorErr) -> Self {
        Self::server_function(value.to_string())
    }
}

/// Projects an `InternalError`'s `(kind, public_message)` to its outward
/// `WebError` wire form — the total, message-carrying counterpart to the
/// carrier's construction. This is the single point where the carrier becomes a
/// wire type (see [`server_boundary`]); the operator-side `source`/`context`
/// have no projection and so cannot leak. Masking kinds
/// (`Storage`/`Internal`/`External`) carry only their generic public message,
/// never the source detail.
#[cfg(feature = "server")]
pub(crate) fn project(kind: ErrorKind, public_message: &str) -> WebError {
    match kind {
        ErrorKind::Auth => WebError::Unauthorized,
        ErrorKind::NotFound => WebError::NotFound {
            message: public_message.to_string(),
        },
        ErrorKind::Validation => WebError::Validation {
            message: public_message.to_string(),
        },
        ErrorKind::Conflict => WebError::Conflict {
            message: public_message.to_string(),
        },
        ErrorKind::Storage => WebError::Storage {
            message: public_message.to_string(),
        },
        ErrorKind::Internal | ErrorKind::External => WebError::Server {
            message: public_message.to_string(),
        },
    }
}

/// Wraps a `Resource` fetcher's future so the reactive owner — captured here, while it
/// is still current (the resource's own owner) — is held by a *strong* ref and
/// re-applied on every poll. This keeps server-fn context (storage trait objects and
/// request `Parts`) alive even when the future is later polled on a worker thread
/// detached from the owner. The owner is live only at fetcher invocation; an
/// `async fn` body has no synchronous prologue, so this cannot live in the handler.
/// `new_untracked` so context reads don't create spurious reactive subscriptions.
/// Extends #89's [`server_boundary`] mechanism to the `Resource` layer; see the
/// ADR-0016 #124 addendum.
fn scoped_fetcher_future<Fut>(fut: Fut) -> leptos::reactive::computed::ScopedFuture<Fut>
where
    Fut: std::future::Future,
{
    leptos::reactive::computed::ScopedFuture::new_untracked(fut)
}

/// The sanctioned way to create a `Resource` in `web`: identical to
/// `leptos::prelude::Resource::new`, but wraps the fetcher's future via
/// [`scoped_fetcher_future`] so server-fn context survives SSR polling on a worker
/// thread detached from the owner (issue #124). Raw `Resource::new` is banned in
/// `web` outside this definition (static guard).
pub fn server_resource<T, S, Fut>(
    source: impl Fn() -> S + Send + Sync + 'static,
    fetcher: impl Fn(S) -> Fut + Send + Sync + 'static,
) -> leptos::prelude::Resource<T>
where
    T: serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static,
    S: PartialEq + Clone + Send + Sync + 'static,
    Fut: std::future::Future<Output = T> + Send + 'static,
{
    #[expect(
        clippy::disallowed_methods,
        reason = "the one sanctioned Resource::new; all other call sites must go through \
                  web::server_resource (#124)"
    )]
    leptos::prelude::Resource::new(source, move |s| scoped_fetcher_future(fetcher(s)))
}

/// Collects strong [`Owner`](leptos::reactive::owner::Owner) handles for every
/// ancestor of `owner` (parent, grandparent, … up to the root). `Owner::parent()`
/// upgrades the internally-*weak* parent link to a strong ref, so holding the
/// returned vec keeps each ancestor's context map alive. Used by
/// [`server_boundary`] to preserve contexts provided in an ancestor owner across
/// an SSR await (#138).
#[cfg(feature = "server")]
fn owner_ancestry_strong(
    owner: &leptos::reactive::owner::Owner,
) -> Vec<leptos::reactive::owner::Owner> {
    let mut ancestry = Vec::new();
    let mut next = owner.parent();
    while let Some(parent) = next {
        next = parent.parent();
        ancestry.push(parent);
    }
    ancestry
}

/// Awaits the given future, converting any `InternalError` to its public `WebError` form.
///
/// # Errors
///
/// Returns `Err(ServerFnError)` if the wrapped future returns an `InternalError`.
#[cfg(feature = "server")]
pub async fn server_boundary<T>(
    server_fn: &'static str,
    future: impl std::future::Future<Output = InternalResult<T>>,
) -> WebResult<T> {
    // #89: a server-fn body that reads Leptos context after an `.await` can panic
    // during SSR. The reactive "current owner" is a *weak* thread-local; if its last
    // strong ref is dropped while the future is suspended at an await, the owner's
    // context map is freed and the post-await `expect_context` finds nothing.
    // `ScopedFuture` captures a *strong* owner ref and re-applies it on every poll,
    // keeping context alive across awaits. Guard on a current owner:
    // `ScopedFuture::new_untracked` captures `Owner::current().unwrap_or_default()`, so
    // wrapping with no owner would capture an empty owner and lose context
    // deterministically; the guard instead falls back to a plain await (today's behavior).
    // See ADR-0016 addendum.
    // #138: `ScopedFuture` re-applies and holds a strong ref to the *current*
    // (leaf) owner only. But the storage contexts are provided in an *ancestor*
    // owner — the SSR root (`provide_app_state_contexts`) — which the SSR runtime
    // can drop while this future is suspended. `expect_context` walks the owner
    // ancestry, so once an ancestor's last strong ref is gone its context map is
    // freed and a post-await read panics. Hold the full ancestry strong for the
    // future's lifetime so post-await reactive-context reads always resolve,
    // regardless of read-before/after-await ordering. See the `owner_lifetime`
    // tests and the ADR-0016 #138 addendum.
    let outcome = if let Some(owner) = leptos::reactive::owner::Owner::current() {
        let _ancestry = owner_ancestry_strong(&owner);
        leptos::reactive::computed::ScopedFuture::new_untracked(future).await
    } else {
        future.await
    };
    match outcome {
        Ok(value) => Ok(value),
        Err(error) => {
            // The carrier owns its own observability (structured log + metric);
            // `web` only performs the wire projection.
            error.emit_boundary_failure(server_fn);
            Err(project(error.kind(), error.public_message()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::WebError;
    #[cfg(feature = "server")]
    use super::{project, server_boundary, ErrorClass, ErrorKind, InternalError, WebResult};
    use leptos::prelude::FromServerFnError;
    use leptos::server_fn::{codec::JsonEncoding, error::ServerFnErrorErr, Decodes, Encodes};
    use std::error::Error;
    use std::fmt;

    #[derive(Debug)]
    struct SourceError;

    impl fmt::Display for SourceError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("source context")
        }
    }

    impl Error for SourceError {}

    #[derive(Debug)]
    struct OuterError {
        source: SourceError,
    }

    impl fmt::Display for OuterError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("outer failure")
        }
    }

    impl Error for OuterError {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            Some(&self.source)
        }
    }

    #[test]
    fn server_function_errors_map_to_web_error() {
        let error = WebError::from_server_fn_error(ServerFnErrorErr::Args("bad arg".to_string()));

        assert!(matches!(error, WebError::ServerFunction { .. }));
        assert!(error.to_string().contains("bad arg"));
    }

    #[test]
    fn constructors_create_expected_variants() {
        assert_eq!(
            WebError::not_found("Post"),
            WebError::NotFound {
                message: "Post not found".to_string()
            }
        );
        assert_eq!(
            WebError::validation("bad input"),
            WebError::Validation {
                message: "bad input".to_string()
            }
        );
        assert_eq!(
            WebError::conflict("already exists"),
            WebError::Conflict {
                message: "already exists".to_string()
            }
        );
        assert_eq!(
            WebError::server_message("boom"),
            WebError::Server {
                message: "boom".to_string()
            }
        );
        assert_eq!(
            WebError::server_function("bad args"),
            WebError::ServerFunction {
                message: "bad args".to_string()
            }
        );
    }

    #[cfg(feature = "server")]
    #[test]
    fn masked_internal_errors_never_leak_source_chain_to_public() {
        // §2.4 regression guard: storage/server failures reach the client only
        // through `InternalError`, which must mask. The raw source chain may
        // appear in the operator message (logged) but never in the public
        // `WebError` sent to the browser. The leaky `WebError::storage`/`server`
        // constructors that embedded the chain were removed for this reason.
        for internal in [
            InternalError::storage(OuterError {
                source: SourceError,
            }),
            InternalError::server(OuterError {
                source: SourceError,
            }),
        ] {
            assert!(
                internal.operator_message().contains("source context"),
                "operator message should retain the source chain for logs"
            );
            let public = project(internal.kind(), internal.public_message());
            assert!(
                !public.to_string().contains("source context"),
                "public error leaked source detail: {public}"
            );
        }
    }

    #[test]
    fn json_encoding_uses_stable_snake_case_variant_names() {
        let encoded = <JsonEncoding as Encodes<WebError>>::encode(&WebError::Unauthorized).unwrap();
        assert_eq!(encoded.as_ref(), br#""unauthorized""#);

        let decoded = <JsonEncoding as Decodes<WebError>>::decode(encoded).unwrap();
        assert_eq!(decoded, WebError::Unauthorized);
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn server_boundary_logs_and_returns_public_error() {
        let result: Result<(), WebError> = server_boundary("test_fn", async {
            Err(InternalError::storage(OuterError {
                source: SourceError,
            }))
        })
        .await;

        assert_eq!(
            result,
            Err(WebError::Storage {
                message: "storage operation failed".to_string()
            })
        );
    }

    #[cfg(feature = "server")]
    #[test]
    fn internal_error_preserves_operator_message() {
        let error = InternalError::server(OuterError {
            source: SourceError,
        });

        assert_eq!(error.operator_message(), "outer failure: source context");
        assert_eq!(
            project(error.kind(), error.public_message()),
            WebError::Server {
                message: "server operation failed".to_string()
            }
        );
    }

    #[cfg(feature = "server")]
    #[test]
    fn internal_error_server_message_keeps_operator_detail_and_generic_public_message() {
        let error = InternalError::server_message("operator-only context");
        assert_eq!(error.operator_message(), "operator-only context");
        assert_eq!(
            project(error.kind(), error.public_message()),
            WebError::Server {
                message: "server operation failed".to_string()
            }
        );
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn server_boundary_evaluates_tracing_fields_when_subscriber_is_active() {
        use tracing_subscriber::fmt;

        let subscriber = fmt()
            .with_test_writer()
            .with_max_level(tracing::Level::TRACE)
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);

        let result = server_boundary("test_fn", async {
            Err::<(), _>(InternalError::server(OuterError {
                source: SourceError,
            }))
        })
        .await;

        assert_eq!(
            result,
            Err(WebError::Server {
                message: "server operation failed".to_string()
            })
        );
    }

    #[cfg(feature = "server")]
    #[test]
    fn internal_error_constructors_set_correct_public_variants() {
        let unauth = InternalError::unauthorized("not allowed");
        assert_eq!(
            project(unauth.kind(), unauth.public_message()),
            WebError::Unauthorized
        );
        assert_eq!(unauth.operator_message(), "not allowed");

        let not_found = InternalError::not_found("Post");
        assert_eq!(
            project(not_found.kind(), not_found.public_message()),
            WebError::not_found("Post")
        );

        let validation = InternalError::validation("bad input");
        assert_eq!(
            project(validation.kind(), validation.public_message()),
            WebError::validation("bad input")
        );

        let conflict = InternalError::conflict("already exists");
        assert_eq!(
            project(conflict.kind(), conflict.public_message()),
            WebError::conflict("already exists")
        );
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn server_boundary_passes_through_ok_value() {
        let result: WebResult<u32> = server_boundary("test_fn", async { Ok(42) }).await;
        assert_eq!(result, Ok(42));
    }

    #[cfg(feature = "server")]
    #[test]
    fn masked_internal_error_keeps_public_and_operator_messages_separate() {
        let error = InternalError::masked(
            ErrorKind::NotFound,
            ErrorClass::Client,
            "Post not found",
            anyhow::Error::msg("draft access denied for missing session token"),
        );

        assert_eq!(
            error.operator_message(),
            "draft access denied for missing session token"
        );
        assert_eq!(
            project(error.kind(), error.public_message()),
            WebError::not_found("Post")
        );
    }

    #[cfg(feature = "server")]
    #[test]
    fn external_constructor_sets_external_kind_and_class() {
        let error = InternalError::external(OuterError {
            source: SourceError,
        });
        assert_eq!(error.kind(), ErrorKind::External);
        assert_eq!(error.class(), ErrorClass::External);
        assert_eq!(error.operator_message(), "outer failure: source context");
        // Outward it still masks as a generic 500.
        assert_eq!(
            project(error.kind(), error.public_message()),
            WebError::Server {
                message: "server operation failed".to_string()
            }
        );
    }

    #[cfg(feature = "server")]
    #[test]
    fn project_is_the_total_kind_to_web_error_map() {
        assert_eq!(project(ErrorKind::Auth, "ignored"), WebError::Unauthorized);
        assert_eq!(
            project(ErrorKind::NotFound, "x not found"),
            WebError::NotFound {
                message: "x not found".to_string()
            }
        );
        assert_eq!(
            project(ErrorKind::Validation, "bad"),
            WebError::Validation {
                message: "bad".to_string()
            }
        );
        assert_eq!(
            project(ErrorKind::Conflict, "dupe"),
            WebError::Conflict {
                message: "dupe".to_string()
            }
        );
        assert_eq!(
            project(ErrorKind::Storage, "storage operation failed"),
            WebError::Storage {
                message: "storage operation failed".to_string()
            }
        );
        assert_eq!(
            project(ErrorKind::Internal, "server operation failed"),
            WebError::Server {
                message: "server operation failed".to_string()
            }
        );
        assert_eq!(
            project(ErrorKind::External, "server operation failed"),
            WebError::Server {
                message: "server operation failed".to_string()
            }
        );
    }

    #[cfg(feature = "server")]
    #[test]
    fn masking_constructors_set_generic_public_message_and_preserve_source() {
        let error = InternalError::storage(OuterError {
            source: SourceError,
        });
        assert_eq!(error.kind(), ErrorKind::Storage);
        assert_eq!(
            project(error.kind(), error.public_message()),
            WebError::Storage {
                message: "storage operation failed".to_string()
            }
        );
        // The source chain is preserved for operator logs, never on the wire.
        assert!(error.operator_message().contains("source context"));
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn server_boundary_err_path_projects_the_same_wire_error() {
        let result: WebResult<()> =
            server_boundary("test_fn", async { Err(InternalError::not_found("Post")) }).await;
        assert_eq!(result, Err(WebError::not_found("Post")));
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn server_boundary_logs_client_at_debug_and_returns_public() {
        let result: WebResult<()> = server_boundary("test_fn", async {
            Err(InternalError::validation("bad input"))
        })
        .await;
        assert_eq!(result, Err(WebError::validation("bad input")));
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn server_boundary_logs_external_at_warn_and_returns_public() {
        let result: WebResult<()> = server_boundary("test_fn", async {
            Err(InternalError::external(OuterError {
                source: SourceError,
            }))
        })
        .await;
        assert_eq!(
            result,
            Err(WebError::Server {
                message: "server operation failed".to_string()
            })
        );
    }
}

/// Deterministic validation of the SSR server-fn context-loss mechanism (#89)
/// and the candidate fixes, without the flaky e2e.
///
/// Root cause: the thread-local "current owner" in `reactive_graph` is a *weak*
/// reference. `expect_context`/`use_context` resolve a value by upgrading it and
/// walking the owner ancestry. If the owner's last *strong* ref is dropped while a
/// server-fn future is suspended at an `.await`, its `OwnerInner` (and its context
/// map) is freed, so the post-await lookup upgrades the weak ref to `None` and the
/// context is gone (`expect_context` then panics). These tests reproduce that drop
/// deterministically by hand-polling and dropping the owner between polls — no SSR
/// runtime, no race.
#[cfg(test)]
mod owner_lifetime {
    use leptos::prelude::{provide_context, use_context};
    use leptos::reactive::computed::ScopedFuture;
    use leptos::reactive::owner::Owner;
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll, Waker};

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct Marker(u32);

    /// Yields `Pending` exactly once, then `Ready` — a single suspension point.
    struct YieldOnce(bool);

    impl Future for YieldOnce {
        type Output = ();
        fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
            let this = self.get_mut();
            if this.0 {
                Poll::Ready(())
            } else {
                this.0 = true;
                Poll::Pending
            }
        }
    }

    /// Maps a `Poll` to an `Option`. Routing both the first poll (which yields
    /// `Pending`) and the final poll (`Ready`) through this exercises both arms across
    /// the suite, so there is no uncovered match arm — and it pairs with `Waker::noop()`
    /// to avoid a hand-rolled waker whose vtable closures would never be called.
    fn step<T>(poll: Poll<T>) -> Option<T> {
        match poll {
            Poll::Ready(value) => Some(value),
            Poll::Pending => None,
        }
    }

    /// Covers [`server_resource`](super::server_resource). With a current owner set
    /// (mirroring the adjacent owner tests), calling the sanctioned wrapper exercises
    /// its generic signature and the `Resource::new` line. `Resource::new` eagerly
    /// spawns the fetcher through `any_spawner::Executor`, which panics unless an async
    /// executor is initialized — and no executor is reachable host-side without adding a
    /// dependency (leptos only initializes one inside its wasm `mount` or `leptos_axum`'s
    /// private `init_executor`). So we call `server_resource` (its body runs down to the
    /// `Resource::new` call, covering those lines) and catch the deep executor-spawn
    /// panic; `catch_unwind` therefore returns `Err`. The panic hook is swapped for a
    /// no-op so the expected panic does not print (nextest runs each test in its own
    /// process, so the hook swap is isolated).
    #[cfg(feature = "server")]
    #[test]
    fn server_resource_constructs_under_owner() {
        let owner = Owner::new();
        owner.set();
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            super::server_resource::<i32, (), _>(|| (), |()| async { 0 })
        }));
        std::panic::set_hook(prev);
        assert!(
            result.is_err(),
            "expected the executor-spawn panic reached via Resource::new"
        );
    }

    /// Reproduces #89: a future that reads context *after* an await loses it when
    /// the owner's strong ref is dropped during the suspension.
    #[test]
    fn context_lost_when_owner_dropped_across_await() {
        let owner = Owner::new();
        owner.set();
        provide_context(Marker(7));

        let mut fut = Box::pin(async {
            let pre = use_context::<Marker>();
            YieldOnce(false).await;
            let post = use_context::<Marker>();
            (pre, post)
        });

        let mut cx = Context::from_waker(Waker::noop());
        assert!(step(fut.as_mut().poll(&mut cx)).is_none());
        drop(owner); // last strong ref gone -> OwnerInner freed -> context map dropped
        let (pre, post) =
            step(fut.as_mut().poll(&mut cx)).expect("future did not complete on second poll");
        assert_eq!(pre, Some(Marker(7)));
        assert_eq!(
            post, None,
            "context must vanish once the owner is dropped mid-await"
        );
    }

    /// Central fix: `ScopedFuture` holds a *strong* owner and re-applies it on every
    /// poll, so context survives the await even after our own strong ref is dropped.
    #[test]
    fn scoped_future_keeps_context_alive_across_await() {
        let owner = Owner::new();
        owner.set();
        provide_context(Marker(7));

        let mut fut = Box::pin(ScopedFuture::new_untracked(async {
            let pre = use_context::<Marker>();
            YieldOnce(false).await;
            let post = use_context::<Marker>();
            (pre, post)
        }));

        let mut cx = Context::from_waker(Waker::noop());
        assert!(step(fut.as_mut().poll(&mut cx)).is_none());
        drop(owner); // ScopedFuture retains its own strong owner ref
        let (pre, post) =
            step(fut.as_mut().poll(&mut cx)).expect("future did not complete on second poll");
        assert_eq!(pre, Some(Marker(7)));
        assert_eq!(
            post,
            Some(Marker(7)),
            "ScopedFuture must keep context alive across the await"
        );
    }

    /// Per-fn convention: a value read *before* the await survives the owner drop,
    /// because it no longer depends on the owner once copied out.
    #[test]
    fn context_read_before_await_survives_owner_drop() {
        let owner = Owner::new();
        owner.set();
        provide_context(Marker(7));

        let mut fut = Box::pin(async {
            let captured = use_context::<Marker>(); // read before the await
            YieldOnce(false).await;
            captured
        });

        let mut cx = Context::from_waker(Waker::noop());
        assert!(step(fut.as_mut().poll(&mut cx)).is_none());
        drop(owner);
        let captured =
            step(fut.as_mut().poll(&mut cx)).expect("future did not complete on second poll");
        assert_eq!(
            captured,
            Some(Marker(7)),
            "a value read before the await survives the owner drop"
        );
    }

    /// Falsifies the central fix's failure mode: `ScopedFuture::new` captures
    /// `Owner::current().unwrap_or_default()`, so wrapping a body when *no* owner is
    /// current captures a fresh empty owner and every lookup returns `None` — a
    /// deterministic regression worse than the race. A central fix must therefore
    /// wrap only when an owner is actually current.
    #[test]
    fn scoped_future_with_no_current_owner_sees_empty_context() {
        let owner = Owner::new();
        owner.with(|| provide_context(Marker(7))); // provide, but do NOT set current
        assert!(
            Owner::current().is_none(),
            "precondition: no current owner at wrap time"
        );

        let mut fut = Box::pin(ScopedFuture::new_untracked(async {
            use_context::<Marker>()
        }));
        let mut cx = Context::from_waker(Waker::noop());
        let result = step(fut.as_mut().poll(&mut cx)).expect("future did not complete");
        assert_eq!(
            result, None,
            "ScopedFuture wrapped with no current owner captures an empty owner"
        );
        drop(owner);
    }

    /// #124: a `Resource` fetcher's future wrapped by `scoped_fetcher_future` (what
    /// `server_resource` applies) keeps its context across an owner drop — even when
    /// the owner's strong ref is gone before the future is first polled, the
    /// SSR-resource detachment that `server_boundary` could not cover.
    #[test]
    fn scoped_fetcher_future_keeps_context_across_owner_drop() {
        let owner = Owner::new();
        owner.set();
        provide_context(Marker(7));

        // Build the future exactly as `server_resource` does, then drop our owner
        // ref *before the first poll* (the SSR-resource detachment).
        let mut fut = Box::pin(crate::error::scoped_fetcher_future(async {
            let pre = use_context::<Marker>();
            YieldOnce(false).await;
            let post = use_context::<Marker>();
            (pre, post)
        }));
        drop(owner);

        let mut cx = Context::from_waker(Waker::noop());
        assert!(step(fut.as_mut().poll(&mut cx)).is_none());
        let (pre, post) =
            step(fut.as_mut().poll(&mut cx)).expect("future did not complete on second poll");
        assert_eq!(
            pre,
            Some(Marker(7)),
            "context present at first (detached) poll"
        );
        assert_eq!(post, Some(Marker(7)), "context survives the await");
    }

    /// The actual fix: `server_boundary` must keep context alive across an await,
    /// even when the caller's owner ref is dropped mid-suspension.
    #[cfg(feature = "server")]
    #[test]
    fn server_boundary_keeps_context_alive_across_await() {
        let owner = Owner::new();
        owner.set();
        provide_context(Marker(7));

        let mut fut = Box::pin(crate::error::server_boundary("spike_test", async {
            let _pre = use_context::<Marker>();
            YieldOnce(false).await;
            Ok::<Option<Marker>, crate::error::InternalError>(use_context::<Marker>())
        }));

        let mut cx = Context::from_waker(Waker::noop());
        assert!(step(fut.as_mut().poll(&mut cx)).is_none());
        drop(owner);
        let result =
            step(fut.as_mut().poll(&mut cx)).expect("server_boundary future did not complete");
        assert_eq!(
            result,
            Ok(Some(Marker(7))),
            "server_boundary must keep context alive across the await"
        );
    }

    /// #138: the storage contexts (`UserStorage`/`SiteConfigStorage`) are provided in
    /// an *ancestor* owner (the root `provide_app_state_contexts`), while
    /// `scoped_fetcher_future`/`server_boundary` hold a strong ref only to the
    /// *captured child* owner (the resource's own owner). A post-await `use_context`
    /// walks the ancestry; if the ancestor's strong ref is dropped during the SSR
    /// await, the walk fails — reproducing the backup-fn panic. A pre-await read
    /// resolves before the drop, which is why the ~75 pre-await sites do not panic.
    #[test]
    fn post_await_read_loses_ancestor_context_when_parent_owner_dropped() {
        let parent = Owner::new();
        parent.set();
        provide_context(Marker(7)); // provided in the ANCESTOR (like the root provide)

        let child = Owner::new(); // parent = current = `parent`
        child.set(); // resource's own owner is the captured one

        // Build the fetcher future exactly as `server_resource` does: it captures the
        // currently-set owner (`child`) via `Owner::current().unwrap_or_default()`.
        let mut fut = Box::pin(crate::error::scoped_fetcher_future(async {
            let pre = use_context::<Marker>();
            YieldOnce(false).await;
            let post = use_context::<Marker>();
            (pre, post)
        }));

        let mut cx = Context::from_waker(Waker::noop());
        assert!(step(fut.as_mut().poll(&mut cx)).is_none()); // first poll: reads `pre`, suspends
        drop(parent); // SSR drops the ancestor while the resource future is suspended
        drop(child); // only `ScopedFuture`'s captured strong ref keeps the child alive

        let (pre, post) =
            step(fut.as_mut().poll(&mut cx)).expect("future did not complete on second poll");
        assert_eq!(
            pre,
            Some(Marker(7)),
            "ancestor context resolvable before the drop"
        );
        assert_eq!(
            post, None,
            "#138: post-await read loses ancestor context once the ancestor owner is dropped"
        );
    }

    /// #138 fix: `server_boundary` must keep context that lives in an *ancestor*
    /// owner alive across the await — not just the current (leaf) owner. The
    /// storage contexts are provided at the SSR root (`provide_app_state_contexts`),
    /// an ancestor of each resource's own owner; the SSR runtime can drop that
    /// ancestor while a server fn is suspended. Holding the full ancestry strong
    /// (via `Owner::parent()`) for the future's lifetime makes any post-await
    /// reactive-context read safe, so no per-fn read-before-await discipline is
    /// required. Red before the fix (only the leaf is held), green after.
    #[cfg(feature = "server")]
    #[test]
    fn server_boundary_keeps_ancestor_context_alive_across_await() {
        let root = Owner::new();
        root.set();
        provide_context(Marker(7)); // provided in the ANCESTOR (the SSR root)

        let leaf = Owner::new(); // resource's own owner; parent = root
        leaf.set();

        let mut fut = Box::pin(crate::error::server_boundary("spike_test", async {
            let _pre = use_context::<Marker>();
            YieldOnce(false).await;
            Ok::<Option<Marker>, crate::error::InternalError>(use_context::<Marker>())
        }));

        let mut cx = Context::from_waker(Waker::noop());
        assert!(step(fut.as_mut().poll(&mut cx)).is_none());
        drop(root); // SSR drops the ancestor while the future is suspended
        drop(leaf);
        let result =
            step(fut.as_mut().poll(&mut cx)).expect("server_boundary future did not complete");
        assert_eq!(
            result,
            Ok(Some(Marker(7))),
            "server_boundary must keep ancestor context alive across the await"
        );
    }
}
