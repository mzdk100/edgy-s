#[cfg(feature = "client")]
use super::HttpClientRouter;
#[cfg(feature = "server")]
use super::{FromStreamingBody, HttpServerRouter, IntoStreamingBody};
use {
    super::{super::get_path, Accessor, WsRouter},
    serde::{Deserialize, Serialize},
    std::io::Result as IoResult,
};

/// Trait for async functions that can handle WebSocket messages.
///
/// This trait is automatically implemented for async functions that:
/// - Take an `Accessor<Acc, S>` as the first argument
/// - Take up to 26 additional serializable arguments
/// - Return a serializable type
///
/// # Example
/// ```ignore
/// async fn my_handler(accessor: WsAccessor, a: i32, b: i32) -> i32 {
///     a + b
/// }
///
/// // Can be bound to a WebSocket route
/// my_handler.bind(&client).await?;
/// ```
pub trait WsAsyncFn<Args, Ret, Acc, S = ()>
where
    Self: Copy + Send + 'static,
{
    fn bind_by_path<R, P>(self, router: &R, path: P) -> impl Future<Output = IoResult<R::Binding>>
    where
        R: WsRouter<Acc, S>,
        P: AsRef<str> + Send + Sync + 'static,
        Args: for<'a> Deserialize<'a> + Serialize + 'static,
        Ret: for<'a> Deserialize<'a> + Serialize + Send + 'static,
    {
        router.add_route::<_, _, Args, Ret>(path, self)
    }

    fn bind<R>(self, router: &R) -> impl Future<Output = IoResult<R::Binding>>
    where
        R: WsRouter<Acc, S>,
        Args: for<'a> Deserialize<'a> + Serialize + 'static,
        Ret: for<'a> Deserialize<'a> + Serialize + Send + 'static,
    {
        router.add_route::<_, _, Args, Ret>(get_path::<Self>(), self)
    }

    fn call(self, accessor: Accessor<Acc>, args: Args) -> impl Future<Output = Ret> + Send;
}

macro_rules! impl_ws_async_fn {
    ($($t:ident : $T:ident),* $(,)?) => {
        impl<Fun, Fut, Ret, Acc, State, $($T,)*> WsAsyncFn<($($T,)*), Ret, Acc, State> for Fun
        where
            Fut: Future<Output = Ret> + Send + 'static,
            Fun: Fn(Accessor<Acc>, $($T,)*) -> Fut + Copy + Send + 'static,
            $($T: Send,)*
        {
            fn call(self, accessor: Accessor<Acc>, ($($t,)*): ($($T,)*)) -> impl Future<Output = Ret> + Send {
                self(accessor, $($t),*)
            }
        }
    }
}

impl_ws_async_fn!();
impl_ws_async_fn!(a: A);
impl_ws_async_fn!(a: A, b: B);
impl_ws_async_fn!(a: A, b: B, c: C);
impl_ws_async_fn!(a: A, b: B, c: C, d: D);
impl_ws_async_fn!(a: A, b: B, c: C, d: D, e: E);
impl_ws_async_fn!(a: A, b: B, c: C, d: D, e: E, f: F);
impl_ws_async_fn!(a: A, b: B, c: C, d: D, e: E, f: F, g: G);
impl_ws_async_fn!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H);
impl_ws_async_fn!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I);
impl_ws_async_fn!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J);
impl_ws_async_fn!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K);
impl_ws_async_fn!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K, l: L);
impl_ws_async_fn!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K, l: L, m: M);
impl_ws_async_fn!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K, l: L, m: M, n: N);
impl_ws_async_fn!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K, l: L, m: M, n: N, o: O);
impl_ws_async_fn!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K, l: L, m: M, n: N, o: O, p: P);
impl_ws_async_fn!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K, l: L, m: M, n: N, o: O, p: P, q: Q);
impl_ws_async_fn!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K, l: L, m: M, n: N, o: O, p: P, q: Q, r: R);
impl_ws_async_fn!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K, l: L, m: M, n: N, o: O, p: P, q: Q, r: R, s: S);
impl_ws_async_fn!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K, l: L, m: M, n: N, o: O, p: P, q: Q, r: R, s: S, t: T);
impl_ws_async_fn!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K, l: L, m: M, n: N, o: O, p: P, q: Q, r: R, s: S, t: T, u: U);
impl_ws_async_fn!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K, l: L, m: M, n: N, o: O, p: P, q: Q, r: R, s: S, t: T, u: U, v: V);
impl_ws_async_fn!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K, l: L, m: M, n: N, o: O, p: P, q: Q, r: R, s: S, t: T, u: U, v: V, w: W);
impl_ws_async_fn!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K, l: L, m: M, n: N, o: O, p: P, q: Q, r: R, s: S, t: T, u: U, v: V, w: W, x: X);
impl_ws_async_fn!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K, l: L, m: M, n: N, o: O, p: P, q: Q, r: R, s: S, t: T, u: U, v: V, w: W, x: X, y: Y);
impl_ws_async_fn!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K, l: L, m: M, n: N, o: O, p: P, q: Q, r: R, s: S, t: T, u: U, v: V, w: W, x: X, y: Y, z: Z);

/// Trait for async functions that can handle HTTP requests (server-side).
///
/// This trait is automatically implemented for async functions that:
/// - Take an `Accessor<Acc, S>` as the first argument
/// - Take a body type that implements `From<StreamingBody>`
/// - Return a type that implements `IntoStreamingBody`
///
/// # Example
/// ```ignore
/// async fn my_handler(accessor: HttpAccessor, body: String) -> String {
///     format!("Received: {}", body)
/// }
///
/// // Can be bound to an HTTP route
/// my_handler.bind_as_response(&service).await?;
/// ```
#[cfg(feature = "server")]
pub trait HttpServerAsyncFn<Body, Ret, Acc, S = ()>
where
    Self: Copy + Send + 'static,
{
    fn bind_by_path_as_response<R, P>(
        self,
        router: &R,
        path: P,
    ) -> impl Future<Output = IoResult<R::Binding>>
    where
        R: HttpServerRouter<Acc, S>,
        P: AsRef<str> + Send + Sync + 'static,
        Body: FromStreamingBody,
        Ret: IntoStreamingBody,
    {
        router.add_route::<_, _, Body, Ret>(path, self)
    }

    fn bind_as_response<R>(self, router: &R) -> impl Future<Output = IoResult<R::Binding>>
    where
        R: HttpServerRouter<Acc, S>,
        Body: FromStreamingBody,
        Ret: IntoStreamingBody,
    {
        router.add_route::<_, _, Body, Ret>(get_path::<Self>(), self)
    }

    /// Binds this handler as the default HTTP route for unmatched paths.
    ///
    /// If a request arrives for a path that has no registered route,
    /// this handler will be invoked. If no default handler is set,
    /// unmatched requests return a 500 Internal Server Error.
    fn bind_as_default<R>(self, router: &R) -> impl Future<Output = IoResult<R::Binding>>
    where
        R: HttpServerRouter<Acc, S>,
        Body: FromStreamingBody,
        Ret: IntoStreamingBody,
    {
        router.add_default_route::<_, Body, Ret>(self)
    }

    fn call(self, accessor: Accessor<Acc>, body: Body) -> impl Future<Output = Ret> + Send;
}

#[cfg(feature = "server")]
impl<Fun, Fut, Body, Ret, Acc, S> HttpServerAsyncFn<Body, Ret, Acc, S> for Fun
where
    Fut: Future<Output = Ret> + Send + 'static,
    Fun: for<'a> Fn(Accessor<Acc>, Body) -> Fut + Copy + Send + 'static,
    Body: Send,
{
    fn call(self, accessor: Accessor<Acc>, body: Body) -> impl Future<Output = Ret> + Send {
        self(accessor, body)
    }
}

/// Trait for async functions that can handle HTTP requests (client-side).
///
/// This trait is automatically implemented for async functions that:
/// - Take an `Accessor<Acc, S>` as the only argument
/// - Return `()`
///
/// Used for binding HTTP request handlers on the client side.
#[cfg(feature = "client")]
pub trait HttpClientAsyncFn<Acc, S = ()>
where
    Self: Copy + Send + 'static,
{
    fn bind_by_path_as_request<R, P>(
        self,
        router: &R,
        path: P,
    ) -> impl Future<Output = IoResult<R::Binding>>
    where
        R: HttpClientRouter<Acc, S>,
        P: AsRef<str> + Send + Sync + 'static,
    {
        router.add_route::<_, _>(path, self)
    }

    fn bind_as_request<R>(self, router: &R) -> impl Future<Output = IoResult<R::Binding>>
    where
        R: HttpClientRouter<Acc, S>,
    {
        router.add_route::<_, _>(get_path::<Self>(), self)
    }

    fn call(self, accessor: Accessor<Acc>) -> impl Future<Output = ()> + Send;
}

#[cfg(feature = "client")]
impl<Fun, Fut, Acc, S> HttpClientAsyncFn<Acc, S> for Fun
where
    Fut: Future<Output = ()> + Send + 'static,
    Fun: for<'a> Fn(Accessor<Acc>) -> Fut + Copy + Send + 'static,
{
    fn call(self, accessor: Accessor<Acc>) -> impl Future<Output = ()> + Send {
        self(accessor)
    }
}
