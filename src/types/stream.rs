#[cfg(feature = "serde_json")]
use serde_json::{Value, from_slice, to_vec};
use {
    super::{FromBytes, IntoBytes},
    futures_util::{Stream, StreamExt},
    hyper::{
        Error,
        body::{Body, Bytes, Frame, Incoming, SizeHint},
    },
    parking_lot::RwLock,
    std::{
        fmt::Debug,
        io::{Error as IoError, Result as IoResult},
        mem::take,
        pin::Pin,
        sync::Arc,
        task::{Context, Poll},
    },
    tracing::error,
};

enum VecU8Future {
    Bytes {
        bytes: Option<Bytes>,
    },
    Incoming {
        incoming: Arc<RwLock<Incoming>>,
        all_data: Vec<u8>,
    },
    Stream {
        stream: Arc<RwLock<Pin<Box<dyn Stream<Item = Bytes> + Send + Sync>>>>,
        all_data: Vec<u8>,
    },
}

impl Default for VecU8Future {
    fn default() -> Self {
        Self::Bytes { bytes: None }
    }
}

impl Future for VecU8Future {
    type Output = Vec<u8>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        match this {
            Self::Incoming { incoming, all_data } => {
                let mut incoming = incoming.write();

                if incoming.is_end_stream() {
                    return Poll::Ready(take(all_data));
                }

                match Pin::new(&mut *incoming).poll_frame(cx) {
                    Poll::Ready(Some(Ok(frame))) => {
                        match frame.into_data() {
                            Ok(data) => all_data.extend_from_slice(&data),

                            Err(e) => {
                                error!(?e, "Failed to get data");
                                return Poll::Ready(take(all_data));
                            }
                        }
                        cx.waker().wake_by_ref();
                        Poll::Pending
                    }
                    Poll::Ready(Some(Err(e))) => {
                        error!(?e, "Failed to get frame");
                        Poll::Ready(take(all_data))
                    }
                    Poll::Ready(None) => Poll::Ready(take(all_data)),
                    Poll::Pending => Poll::Pending,
                }
            }
            Self::Stream { stream, all_data } => {
                let mut stream = stream.write();

                match stream.as_mut().poll_next(cx) {
                    Poll::Ready(Some(data)) => {
                        all_data.extend_from_slice(&data);
                        cx.waker().wake_by_ref();
                        Poll::Pending
                    }
                    Poll::Ready(None) => Poll::Ready(take(all_data)),
                    Poll::Pending => Poll::Pending,
                }
            }
            Self::Bytes { bytes } => {
                if let Some(bytes) = bytes.take() {
                    Poll::Ready(bytes.to_vec())
                } else {
                    Poll::Ready(Default::default())
                }
            }
        }
    }
}

/// A streaming body type that can represent various HTTP body sources.
///
/// This enum supports:
/// - `Null`: Empty body
/// - `Bytes`: Fixed-size byte buffer
/// - `Incoming`: Hyper's incoming body stream
/// - `Stream`: Custom async stream
#[derive(Default)]
pub enum StreamingBody {
    #[default]
    Null,
    Bytes {
        bytes: Option<Bytes>,
    },
    Incoming {
        incoming: Arc<RwLock<Incoming>>,
    },
    Stream {
        stream: Arc<RwLock<Pin<Box<dyn Stream<Item = Bytes> + Send + Sync>>>>,
    },
}

impl Clone for StreamingBody {
    fn clone(&self) -> Self {
        match self {
            Self::Null => Self::Null,
            Self::Bytes { bytes } => Self::Bytes {
                bytes: bytes.clone(),
            },
            Self::Incoming { incoming } => Self::Incoming {
                incoming: incoming.clone(),
            },
            Self::Stream { stream } => Self::Stream {
                stream: stream.clone(),
            },
        }
    }
}

impl Debug for StreamingBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Null => write!(f, "Null"),
            Self::Bytes { .. } => write!(f, "Bytes"),
            Self::Incoming { .. } => write!(f, "Incoming"),
            Self::Stream { .. } => write!(f, "Stream"),
        }
    }
}

impl StreamingBody {
    fn into_vec(self) -> VecU8Future {
        match self {
            Self::Null => VecU8Future::Bytes { bytes: None },
            Self::Bytes { bytes } => VecU8Future::Bytes { bytes },
            Self::Incoming { incoming } => VecU8Future::Incoming {
                incoming: incoming.clone(),
                all_data: Default::default(),
            },
            Self::Stream { stream } => VecU8Future::Stream {
                stream: stream.clone(),
                all_data: Default::default(),
            },
        }
    }
}

impl Body for StreamingBody {
    type Data = Bytes;
    type Error = Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        match self.get_mut() {
            Self::Null => Poll::Ready(None),
            Self::Bytes { bytes } => Poll::Ready(bytes.take().map(|b| Ok(Frame::data(b)))),
            Self::Incoming { incoming } => Pin::new(&mut *incoming.write()).poll_frame(cx),
            Self::Stream { stream } => {
                let mut stream = stream.write();
                stream
                    .as_mut()
                    .poll_next(cx)
                    .map(|opt| opt.map(|i| Ok(Frame::data(i))))
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        match self {
            Self::Null => true,
            Self::Bytes { bytes } => bytes.is_none(),
            Self::Incoming { incoming } => incoming.read().is_end_stream(),
            Self::Stream { .. } => false,
        }
    }

    fn size_hint(&self) -> SizeHint {
        match self {
            Self::Bytes { bytes: Some(bytes) } => SizeHint::with_exact(bytes.len() as _),
            Self::Incoming { incoming } => incoming.read().size_hint(),
            Self::Stream { stream } => {
                let (min, max) = stream.read().size_hint();
                let mut size = SizeHint::new();
                size.set_lower(min as _);
                if let Some(max) = max {
                    size.set_upper(max as _);
                }
                size
            }
            _ => Default::default(),
        }
    }
}

// Concrete From implementations for common types
/// Trait for types that can be converted into a `StreamingBody`.
///
/// Implemented for common types like `String`, `Vec<u8>`, `Bytes`,
/// and streams that produce `Bytes`.
pub trait IntoStreamingBody {
    /// Converts this type into a `StreamingBody`.
    fn into_streaming_body(self) -> StreamingBody;
}

impl IntoStreamingBody for Incoming {
    fn into_streaming_body(self) -> StreamingBody {
        StreamingBody::Incoming {
            incoming: RwLock::new(self).into(),
        }
    }
}

impl IntoStreamingBody for &str {
    fn into_streaming_body(self) -> StreamingBody {
        IntoBytes::into(self).into_streaming_body()
    }
}

impl IntoStreamingBody for &[u8] {
    fn into_streaming_body(self) -> StreamingBody {
        IntoBytes::into(self).into_streaming_body()
    }
}

impl IntoStreamingBody for String {
    fn into_streaming_body(self) -> StreamingBody {
        IntoBytes::into(self).into_streaming_body()
    }
}

impl IntoStreamingBody for Bytes {
    fn into_streaming_body(self) -> StreamingBody {
        StreamingBody::Bytes { bytes: Some(self) }
    }
}

impl<S, T> IntoStreamingBody for Pin<Box<S>>
where
    S: Stream<Item = T> + Send + Sync + 'static,
    T: IntoBytes,
{
    fn into_streaming_body(self) -> StreamingBody {
        StreamingBody::Stream {
            stream: Arc::new(RwLock::new(Box::pin(self.map(|i| i.into())))),
        }
    }
}

impl IntoStreamingBody for () {
    fn into_streaming_body(self) -> StreamingBody {
        StreamingBody::Null
    }
}

/// Trait for types that can be converted from a `StreamingBody`.
///
/// Implemented for common types like `String`, `Vec<u8>`, `Bytes`,
pub trait FromStreamingBody {
    fn from_streaming_body(body: StreamingBody) -> impl Future<Output = Self> + Send;
}

impl FromStreamingBody for String {
    async fn from_streaming_body(body: StreamingBody) -> Self {
        String::from_utf8(body.into_vec().await).unwrap_or_default()
    }
}

impl FromStreamingBody for Vec<u8> {
    async fn from_streaming_body(body: StreamingBody) -> Self {
        body.into_vec().await
    }
}

impl FromStreamingBody for () {
    async fn from_streaming_body(_: StreamingBody) -> Self {}
}

impl<'a, T> FromStreamingBody for Pin<Box<dyn Stream<Item = IoResult<T>> + Send + Sync + 'a>>
where
    T: FromBytes,
{
    async fn from_streaming_body(body: StreamingBody) -> Self {
        Box::pin(body.map(|i| i.map(FromBytes::from)))
    }
}

impl Stream for StreamingBody {
    type Item = IoResult<Bytes>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.poll_frame(cx).map(|i| {
            i.map(|i| {
                i.map_or_else(
                    |e| Err(IoError::other(e)),
                    |i| {
                        i.into_data().map_err(|e| {
                            IoError::other(format!("Failed to convert the data: {:?}", e))
                        })
                    },
                )
            })
        })
    }
}

#[cfg(feature = "serde_json")]
impl IntoStreamingBody for Value {
    fn into_streaming_body(self) -> StreamingBody {
        to_vec(&self).unwrap_or_default().into_streaming_body()
    }
}

#[cfg(feature = "serde_json")]
impl FromStreamingBody for Value {
    async fn from_streaming_body(body: StreamingBody) -> Self {
        from_slice(&body.into_vec().await).unwrap_or_default()
    }
}
