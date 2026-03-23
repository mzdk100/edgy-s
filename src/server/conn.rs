use {
    super::{super::utils::url_decode, Accessor, Command},
    hyper::{
        header::{HeaderMap, HeaderName, HeaderValue},
        http::Uri,
    },
    std::{
        collections::HashMap,
        io::Error as IoError,
        mem::take,
        net::SocketAddr,
        ops::Deref,
        str::FromStr,
        sync::{LazyLock, Weak},
    },
    tokio::{
        runtime::Runtime,
        sync::{
            Mutex,
            mpsc::{Sender as MpscSender, WeakSender},
            watch::{Sender as WatchSender, channel as watch_channel},
        },
    },
    tokio_tungstenite::tungstenite::Message,
    tracing::{error, warn},
};

pub(super) static WS_CONNS: LazyLock<Mutex<HashMap<(String, SocketAddr), MpscSender<Message>>>> =
    LazyLock::new(Default::default);

/// Base Connection Information Shared Between WebSocket and HTTP connections.
pub struct BaseConn {
    uri: Uri,
    socket_addr: SocketAddr,
    headers: HeaderMap,
}

impl BaseConn {
    /// Returns the client's socket address.
    pub fn get_addr(&self) -> SocketAddr {
        self.socket_addr
    }

    /// Returns a single URL query parameter value by name (URL decoded).
    ///
    /// # Arguments
    /// * `name` - The parameter name
    ///
    /// # Returns
    /// * `Some(String)` - The decoded parameter value (first occurrence)
    /// * `None` - Parameter not found
    ///
    /// # Example
    /// ```ignore
    /// // URL: /api/user?id=123&name=hello%20world
    /// conn.get_argument("id");   // Returns Some("123")
    /// conn.get_argument("name"); // Returns Some("hello world")
    /// conn.get_argument("age");  // Returns None
    /// ```
    pub fn get_argument(&self, name: &str) -> Option<String> {
        let query = self.query()?;
        for pair in query.split('&') {
            if let Some(eq_pos) = pair.find('=') {
                let key = &pair[..eq_pos];
                if let Ok(decoded_key) = url_decode(key)
                    && decoded_key == name
                {
                    let value = &pair[eq_pos + 1..];
                    return url_decode(value).ok();
                }
            } else if let Ok(decoded_key) = url_decode(pair)
                && decoded_key == name
            {
                // Parameter without value, e.g., ?flag
                return Some(String::new());
            }
        }
        None
    }

    /// Returns all values for a query parameter with the given name (URL decoded).
    ///
    /// Useful when the same parameter appears multiple times.
    ///
    /// # Arguments
    /// * `name` - The parameter name
    ///
    /// # Returns
    /// An iterator of decoded parameter values
    pub fn get_arguments<'a>(&'a self, name: &'a str) -> impl Iterator<Item = String> + 'a {
        self.query()
            .into_iter()
            .flat_map(|query| query.split('&'))
            .filter_map(move |pair| {
                if let Some(eq_pos) = pair.find('=') {
                    let key = &pair[..eq_pos];
                    if let Ok(decoded_key) = url_decode(key)
                        && decoded_key == name
                    {
                        url_decode(&pair[eq_pos + 1..]).ok()
                    } else {
                        None
                    }
                } else if let Ok(decoded_key) = url_decode(pair)
                    && decoded_key == name
                {
                    Some(String::new())
                } else {
                    None
                }
            })
    }

    /// Returns all query parameters as a HashMap (URL decoded).
    ///
    /// If a parameter appears multiple times, only the first value is kept.
    ///
    /// # Returns
    /// A HashMap containing all decoded parameters
    pub fn get_all_arguments(&self) -> HashMap<String, String> {
        let mut result = HashMap::new();
        if let Some(query) = self.query() {
            for pair in query.split('&') {
                if let Some(eq_pos) = pair.find('=') {
                    let key = &pair[..eq_pos];
                    let value = &pair[eq_pos + 1..];
                    if let (Ok(key), Ok(value)) = (url_decode(key), url_decode(value)) {
                        result.entry(key).or_insert(value);
                    }
                } else if !pair.is_empty()
                    && let Ok(key) = url_decode(pair)
                {
                    result.entry(key).or_insert_with(String::new);
                }
            }
        }
        result
    }

    /// Returns a request header value by name (case-insensitive).
    ///
    /// # Arguments
    /// * `name` - The header name
    ///
    /// # Returns
    /// * `Some(&str)` - The header value
    /// * `None` - Header not found
    pub fn get_header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }

    /// Returns all request headers.
    pub fn get_headers(&self) -> &HeaderMap {
        &self.headers
    }
}

impl From<(Uri, SocketAddr, HeaderMap)> for BaseConn {
    fn from((uri, socket_addr, headers): (Uri, SocketAddr, HeaderMap)) -> Self {
        Self {
            uri,
            socket_addr,
            headers,
        }
    }
}

impl Deref for BaseConn {
    type Target = Uri;

    fn deref(&self) -> &Self::Target {
        &self.uri
    }
}

/// Type alias for WebSocket connection accessor (server-side).
///
/// Provides access to `WsConn` methods for WebSocket connections.
pub type WsAccessor = Accessor<WsConn>;

impl AsRef<WsAccessor> for WsAccessor {
    fn as_ref(&self) -> &Self {
        self
    }
}

/// WebSocket connection wrapper (server-side).
///
/// Provides access to connection information and allows
/// querying other connections to the same path.
pub struct WsConn {
    inner: BaseConn,
}

impl WsConn {
    /// Returns all other connections to the same path.
    pub async fn get_other_conns(&self) -> impl Iterator<Item = WsAccessor> {
        WS_CONNS
            .lock()
            .await
            .keys()
            .filter(|(path, addr)| path == self.uri.path() && addr != &self.socket_addr)
            .map(|(_, addr)| WsConn::from((self.uri.clone(), *addr, self.headers.clone())).into())
            .collect::<Vec<_>>()
            .into_iter()
    }
}

impl From<(Uri, SocketAddr, HeaderMap)> for WsConn {
    fn from((uri, socket_addr, headers): (Uri, SocketAddr, HeaderMap)) -> Self {
        Self {
            inner: BaseConn::from((uri, socket_addr, headers)),
        }
    }
}

impl Deref for WsConn {
    type Target = BaseConn;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

/// RAII guard to ensure WebSocket connection cleanup on task abort.
pub(super) struct WsConnGuard {
    pub uri: Uri,
    pub socket_addr: SocketAddr,
    pub headers: HeaderMap,
    pub rt: Weak<Runtime>,
    pub command: WeakSender<Command>,
}

impl Drop for WsConnGuard {
    fn drop(&mut self) {
        let Some(rt) = self.rt.upgrade() else {
            warn!("Runtime already dropped.");
            return;
        };
        let Some(command) = self.command.upgrade() else {
            warn!("Command sender already dropped.");
            return;
        };

        let socket_addr = self.socket_addr;
        let headers = take(&mut self.headers);
        let uri = take(&mut self.uri);

        // Spawn a new task to handle async cleanup since Drop cannot be async
        rt.spawn(async move {
            let path = uri.path().to_owned();
            // Send close event if handler is registered
            if let Err(e) = command
                .send(Command::WsClose {
                    uri,
                    socket_addr,
                    headers,
                })
                .await
            {
                error!(?e, "Failed to send close event.");
            }

            WS_CONNS.lock().await.remove(&(path, socket_addr));
        });
    }
}

/// Type alias for HTTP connection accessor (server-side).
///
/// Provides access to `HttpConn` methods for HTTP requests.
pub type HttpAccessor = Accessor<HttpConn>;

impl AsRef<HttpAccessor> for HttpAccessor {
    fn as_ref(&self) -> &Self {
        self
    }
}

/// HTTP connection wrapper with response header support (server-side).
///
/// Allows setting response headers before the response is sent.
pub struct HttpConn {
    inner: BaseConn,
    response_headers: WatchSender<HeaderMap>,
}

impl HttpConn {
    /// Sets a response header (overwrites existing).
    ///
    /// # Arguments
    /// * `name` - The header name
    /// * `value` - The header value
    pub fn set_header(&self, name: &str, value: &str) -> Result<bool, IoError> {
        let name = HeaderName::from_str(name).map_err(IoError::other)?;
        let value = HeaderValue::from_str(value).map_err(IoError::other)?;

        Ok(self.response_headers.send_if_modified(|map| {
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

    /// Appends a response header (allows multiple values).
    ///
    /// # Arguments
    /// * `name` - The header name
    /// * `value` - The header value
    pub fn add_header(&self, name: &str, value: &str) -> Result<(), IoError> {
        let name = HeaderName::from_str(name).map_err(IoError::other)?;
        let value = HeaderValue::from_str(value).map_err(IoError::other)?;

        Ok(self.response_headers.send_modify(|map| {
            map.append(name, value);
        }))
    }
}

impl From<(Uri, SocketAddr, HeaderMap, WatchSender<HeaderMap>)> for HttpConn {
    fn from(
        (uri, socket_addr, request_headers, response_headers): (
            Uri,
            SocketAddr,
            HeaderMap,
            WatchSender<HeaderMap>,
        ),
    ) -> Self {
        Self {
            inner: (uri, socket_addr, request_headers).into(),
            response_headers,
        }
    }
}

impl From<(Uri, SocketAddr, HeaderMap)> for HttpConn {
    fn from((uri, socket_addr, headers): (Uri, SocketAddr, HeaderMap)) -> Self {
        let (tx, _) = watch_channel(Default::default());
        Self {
            inner: (uri, socket_addr, headers).into(),
            response_headers: tx,
        }
    }
}

impl Deref for HttpConn {
    type Target = BaseConn;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_base_conn(uri: &str) -> BaseConn {
        BaseConn::from((
            uri.parse().unwrap(),
            "127.0.0.1:8080".parse().unwrap(),
            HeaderMap::new(),
        ))
    }

    #[test]
    fn test_path() {
        let conn = create_base_conn("/api/user");
        assert_eq!(conn.path(), "/api/user");

        let conn = create_base_conn("/api/user?id=123");
        assert_eq!(conn.path(), "/api/user");

        let conn = create_base_conn("/api/user?id=123&name=test");
        assert_eq!(conn.path(), "/api/user");
    }

    #[test]
    fn test_query() {
        let conn = create_base_conn("/api/user");
        assert!(conn.query().is_none());

        let conn = create_base_conn("/api/user?id=123");
        assert_eq!(conn.query(), Some("id=123"));

        let conn = create_base_conn("/api/user?id=123&name=test");
        assert_eq!(conn.query(), Some("id=123&name=test"));
    }

    #[test]
    fn test_get_argument() {
        let conn = create_base_conn("/api/user?id=123&name=test");
        assert_eq!(conn.get_argument("id"), Some("123".to_string()));
        assert_eq!(conn.get_argument("name"), Some("test".to_string()));
        assert_eq!(conn.get_argument("age"), None);
    }

    #[test]
    fn test_get_argument_url_encoded() {
        let conn = create_base_conn("/search?q=hello%20world&tag=%E4%B8%AD%E6%96%87");
        assert_eq!(conn.get_argument("q"), Some("hello world".to_string()));
        assert_eq!(conn.get_argument("tag"), Some("中文".to_string()));
    }

    #[test]
    fn test_get_argument_plus_as_space() {
        let conn = create_base_conn("/search?q=hello+world");
        assert_eq!(conn.get_argument("q"), Some("hello world".to_string()));
    }

    #[test]
    fn test_get_arguments_multiple_values() {
        let conn = create_base_conn("/api?tag=foo&tag=bar&tag=baz");
        let values: Vec<_> = conn.get_arguments("tag").collect();
        assert_eq!(values, vec!["foo", "bar", "baz"]);
    }

    #[test]
    fn test_get_all_arguments() {
        let conn = create_base_conn("/api?id=123&name=hello%20world&flag");
        let args = conn.get_all_arguments();
        assert_eq!(args.get("id"), Some(&"123".to_string()));
        assert_eq!(args.get("name"), Some(&"hello world".to_string()));
        assert_eq!(args.get("flag"), Some(&String::new()));
    }

    #[test]
    fn test_empty_values() {
        let conn = create_base_conn("/api?empty=&flag");
        assert_eq!(conn.get_argument("empty"), Some(String::new()));
        assert_eq!(conn.get_argument("flag"), Some(String::new()));
    }

    #[test]
    fn test_special_characters_in_key() {
        let conn = create_base_conn("/api?%24key=value");
        assert_eq!(conn.get_argument("$key"), Some("value".to_string()));
    }
}
