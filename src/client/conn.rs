use {
    super::{
        super::types::{Ref, RefMut, State},
        Accessor, OneshotSender,
    },
    hyper::{
        header::{HeaderMap, HeaderName, HeaderValue},
        http::{StatusCode, Uri},
    },
    std::{
        collections::HashMap, fmt::Debug, io::Error as IoError, ops::Deref, str::FromStr, sync::Arc,
    },
    tokio::sync::watch::Sender as WatchSender,
    tracing::{error, info},
};

/// RAII guard to ensure close signal is sent when the reconnection loop exits.
pub(super) struct WsConnGuard<S = ()>
where
    S: Debug,
{
    pub close_tx: Option<(OneshotSender<HttpAccessor<S>>, WsAccessor<S>)>,
}

impl<S> WsConnGuard<S>
where
    S: Debug,
{
    pub(super) fn new() -> Self {
        Self { close_tx: None }
    }
}

impl<S> Drop for WsConnGuard<S>
where
    S: Debug,
{
    fn drop(&mut self) {
        if let Some((close_tx, accessor)) = self.close_tx.take() {
            info!("WebSocket connection closed: {}", accessor.as_uri());
            if let Err(e) = close_tx.send(accessor) {
                error!(?e, "Failed to send close signal.");
            }
        }
    }
}

/// Base connection information for client-side connections.
///
/// Provides access to the request URI via `Deref`.
#[derive(Clone, Debug)]
pub struct BaseConn<S = ()> {
    state: State<S>,
    uri: Arc<Uri>,
}

impl<S> BaseConn<S> {
    /// Returns a reference to the shared state.
    pub async fn borrow(&self) -> Ref<'_, S> {
        Ref::new(self.state.read().await)
    }

    /// Returns a clone of the state Arc.
    pub async fn borrow_mut(&self) -> RefMut<'_, S> {
        RefMut::new(self.state.write().await)
    }
}

impl<S> From<(Uri, State<S>)> for BaseConn<S> {
    fn from((uri, state): (Uri, State<S>)) -> Self {
        Self {
            uri: uri.into(),
            state,
        }
    }
}

impl<S> Deref for BaseConn<S> {
    type Target = Uri;

    fn deref(&self) -> &Self::Target {
        &self.uri
    }
}

/// Response connection wrapper for client-side responses.
///
/// Provides access to response status code and headers.
#[derive(Clone, Debug)]
pub struct ResponseConn<S = ()> {
    base: BaseConn<S>,
    status: StatusCode,
    response_headers: HeaderMap,
}

impl<S> ResponseConn<S> {
    fn as_uri(&self) -> &Uri {
        &self.base.uri
    }

    /// Returns the HTTP status code.
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// Returns a response header value by name (case-insensitive).
    ///
    /// # Arguments
    /// * `name` - The header name
    ///
    /// # Returns
    /// * `Some(&str)` - The header value
    /// * `None` - Header not found
    pub fn get_header(&self, name: &str) -> Option<&str> {
        self.response_headers
            .get(name)
            .and_then(|v| v.to_str().ok())
    }

    /// Returns all response headers.
    pub fn get_headers(&self) -> &HeaderMap {
        &self.response_headers
    }
}

impl<S> Deref for ResponseConn<S> {
    type Target = BaseConn<S>;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<S> From<(Uri, StatusCode, HeaderMap, State<S>)> for ResponseConn<S> {
    fn from(
        (uri, status, response_headers, state): (Uri, StatusCode, HeaderMap, State<S>),
    ) -> Self {
        Self {
            base: (uri, state).into(),
            status,
            response_headers,
        }
    }
}

/// Request connection wrapper for client-side requests.
///
/// Allows setting request headers and query parameters before
/// the request is sent to the server.
#[derive(Clone, Debug)]
pub struct RequestConn<S = ()> {
    base: BaseConn<S>,
    request_headers: WatchSender<HeaderMap>,
    query_params: WatchSender<HashMap<String, String>>,
}

impl<S> Deref for RequestConn<S> {
    type Target = BaseConn<S>;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<S>
    From<(
        Uri,
        WatchSender<HeaderMap>,
        WatchSender<HashMap<String, String>>,
        State<S>,
    )> for RequestConn<S>
{
    fn from(
        (uri, request_headers, query_params, state): (
            Uri,
            WatchSender<HeaderMap>,
            WatchSender<HashMap<String, String>>,
            State<S>,
        ),
    ) -> Self {
        Self {
            base: (uri, state).into(),
            request_headers,
            query_params,
        }
    }
}

impl<S> RequestConn<S> {
    /// Sets a request header (overwrites existing).
    ///
    /// # Arguments
    /// * `name` - The header name
    /// * `value` - The header value
    pub fn set_header(&self, name: &str, value: &str) -> Result<bool, IoError> {
        let name = HeaderName::from_str(name).map_err(IoError::other)?;
        let value = HeaderValue::from_str(value).map_err(IoError::other)?;

        Ok(self.request_headers.send_if_modified(|map| {
            if let Some(v) = map.get(&name)
                && v == &value
            {
                false
            } else {
                map.insert(name, value);
                true
            }
        }))
    }

    /// Appends a request header (allows multiple values).
    ///
    /// # Arguments
    /// * `name` - The header name
    /// * `value` - The header value
    pub fn add_header(&self, name: &str, value: &str) -> Result<(), IoError> {
        let name = HeaderName::from_str(name).map_err(IoError::other)?;
        let value = HeaderValue::from_str(value).map_err(IoError::other)?;

        Ok(self.request_headers.send_modify(|map| {
            map.append(name, value);
        }))
    }

    /// Sets a query parameter (overwrites existing).
    ///
    /// # Arguments
    /// * `name` - The parameter name
    /// * `value` - The parameter value
    pub fn set_argument(&self, name: &str, value: &str) -> bool {
        self.query_params.send_if_modified(|params| {
            if let Some(v) = params.get(name)
                && v == value
            {
                false
            } else {
                params.insert(name.to_owned(), value.to_owned());
                true
            }
        })
    }
}

/// Type alias for WebSocket connection accessor (client-side).
///
/// Provides access to `ResponseConn` methods for WebSocket connections.
pub type WsAccessor<S = ()> = Accessor<ResponseConn<S>>;

/// Type alias for HTTP connection accessor (client-side).
///
/// Provides access to `ResponseConn` methods for HTTP responses.
pub type HttpAccessor<S = ()> = Accessor<ResponseConn<S>>;

/// Type alias for request connection accessor (client-side).
///
/// Provides access to `RequestConn` methods for setting up requests.
pub type RequestAccessor<S = ()> = Accessor<RequestConn<S>>;
