use {
    postcard::{from_bytes, to_allocvec},
    serde::{Deserialize, Serialize},
    std::{
        any::type_name,
        io::{Error as IoError, Result as IoResult},
        ops::{Deref, DerefMut},
    },
    tokio_tungstenite::tungstenite::Message,
};

/// Request ID type based on selected feature flag
/// Priority: u64 > u32 > u16 > u8 (default)
#[cfg(feature = "req_id_u64")]
pub(super) type ReqId = u64;
#[cfg(all(feature = "req_id_u32", not(feature = "req_id_u64")))]
pub(super) type ReqId = u32;
#[cfg(all(
    feature = "req_id_u16",
    not(any(feature = "req_id_u32", feature = "req_id_u64"))
))]
pub(super) type ReqId = u16;
#[cfg(not(any(feature = "req_id_u16", feature = "req_id_u32", feature = "req_id_u64")))]
pub(super) type ReqId = u8;

/// Atomic version of ReqId for thread-safe ID generation
#[cfg(feature = "req_id_u64")]
pub(super) type AtomicReqId = std::sync::atomic::AtomicU64;
#[cfg(all(feature = "req_id_u32", not(feature = "req_id_u64")))]
pub(super) type AtomicReqId = std::sync::atomic::AtomicU32;
#[cfg(all(
    feature = "req_id_u16",
    not(any(feature = "req_id_u32", feature = "req_id_u64"))
))]
pub(super) type AtomicReqId = std::sync::atomic::AtomicU16;
#[cfg(not(any(feature = "req_id_u16", feature = "req_id_u32", feature = "req_id_u64")))]
pub(super) type AtomicReqId = std::sync::atomic::AtomicU8;

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub(super) enum Packet<A, B> {
    Call(ReqId, A),
    Ret(ReqId, B),
}

unsafe impl<A, B> Send for Packet<A, B> {}

impl<A, B> Packet<A, B> {
    pub fn from_message(message: &Message) -> IoResult<Self>
    where
        A: for<'a> Deserialize<'a>,
        B: for<'a> Deserialize<'a>,
    {
        match message {
            Message::Binary(bytes) => from_bytes(bytes).map_err(IoError::other),
            _ => Err(IoError::other("Unsupported message.")),
        }
    }

    pub fn make_ret_message(id: ReqId, data: B) -> IoResult<Message>
    where
        A: Serialize,
        B: Serialize,
    {
        Ok(Message::Binary(
            to_allocvec(&Self::Ret(id, data))
                .map_err(IoError::other)?
                .into(),
        ))
    }

    pub fn make_call_message(id: ReqId, data: A) -> IoResult<Message>
    where
        A: Serialize,
        B: Serialize,
    {
        Ok(Message::Binary(
            to_allocvec(&Self::Call(id, data))
                .map_err(IoError::other)?
                .into(),
        ))
    }
}

pub struct Accessor<I> {
    inner: I,
}

impl<I> Deref for Accessor<I> {
    type Target = I;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<I> DerefMut for Accessor<I> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<I> From<I> for Accessor<I> {
    fn from(inner: I) -> Self {
        Self { inner }
    }
}

pub trait Router<Acc> {
    fn add_route<F, P, Args, Ret>(&self, path: P, handler: F) -> impl Future<Output = IoResult<()>>
    where
        F: AsyncFun<Args, Ret, Acc>,
        Args: for<'a> Deserialize<'a> + Serialize,
        Ret: for<'a> Deserialize<'a> + Serialize + Send,
        P: AsRef<str>;

    fn remove_route<P>(&self, path: P) -> impl Future<Output = IoResult<()>>
    where
        P: AsRef<str>;
}

pub trait AsyncFun<Args, Ret, Acc>
where
    Self: Copy + Send + 'static,
{
    fn bind_by_path<R, P>(self, router: &R, path: P) -> impl Future<Output = IoResult<()>>
    where
        R: Router<Acc>,
        P: AsRef<str> + Send + Sync + 'static,
        Args: for<'a> Deserialize<'a> + Serialize,
        Ret: for<'a> Deserialize<'a> + Serialize + Send,
    {
        router.add_route::<_, _, Args, Ret>(path, self)
    }

    fn bind<R>(self, router: &R) -> impl Future<Output = IoResult<()>>
    where
        R: Router<Acc>,
        Args: for<'a> Deserialize<'a> + Serialize,
        Ret: for<'a> Deserialize<'a> + Serialize + Send,
    {
        router.add_route::<_, _, Args, Ret>(<Self as AsyncFun<Args, Ret, Acc>>::get_path(), self)
    }

    fn unbind_by_path<R, P>(self, router: &R, path: P) -> impl Future<Output = IoResult<()>>
    where
        R: Router<Acc>,
        P: AsRef<str> + Send + Sync + 'static,
    {
        router.remove_route(path)
    }

    fn unbind<R>(self, router: &R) -> impl Future<Output = IoResult<()>>
    where
        R: Router<Acc>,
    {
        router.remove_route(Self::get_path())
    }

    fn get_path() -> String {
        format!(
            "/{}",
            type_name::<Self>()
                .split("::")
                .last()
                .unwrap_or("")
                .replace("_", "/")
        )
    }

    fn call(self, accessor: Acc, args: Args) -> impl Future<Output = Ret> + Send;
}

macro_rules! impl_async_fun {
    ($($t:ident : $T:ident),* $(,)?) => {
        impl<Fun, Fut, Ret, Acc, $($T,)*> AsyncFun<($($T,)*), Ret, Acc> for Fun
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

impl_async_fun!();
impl_async_fun!(a: A);
impl_async_fun!(a: A, b: B);
impl_async_fun!(a: A, b: B, c: C);
impl_async_fun!(a: A, b: B, c: C, d: D);
impl_async_fun!(a: A, b: B, c: C, d: D, e: E);
impl_async_fun!(a: A, b: B, c: C, d: D, e: E, f: F);
impl_async_fun!(a: A, b: B, c: C, d: D, e: E, f: F, g: G);
impl_async_fun!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H);
impl_async_fun!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I);
impl_async_fun!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J);
impl_async_fun!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K);
impl_async_fun!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K, l: L);
impl_async_fun!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K, l: L, m: M);
impl_async_fun!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K, l: L, m: M, n: N);
impl_async_fun!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K, l: L, m: M, n: N, o: O);
impl_async_fun!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K, l: L, m: M, n: N, o: O, p: P);
impl_async_fun!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K, l: L, m: M, n: N, o: O, p: P, q: Q);
impl_async_fun!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K, l: L, m: M, n: N, o: O, p: P, q: Q, r: R);
impl_async_fun!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K, l: L, m: M, n: N, o: O, p: P, q: Q, r: R, s: S);
impl_async_fun!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K, l: L, m: M, n: N, o: O, p: P, q: Q, r: R, s: S, t: T);
impl_async_fun!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K, l: L, m: M, n: N, o: O, p: P, q: Q, r: R, s: S, t: T, u: U);
impl_async_fun!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K, l: L, m: M, n: N, o: O, p: P, q: Q, r: R, s: S, t: T, u: U, v: V);
impl_async_fun!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K, l: L, m: M, n: N, o: O, p: P, q: Q, r: R, s: S, t: T, u: U, v: V, w: W);
impl_async_fun!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K, l: L, m: M, n: N, o: O, p: P, q: Q, r: R, s: S, t: T, u: U, v: V, w: W, x: X);
impl_async_fun!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K, l: L, m: M, n: N, o: O, p: P, q: Q, r: R, s: S, t: T, u: U, v: V, w: W, x: X, y: Y);
impl_async_fun!(a: A, b: B, c: C, d: D, e: E, f: F, g: G, h: H, i: I, j: J, k: K, l: L, m: M, n: N, o: O, p: P, q: Q, r: R, s: S, t: T, u: U, v: V, w: W, x: X, y: Y, z: Z);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_packet() -> anyhow::Result<()> {
        let data = Packet::<(), i32>::make_ret_message(100 as ReqId, 3)?;
        let ret = Packet::<(), i32>::from_message(&data)?;
        assert_eq!(ret, Packet::Ret(100 as ReqId, 3));

        Ok(())
    }
}
