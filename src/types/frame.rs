use {
    super::{IntoBytes, IntoStreamingBody, StreamingBody},
    futures_util::{Stream, StreamExt},
    hyper::body::Bytes,
    parking_lot::RwLock,
    std::{
        fmt::Debug,
        marker::PhantomData,
        pin::Pin,
        sync::Arc,
        task::{Context, Poll},
    },
};

/// Trait for length-prefix encoding used by [`FramedBox`].
///
/// Implemented for `u64`, `u32`, `u16`, `u8` (big-endian) and `()` (no prefix).
pub trait FrameLen {
    /// Encode a data length as a length prefix.
    fn encode_len(len: usize) -> Vec<u8>;

    /// Decode a data length from a buffer prefix.
    /// Returns `None` if the buffer doesn't contain enough bytes for the prefix.
    fn decode_len(buf: &[u8]) -> Option<usize>;
}

impl FrameLen for u64 {
    fn encode_len(len: usize) -> Vec<u8> {
        (len as Self).to_be_bytes().to_vec()
    }

    fn decode_len(buf: &[u8]) -> Option<usize> {
        Some(Self::from_be_bytes(buf.get(..size_of::<Self>())?.try_into().ok()?) as _)
    }
}

impl FrameLen for u32 {
    fn encode_len(len: usize) -> Vec<u8> {
        (len as Self).to_be_bytes().to_vec()
    }

    fn decode_len(buf: &[u8]) -> Option<usize> {
        Some(Self::from_be_bytes(buf.get(..size_of::<Self>())?.try_into().ok()?) as _)
    }
}

impl FrameLen for u16 {
    fn encode_len(len: usize) -> Vec<u8> {
        (len as Self).to_be_bytes().to_vec()
    }

    fn decode_len(buf: &[u8]) -> Option<usize> {
        Some(Self::from_be_bytes(buf.get(..size_of::<Self>())?.try_into().ok()?) as _)
    }
}

impl FrameLen for u8 {
    fn encode_len(len: usize) -> Vec<u8> {
        vec![len as u8]
    }

    fn decode_len(buf: &[u8]) -> Option<usize> {
        buf.first().map(|&b| b as _)
    }
}

impl FrameLen for () {
    fn encode_len(_len: usize) -> Vec<u8> {
        Vec::new()
    }

    fn decode_len(_buf: &[u8]) -> Option<usize> {
        Some(0)
    }
}

/// A boxed pinned stream with configurable length-prefix framing.
///
/// `FramedBox<S, N>` wraps `Pin<Box<S>>` internally, so you don't need
/// to write `Pin<Box<...>>` yourself. Use [`FramedBox::pin`] to construct.
///
/// The generic parameter `N` (default `u16`) controls the length-prefix size:
/// - `u64` → 8-byte prefix, `u32` → 4-byte, `u16` → 2-byte, `u8` → 1-byte
/// - `()` → no prefix (raw stream, each HTTP chunk is one item)
///
/// # Wire format (when `N ≠ ()`)
/// Each item: `[N::SIZE bytes: length in big-endian][length bytes: data]`
///
/// # Server example
/// ```ignore
/// // Raw stream — no framing (same as Pin<Box<S>>)
/// async fn raw_handler() -> Pin<Box<impl Stream<Item = String>>> { ... }
///
/// // Framed stream — 2-byte length prefix (default)
/// async fn framed_handler() -> FramedBox<impl Stream<Item = Value>> {
///     FramedBox::pin(stream! { yield json!({"n": 1}); })
/// }
///
/// // Framed stream — 4-byte length prefix (for items > 65KB)
/// async fn large_handler() -> FramedBox<impl Stream<Item = Value>, u32> {
///     FramedBox::pin(stream! { yield json!({"n": 1}); })
/// }
/// ```
///
/// # Client example
/// ```ignore
/// // Raw stream
/// let (stream, _): (Pin<Box<dyn Stream<Item = IoResult<String>> + Send + Sync>>, _) = ...;
///
/// // Framed stream — 2-byte prefix (default)
/// let (stream, _): (FramedBox<dyn Stream<Item = IoResult<Value>> + Send + Sync>, _) = ...;
///
/// // Framed stream — 4-byte prefix
/// let (stream, _): (FramedBox<dyn Stream<Item = IoResult<Value>> + Send + Sync, u32>, _) = ...;
/// ```
pub struct FramedBox<S, N = u16>
where
    S: ?Sized,
{
    inner: Pin<Box<S>>,
    _len: PhantomData<fn() -> N>,
}

impl<S, N> FramedBox<S, N>
where
    S: ?Sized,
{
    pub(super) fn new(inner: Pin<Box<S>>) -> Self {
        Self {
            inner,
            _len: Default::default(),
        }
    }
}

impl<S, N> FramedBox<S, N> {
    /// Creates a new `FramedBox` by pinning the given stream.
    pub fn pin(stream: S) -> Self {
        Self {
            inner: Box::pin(stream),
            _len: PhantomData,
        }
    }
}

impl<S, N> Debug for FramedBox<S, N>
where
    S: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FramedBox")
            .field("inner", &self.inner)
            .finish()
    }
}

impl<S: ?Sized, N> Stream for FramedBox<S, N>
where
    S: Stream,
{
    type Item = S::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // SAFETY: We never move the inner Pin<Box<S>>, only poll it.
        let this = unsafe { self.get_unchecked_mut() };
        this.inner.as_mut().poll_next(cx)
    }
}

impl<S, T, N> IntoStreamingBody for FramedBox<S, N>
where
    S: Stream<Item = T> + Send + Sync + 'static,
    T: IntoBytes + 'static,
    N: FrameLen,
{
    fn into_streaming_body(self) -> StreamingBody {
        if size_of::<N>() == 0 {
            // Raw mode — no framing prefix
            StreamingBody::Stream {
                stream: Arc::new(RwLock::new(Box::pin(self.inner.map(IntoBytes::into)))),
            }
        } else {
            // Framed mode — prepend length prefix to each item
            StreamingBody::Stream {
                stream: Arc::new(RwLock::new(Box::pin(self.inner.map(|i| {
                    let data: Bytes = i.into();
                    let mut framed = Vec::with_capacity(size_of::<N>() + data.len());
                    framed.extend_from_slice(&N::encode_len(data.len()));
                    framed.extend_from_slice(&data);
                    Bytes::from(framed)
                })))),
            }
        }
    }
}
