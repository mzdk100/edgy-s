use {
    futures_util::{Stream, StreamExt},
    hyper::{
        Error,
        body::{Body, Bytes, Frame, Incoming, SizeHint},
    },
    std::{
        fmt::Debug,
        io::{Error as IoError, Result as IoResult},
        pin::Pin,
        task::{Context, Poll, Waker},
    },
    tracing::error,
};

/// A streaming body type that can represent various HTTP body sources.
///
/// This enum supports:
/// - `Null`: Empty body
/// - `Bytes`: Fixed-size byte buffer
/// - `Incoming`: Hyper's incoming body stream
/// - `Stream`: Custom async stream
pub enum StreamingBody {
    Null,
    Bytes {
        bytes: Option<Bytes>,
    },
    Incoming {
        incoming: Incoming,
    },
    Stream {
        stream: Pin<Box<dyn Stream<Item = Bytes> + Send + Sync>>,
    },
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
    fn into_vec(self) -> Vec<u8> {
        let mut ret = Vec::new();
        let mut cx = Context::from_waker(Waker::noop());
        match self {
            StreamingBody::Null => (),
            StreamingBody::Bytes { bytes } => {
                if let Some(data) = bytes {
                    ret.extend_from_slice(&data)
                }
            }
            StreamingBody::Incoming { mut incoming } => {
                while !incoming.is_end_stream() {
                    match Pin::new(&mut incoming).poll_frame(&mut cx) {
                        Poll::Ready(Some(Ok(frame))) => match frame.into_data() {
                            Ok(data) => ret.extend_from_slice(&data),
                            Err(e) => error!(?e, "Failed to get data"),
                        },
                        Poll::Pending => {
                            cx.waker().wake_by_ref();
                            continue;
                        }
                        Poll::Ready(Some(Err(e))) => {
                            error!(?e, "Failed to get frame");
                            break;
                        }
                        Poll::Ready(None) => break,
                    }
                }
            }
            StreamingBody::Stream { mut stream } => loop {
                match stream.as_mut().poll_next(&mut cx) {
                    Poll::Ready(Some(data)) => ret.extend_from_slice(&data),
                    Poll::Ready(None) => break,
                    Poll::Pending => continue,
                }
            },
        }

        ret
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
            Self::Incoming { incoming } => Pin::new(incoming).poll_frame(cx),
            Self::Stream { stream } => stream
                .as_mut()
                .poll_next(cx)
                .map(|opt| opt.map(|i| Ok(Frame::data(i)))),
        }
    }

    fn is_end_stream(&self) -> bool {
        match self {
            Self::Null => true,
            Self::Bytes { bytes } => bytes.is_none(),
            Self::Incoming { incoming } => incoming.is_end_stream(),
            Self::Stream { .. } => false,
        }
    }

    fn size_hint(&self) -> SizeHint {
        match self {
            Self::Bytes { bytes: Some(bytes) } => SizeHint::with_exact(bytes.len() as _),
            Self::Incoming { incoming } => incoming.size_hint(),
            Self::Stream { stream } => {
                let (min, max) = stream.size_hint();
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

impl Default for StreamingBody {
    fn default() -> Self {
        Self::Null
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
        StreamingBody::Incoming { incoming: self }
    }
}

impl IntoStreamingBody for String {
    fn into_streaming_body(self) -> StreamingBody {
        self.into_bytes().into_streaming_body()
    }
}

impl IntoStreamingBody for Vec<u8> {
    fn into_streaming_body(self) -> StreamingBody {
        Bytes::from(self).into_streaming_body()
    }
}

impl IntoStreamingBody for Bytes {
    fn into_streaming_body(self) -> StreamingBody {
        StreamingBody::Bytes { bytes: Some(self) }
    }
}

impl IntoStreamingBody for &str {
    fn into_streaming_body(self) -> StreamingBody {
        Bytes::copy_from_slice(self.as_bytes()).into_streaming_body()
    }
}

impl IntoStreamingBody for &[u8] {
    fn into_streaming_body(self) -> StreamingBody {
        Bytes::copy_from_slice(self).into_streaming_body()
    }
}

impl<S, T> IntoStreamingBody for Pin<Box<S>>
where
    S: Stream<Item = T> + Send + Sync + 'static,
    Bytes: From<T>,
{
    fn into_streaming_body(self) -> StreamingBody {
        StreamingBody::Stream {
            stream: Box::pin(self.map(|i| i.into())),
        }
    }
}
impl IntoStreamingBody for () {
    fn into_streaming_body(self) -> StreamingBody {
        StreamingBody::Null
    }
}

impl From<StreamingBody> for String {
    fn from(value: StreamingBody) -> Self {
        String::from_utf8(value.into_vec()).unwrap_or_default()
    }
}

impl From<StreamingBody> for Vec<u8> {
    fn from(value: StreamingBody) -> Self {
        value.into_vec()
    }
}

impl<T> From<StreamingBody> for Pin<Box<dyn Stream<Item = IoResult<T>> + Send + Sync>>
where
    T: FromBytes,
{
    fn from(value: StreamingBody) -> Self {
        Box::pin(value.map(|i| i.map(FromBytes::from)))
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

/// Trait for types that can be created from `Bytes`.
///
/// Used for converting streaming body chunks into concrete types.
pub trait FromBytes {
    /// Creates this type from a `Bytes` value.
    fn from(value: Bytes) -> Self;
}

impl FromBytes for String {
    fn from(value: Bytes) -> Self {
        String::from_utf8(value.to_vec()).unwrap_or_default()
    }
}

impl FromBytes for Vec<u8> {
    fn from(value: Bytes) -> Self {
        value.into()
    }
}

impl FromBytes for Box<[u8]> {
    fn from(value: Bytes) -> Self {
        value.to_vec().into()
    }
}
