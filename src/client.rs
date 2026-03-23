mod binding;
mod caller;
mod command;
mod conn;
mod handler;
mod request;

use {
    super::{
        types::{Accessor, HttpClientAsyncFn, HttpClientRouter, WsAsyncFn, WsRouter},
        utils::build_uri,
    },
    binding::{HttpBinding, WsBinding},
    caller::WS_BINDING_SENDERS,
    command::Command,
    conn::{RequestConn, ResponseConn},
    handler::{HttpCall, http_dispatch, ws_dispatch_with_auto_reconnection},
    hyper::http::Uri,
    request::HTTP_BINDING_SENDERS,
    serde::{Deserialize, Serialize},
    std::{
        collections::HashMap,
        io::{Error as IoError, ErrorKind, Result as IoResult},
        sync::Arc,
        time::Duration,
    },
    tokio::{
        runtime::{Builder, Runtime},
        sync::{
            mpsc::{Receiver as MpscReceiver, Sender as MpscSender, channel as mpsc_channel},
            oneshot::{Sender as OneshotSender, channel as oneshot_channel},
        },
        task::JoinHandle,
    },
    tracing::{error, info},
};
pub use {
    caller::WsCaller,
    conn::{HttpAccessor, RequestAccessor, WsAccessor},
    request::{HttpDelete, HttpGet, HttpHead, HttpPatch, HttpPost, HttpPut},
};

/// Default configuration values
const DEFAULT_NUM_WORKERS: usize = 4;
const DEFAULT_MAX_RETRIES: usize = 3;
const DEFAULT_RETRY_INTERVAL_MS: u64 = 1000;

/// Builder for creating `EdgyClient` with custom configuration.
///
/// # Example
/// ```no_run
/// use edgy_s::EdgyClient;
///
/// let client = EdgyClient::builder("ws://localhost")
///     .workers(2)
///     .max_retries(5)
///     .retry_interval_ms(500)
///     .build()?;
/// ```
pub struct EdgyClientBuilder {
    base_url: Uri,
    num_workers: usize,
    max_retries: usize,
    retry_interval: Duration,
}

impl EdgyClientBuilder {
    /// Sets the number of worker threads for the async runtime.
    pub fn workers(mut self, num: usize) -> Self {
        self.num_workers = num;
        self
    }

    /// Sets the maximum number of reconnection attempts for WebSocket connections.
    pub fn max_retries(mut self, num: usize) -> Self {
        self.max_retries = num;
        self
    }

    /// Sets the retry interval in milliseconds between reconnection attempts.
    pub fn retry_interval_ms(mut self, ms: u64) -> Self {
        self.retry_interval = Duration::from_millis(ms);
        self
    }

    /// Sets the retry interval as a Duration.
    pub fn retry_interval(mut self, duration: Duration) -> Self {
        self.retry_interval = duration;
        self
    }

    /// Builds the `EdgyClient` with the configured settings.
    pub fn build(self) -> IoResult<EdgyClient> {
        let rt = Builder::new_multi_thread()
            .worker_threads(self.num_workers)
            .enable_all()
            .build()?;
        let (tx, rx) = mpsc_channel(2);
        let task = rt.spawn(EdgyClient::worker(rx));

        Ok(EdgyClient {
            base_url: self.base_url,
            rt: rt.into(),
            command: tx,
            task: Some(task),
            max_retries: self.max_retries,
            retry_interval: self.retry_interval,
        })
    }
}

/// HTTP/WebSocket client for making requests and establishing WebSocket connections.
///
/// The client provides both HTTP request routing and WebSocket connection management
/// with automatic reconnection support.
///
/// # Example
/// ```no_run
/// use edgy_s::EdgyClient;
///
/// #[tokio::main]
/// async fn main() -> std::io::Result<()> {
///     let client = EdgyClient::builder("ws://localhost:8080")
///         .workers(2)
///         .max_retries(5)
///         .build()?;
///     
///     client.run().await
/// }
/// ```
pub struct EdgyClient {
    base_url: Uri,
    rt: Arc<Runtime>,
    command: MpscSender<Command>,
    task: Option<JoinHandle<IoResult<()>>>,
    max_retries: usize,
    retry_interval: Duration,
}

impl WsRouter<ResponseConn> for EdgyClient {
    type Binding = WsBinding<RequestConn, ResponseConn>;

    async fn add_route<F, P, Args, Ret>(&self, path: P, handler: F) -> IoResult<Self::Binding>
    where
        F: WsAsyncFn<Args, Ret, ResponseConn>,
        Args: for<'a> Deserialize<'a> + Serialize + 'static,
        Ret: for<'a> Deserialize<'a> + Serialize + 'static,
        P: AsRef<str>,
    {
        let uri = build_uri(&self.base_url, &path, None)?;
        info!("Connect to {}", uri);

        let (request_tx, request_rx) = oneshot_channel();
        let (open_tx, open_rx) = oneshot_channel();
        let (close_tx, close_rx) = oneshot_channel();
        {
            let mut lock = WS_BINDING_SENDERS.lock().await;
            if lock.contains_key(path.as_ref()) {
                return Err(IoError::other(format!(
                    "Can't bind to route, `{}` path already exists.",
                    path.as_ref()
                )));
            }
            lock.insert(path.as_ref().into(), self.command.downgrade());
        }
        let path = path.as_ref().to_owned();
        let path2 = path.clone();

        let (call_tx, call_rx) = mpsc_channel(2);
        let max_retries = self.max_retries;
        let retry_interval = self.retry_interval;
        let task = self.rt.spawn(async move {
            ws_dispatch_with_auto_reconnection(
                uri,
                request_tx,
                call_rx,
                handler,
                open_tx,
                close_tx,
                max_retries,
                retry_interval,
            )
            .await
        });

        let (ret_tx, ret_rx) = oneshot_channel();
        self.command
            .send(Command::AddWsRoute {
                path: path2,
                stream: call_tx,
                task,
                opt_return: ret_tx,
            })
            .await
            .map_err(IoError::other)?;
        ret_rx.await.map_err(IoError::other)??;

        WsBinding::new(
            path,
            self.command.downgrade(),
            Arc::downgrade(&self.rt),
            request_rx,
            open_rx,
            close_rx,
        )
    }

    async fn remove_route(binding: Self::Binding) -> IoResult<()> {
        let path = binding.get_path();
        WS_BINDING_SENDERS.lock().await.remove(path);
        let (ret_tx, ret_rx) = oneshot_channel();
        binding
            .send_command(Command::RemoveWsRoute {
                path: path.into(),
                opt_return: ret_tx,
            })
            .await?;
        ret_rx.await.map_err(IoError::other)??;

        Ok(())
    }
}

impl HttpClientRouter<RequestConn> for EdgyClient {
    type Binding = HttpBinding;

    async fn add_route<F, P>(&self, path: P, _handler: F) -> IoResult<Self::Binding>
    where
        F: HttpClientAsyncFn<RequestConn>,
        P: AsRef<str>,
    {
        let (request_tx, request_rx) = mpsc_channel(16);
        let task = self.rt.spawn(http_dispatch(request_rx));

        {
            let mut lock = HTTP_BINDING_SENDERS.lock().await;
            if lock.contains_key(path.as_ref()) {
                task.abort();
                return Err(IoError::other(format!(
                    "Can't bind to route, `{}` path already exists.",
                    path.as_ref()
                )));
            }
            lock.insert(
                path.as_ref().into(),
                request::HttpBindingConfig {
                    sender: request_tx,
                    base_url: self.base_url.clone(),
                    max_retries: self.max_retries,
                    retry_interval: self.retry_interval,
                },
            );
        }

        Ok(HttpBinding::new(path, self.command.downgrade(), task))
    }

    async fn remove_route(binding: Self::Binding) -> IoResult<()> {
        let path = binding.get_path();
        HTTP_BINDING_SENDERS.lock().await.remove(path).ok_or({
            IoError::other(format!(
                "Can't remove route, `{}` path doesn't exists.",
                path
            ))
        })?;

        Ok(())
    }
}

impl EdgyClient {
    /// Creates a new `EdgyClient` with default settings.
    pub fn new<U>(base_url: U) -> IoResult<Self>
    where
        U: AsRef<str>,
    {
        Self::builder(base_url)?.build()
    }

    /// Creates a builder for configuring the client.
    pub fn builder<U>(base_url: U) -> IoResult<EdgyClientBuilder>
    where
        U: AsRef<str>,
    {
        Ok(EdgyClientBuilder {
            base_url: base_url.as_ref().parse().map_err(IoError::other)?,
            num_workers: DEFAULT_NUM_WORKERS,
            max_retries: DEFAULT_MAX_RETRIES,
            retry_interval: Duration::from_millis(DEFAULT_RETRY_INTERVAL_MS),
        })
    }

    /// Runs the client until all tasks complete or an error occurs.
    ///
    /// This method blocks until the internal worker task finishes.
    /// The client will continue processing requests and handling
    /// WebSocket connections until explicitly aborted.
    pub async fn run(mut self) -> IoResult<()> {
        if let Some(task) = self.task.take() {
            task.await.map_err(IoError::other)??;
        }

        Ok(())
    }

    async fn worker(mut command: MpscReceiver<Command>) -> IoResult<()> {
        let mut tasks = HashMap::new();

        while let Some(item) = command.recv().await {
            match item {
                Command::AddWsRoute {
                    path,
                    stream,
                    task,
                    opt_return,
                } => {
                    opt_return
                        .send(if tasks.contains_key(&path) {
                            Err(IoError::other(format!("Can't add route: {}", path)))
                        } else {
                            tasks.insert(path, (stream, task));
                            Ok(())
                        })
                        .map_or_else(|e| e.map_err(IoError::other), Ok)?;
                }

                Command::RemoveWsRoute { path, opt_return } => opt_return
                    .send(tasks.remove(&path).map_or(
                        Err(IoError::other(format!("Can't remove route: {}", path))),
                        |(_, i)| Ok(i.abort()),
                    ))
                    .map_or_else(|e| e, Ok)?,

                Command::CallRemotely {
                    path,
                    id,
                    msg,
                    opt_return,
                } => {
                    if let Some((sender, _)) = tasks.get(path.as_str()) {
                        let (tx, rx) = oneshot_channel();
                        sender.send((id, msg, tx)).await.map_err(IoError::other)?;
                        if let Err(e) = opt_return.send(rx.await.map_err(IoError::other)) {
                            error!(?e, "Can't send the message.");
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Aborts the client and all background tasks immediately.
    ///
    /// This will terminate all active connections and stop processing
    /// any pending requests. Use this for graceful shutdown.
    pub fn abort(self) {
        if let Some(task) = self.task {
            task.abort();
        }
    }
}
