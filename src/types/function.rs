#[cfg(feature = "client")]
use super::HttpClientRouter;
#[cfg(feature = "server")]
use super::HttpServerRouter;
use {
    super::{Accessor, IoResult, WsRouter},
    serde::{Deserialize, Serialize},
};

/// Trait for async functions that can handle WebSocket messages.
///
/// This trait is automatically implemented for async functions that:
/// - Take an `Accessor<Acc>` as the first argument
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
pub trait WsAsyncFn<Args, Ret, Acc>
where
    Self: Copy + Send + 'static,
{
    fn bind_by_path<R, P>(self, router: &R, path: P) -> impl Future<Output = IoResult<R::Binding>>
    where
        R: WsRouter<Acc>,
        P: AsRef<str> + Send + Sync + 'static,
        Args: for<'a> Deserialize<'a> + Serialize + 'static,
        Ret: for<'a> Deserialize<'a> + Serialize + Send + 'static,
    {
        router.add_route::<_, _, Args, Ret>(path, self)
    }

    fn bind<R>(self, router: &R) -> impl Future<Output = IoResult<R::Binding>>
    where
        R: WsRouter<Acc>,
        Args: for<'a> Deserialize<'a> + Serialize + 'static,
        Ret: for<'a> Deserialize<'a> + Serialize + Send + 'static,
    {
        router.add_route::<_, _, Args, Ret>(crate::utils::get_path::<Self>(), self)
    }

    fn call(self, accessor: Acc, args: Args) -> impl Future<Output = Ret> + Send;
}

macro_rules! impl_ws_async_fn {
    ($($t:ident : $T:ident),* $(,)?) => {
        impl<Fun, Fut, Ret, Acc, $($T,)*> WsAsyncFn<($($T,)*), Ret, Acc> for Fun
        where
            Fut: Future<Output = Ret> + Send + 'static,
            Fun: Fn(Accessor<Acc>, $($T,)*) -> Fut + Copy + Send + 'static,
            $($T: Send,)*
        {
            fn call(self, accessor: Acc, ($($t,)*): ($($T,)*)) -> impl Future<Output = Ret> + Send {
                self(accessor.into(), $($t),*)
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
/// - Take an `Accessor<Acc>` as the first argument
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
pub trait HttpServerAsyncFn<Body, Ret, Acc>
where
    Self: Copy + Send + 'static,
{
    fn bind_by_path_as_response<R, P>(
        self,
        router: &R,
        path: P,
    ) -> impl Future<Output = IoResult<R::Binding>>
    where
        R: HttpServerRouter<Acc>,
        P: AsRef<str> + Send + Sync + 'static,
        Body: From<crate::types::stream::StreamingBody>,
        Ret: crate::types::stream::IntoStreamingBody,
    {
        router.add_route::<_, _, Body, Ret>(path, self)
    }

    fn bind_as_response<R>(self, router: &R) -> impl Future<Output = IoResult<R::Binding>>
    where
        R: HttpServerRouter<Acc>,
        Body: From<crate::types::stream::StreamingBody>,
        Ret: crate::types::stream::IntoStreamingBody,
    {
        router.add_route::<_, _, Body, Ret>(crate::utils::get_path::<Self>(), self)
    }

    fn call(self, accessor: Acc, body: Body) -> impl Future<Output = Ret> + Send;
}

#[cfg(feature = "server")]
impl<Fun, Fut, Body, Ret, Acc> HttpServerAsyncFn<Body, Ret, Acc> for Fun
where
    Fut: Future<Output = Ret> + Send + 'static,
    Fun: for<'a> Fn(crate::types::Accessor<Acc>, Body) -> Fut + Copy + Send + 'static,
    Body: Send,
{
    fn call(self, accessor: Acc, body: Body) -> impl Future<Output = Ret> + Send {
        self(accessor.into(), body)
    }
}

/// Trait for async functions that can handle HTTP requests (client-side).
///
/// This trait is automatically implemented for async functions that:
/// - Take an `Accessor<Acc>` as the only argument
/// - Return `()`
///
/// Used for binding HTTP request handlers on the client side.
#[cfg(feature = "client")]
pub trait HttpClientAsyncFn<Acc>
where
    Self: Copy + Send + 'static,
{
    fn bind_by_path_as_request<R, P>(
        self,
        router: &R,
        path: P,
    ) -> impl Future<Output = IoResult<R::Binding>>
    where
        R: HttpClientRouter<Acc>,
        P: AsRef<str> + Send + Sync + 'static,
    {
        router.add_route::<_, _>(path, self)
    }

    fn bind_as_request<R>(self, router: &R) -> impl Future<Output = IoResult<R::Binding>>
    where
        R: HttpClientRouter<Acc>,
    {
        router.add_route::<_, _>(crate::utils::get_path::<Self>(), self)
    }

    fn call(self, accessor: Acc) -> impl Future<Output = ()> + Send;
}

#[cfg(feature = "client")]
impl<Fun, Fut, Acc> HttpClientAsyncFn<Acc> for Fun
where
    Fut: Future<Output = ()> + Send + 'static,
    Fun: for<'a> Fn(Accessor<Acc>) -> Fut + Copy + Send + 'static,
{
    fn call(self, accessor: Acc) -> impl Future<Output = ()> + Send {
        self(accessor.into())
    }
}
