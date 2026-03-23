use {
    super::{Accessor, OneshotSender},
    hyper::{
        header::{HeaderMap, HeaderName, HeaderValue},
        http::{StatusCode, Uri},
    },
    std::{collections::HashMap, io::Error as IoError, ops::Deref, str::FromStr},
    tokio::sync::watch::Sender as WatchSender,
    tracing::{error, info},
};

/// RAII guard to ensure close signal is sent when the reconnection loop exits.
pub(super) struct WsConnGuard {
    pub(super) open_tx: Option<OneshotSender<WsAccessor>>,
    close_tx: Option<OneshotSender<WsAccessor>>,
    uri: Uri,
}

impl WsConnGuard {
    pub(super) fn new(
        open_tx: OneshotSender<WsAccessor>,
        close_tx: OneshotSender<WsAccessor>,
        uri: Uri,
    ) -> Self {
        Self {
            open_tx: open_tx.into(),
            close_tx: close_tx.into(),
            uri,
        }
    }
}

impl Drop for WsConnGuard {
    fn drop(&mut self) {
        if let Some(close_tx) = self.close_tx.take()
            && self.open_tx.is_none()
        {
            let accessor: ResponseConn = self.uri.clone().into();
            if let Err(e) = close_tx.send(accessor.into()) {
                error!(?e, "Failed to send close signal.");
            }
            info!("WebSocket connection closed: {}", self.uri);
        }
    }
}

/// Base connection information for client-side connections.
///
/// Provides access to the request URI via `Deref`.
#[derive(Debug)]
pub struct BaseConn {
    uri: Uri,
}

impl From<Uri> for BaseConn {
    fn from(value: Uri) -> Self {
        Self { uri: value }
    }
}

impl Deref for BaseConn {
    type Target = Uri;

    fn deref(&self) -> &Self::Target {
        &self.uri
    }
}

/// Response connection wrapper for client-side responses.
///
/// Provides access to response status code and headers.
#[derive(Debug)]
pub struct ResponseConn {
    base: BaseConn,
    status: StatusCode,
    response_headers: HeaderMap,
}

impl ResponseConn {
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

impl Deref for ResponseConn {
    type Target = BaseConn;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl From<Uri> for ResponseConn {
    fn from(value: Uri) -> Self {
        Self {
            base: value.into(),
            status: StatusCode::default(),
            response_headers: Default::default(),
        }
    }
}

impl From<(Uri, StatusCode, HeaderMap)> for ResponseConn {
    fn from((uri, status, response_headers): (Uri, StatusCode, HeaderMap)) -> Self {
        Self {
            base: uri.into(),
            status,
            response_headers,
        }
    }
}

/// Request connection wrapper for client-side requests.
///
/// Allows setting request headers and query parameters before
/// the request is sent to the server.
#[derive(Debug)]
pub struct RequestConn {
    base: BaseConn,
    request_headers: WatchSender<HeaderMap>,
    query_params: WatchSender<HashMap<String, String>>,
}

impl Deref for RequestConn {
    type Target = BaseConn;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl
    From<(
        Uri,
        WatchSender<HeaderMap>,
        WatchSender<HashMap<String, String>>,
    )> for RequestConn
{
    fn from(
        (uri, request_headers, query_params): (
            Uri,
            WatchSender<HeaderMap>,
            WatchSender<HashMap<String, String>>,
        ),
    ) -> Self {
        Self {
            base: uri.into(),
            request_headers,
            query_params,
        }
    }
}

impl RequestConn {
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
pub type WsAccessor = Accessor<ResponseConn>;

/// Type alias for HTTP connection accessor (client-side).
///
/// Provides access to `ResponseConn` methods for HTTP responses.
pub type HttpAccessor = Accessor<ResponseConn>;

/// Type alias for request connection accessor (client-side).
///
/// Provides access to `RequestConn` methods for setting up requests.
pub type RequestAccessor = Accessor<RequestConn>;
