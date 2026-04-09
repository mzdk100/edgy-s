mod binding;
mod builder;
mod caller;
mod command;
mod conn;
mod handler;

use tokio::sync::RwLock;
use {
    super::types::{
        Accessor, HttpServerAsyncFn, HttpServerRouter, IntoStreamingBody, Packet, State,
        StreamingBody, WsAsyncFn, WsRouter,
    },
    binding::{HttpBinding, WsBinding},
    builder::EdgyServiceBuilder,
    caller::BIND_SENDERS,
    command::Command,
    conn::{HttpConn, WS_CONNS, WsConn},
    futures_util::future::{AbortHandle, Abortable},
    handler::WebHandler,
    hyper::{header::HeaderMap, http::Uri, server::conn::http1::Builder as Http1Builder},
    hyper_util::rt::TokioIo,
    serde::{Deserialize, Serialize},
    std::{
        collections::{HashMap, hash_map::Entry},
        io::{Error as IoError, ErrorKind, Result as IoResult},
        net::SocketAddr,
        ops::Deref,
        sync::Arc,
    },
    tokio::{
        net::{TcpListener, ToSocketAddrs, lookup_host},
        runtime::Runtime,
        select,
        sync::{
            mpsc::{Receiver as MpscReceiver, Sender as MpscSender, channel as mpsc_channel},
            oneshot::{Sender as OneshotSender, channel as oneshot_channel},
            watch::channel as watch_channel,
        },
        task::JoinHandle,
    },
    tokio_util::sync::CancellationToken,
    tracing::{error, info},
};

pub use {
    caller::WsCaller,
    conn::{HttpAccessor, WsAccessor},
};

/// WebSocket/HTTP server for handling incoming connections and requests.
///
/// The service provides both HTTP request routing and WebSocket connection management
/// with support for bidirectional communication.
///
/// # Type Parameters
/// - `S`: Shared state type (defaults to `()`)
///
/// # Example
/// ```no_run
/// use edgy_s::server::EdgyService;
///
/// #[tokio::main]
/// async fn main() -> std::io::Result<()> {
///     let service = EdgyService::builder("0.0.0.0:8080")
///         .workers(2)
///         .build()
///         .await?;
///     
///     service.run().await
/// }
/// ```
pub struct EdgyService<S = ()> {
    bind_addr: SocketAddr,
    rt: Arc<Runtime>,
    command: MpscSender<Command>,
    worker_task: JoinHandle<IoResult<()>>,
    state: State<S>,
}

impl<S> WsRouter<WsConn<S>, S> for EdgyService<S>
where
    S: Send + Sync + 'static,
{
    type Binding = WsBinding<HttpConn<S>, WsConn<S>>;

    async fn add_route<F, P, Args, Ret>(&self, path: P, handler: F) -> IoResult<Self::Binding>
    where
        F: WsAsyncFn<Args, Ret, WsConn<S>, S>,
        Args: for<'a> Deserialize<'a> + Serialize,
        Ret: for<'a> Deserialize<'a> + Serialize + Send,
        P: AsRef<str>,
    {
        let (stream_tx, mut stream_rx) = mpsc_channel(2);
        let (open_tx, open_rx) = mpsc_channel(2);
        let (close_tx, close_rx) = mpsc_channel(2);
        let (ret_tx, ret_rx) = oneshot_channel();
        let command = self.command.clone();
        let state = self.state.clone();
        command
            .send(Command::AddWsRoute {
                path: path.as_ref().into(),
                stream: stream_tx,
                opt_return: ret_tx,
                open: open_tx,
                close: close_tx,
            })
            .await
            .map_err(IoError::other)?;
        {
            let mut lock = BIND_SENDERS.lock().await;
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
        let state2 = state.clone();
        self.rt.spawn(async move {
            while let Some((uri, socket_addr, headers, msg, ret_tx)) = stream_rx.recv().await {
                let packet = match Packet::<Args, Ret>::from_message(&msg) {
                    Ok(args) => args,
                    Err(e) => {
                        error!(?e, "Unable to decode packet.");
                        continue;
                    }
                };

                match packet {
                    Packet::Call(id, args) => {
                        let accessor =
                            WsConn::from((uri, socket_addr, headers, state.clone())).into();
                        let ret = match Packet::<Args, Ret>::make_ret_message(
                            id,
                            handler.call(accessor, args).await,
                        ) {
                            Ok(r) => r,
                            Err(e) => {
                                error!(?e, "Unable to encode message.");
                                continue;
                            }
                        };
                        if let Err(e) = ret_tx.send(Some(ret)) {
                            error!(?e, "Unable to send data.");
                        }
                    }

                    Packet::Ret(id, _ret) => {
                        if let Err(e) = command
                            .send(Command::CommitReturn {
                                path: path2.clone(),
                                socket_addr,
                                id,
                                msg,
                            })
                            .await
                        {
                            error!(?e, "Unable to commit return.");
                        }
                        if let Err(e) = ret_tx.send(None) {
                            error!(?e, "Unable to send data.");
                        }
                    }
                }
            }
        });
        ret_rx.await.map_err(IoError::other)??;

        WsBinding::new(
            path,
            self.command.downgrade(),
            Arc::downgrade(&self.rt),
            open_rx,
            close_rx,
            state2,
        )
    }

    async fn remove_route(binding: Self::Binding) -> IoResult<()> {
        let path = binding.get_path();
        BIND_SENDERS.lock().await.remove(path);
        let (ret_tx, ret_rx) = oneshot_channel();
        binding
            .send_command(Command::RemoveWsRoute {
                path: path.into(),
                opt_return: ret_tx,
            })
            .await?;
        ret_rx.await.map_err(IoError::other)?
    }
}

impl<S> HttpServerRouter<HttpConn<S>, S> for EdgyService<S>
where
    S: Send + Sync + 'static,
{
    type Binding = HttpBinding<S>;

    async fn add_route<F, P, Body, Ret>(&self, path: P, handler: F) -> IoResult<Self::Binding>
    where
        F: HttpServerAsyncFn<Body, Ret, HttpConn<S>, S>,
        Body: From<StreamingBody>,
        P: AsRef<str>,
        Ret: IntoStreamingBody,
    {
        let (req_tx, mut req_rx) = mpsc_channel::<(
            Uri,
            SocketAddr,
            HeaderMap,
            StreamingBody,
            OneshotSender<_>,
            CancellationToken,
        )>(2);
        let state = self.state.clone();
        let task = self.rt.spawn(async move {
            while let Some((uri, addr, headers, body, ret_tx, cancel_token)) = req_rx.recv().await {
                let (tx, rx) = watch_channel(Default::default());
                let accessor = HttpConn::from((uri, addr, headers, state.clone(), tx)).into();

                // Create abortable future so we can cancel the handler
                let (abort_handle, abort_registration) = AbortHandle::new_pair();
                let abortable_future =
                    Abortable::new(handler.call(accessor, body.into()), abort_registration);

                let ret = select! {
                    biased;
                    _ = cancel_token.cancelled() => {
                        // Connection was canceled, abort the handler
                        abort_handle.abort();
                        continue;
                    }
                    result = abortable_future => {
                        match result {
                            Ok(r) => r,
                            Err(_) => {
                                // Handler was aborted
                                continue;
                            }
                        }
                    }
                };

                let response_headers = rx.borrow().clone();
                if let Err(e) = ret_tx.send((response_headers, ret.into_streaming_body())) {
                    error!(?e, "Unable to send data.");
                }
            }
        });

        let (ret_tx, ret_rx) = oneshot_channel();
        self.command
            .send(Command::AddHttpRoute {
                task,
                path: path.as_ref().to_owned(),
                req_tx,
                opt_return: ret_tx,
            })
            .await
            .map_err(IoError::other)?;
        ret_rx.await.map_err(IoError::other)??;

        Ok(HttpBinding::new(path, self.command.downgrade()))
    }

    async fn remove_route(binding: Self::Binding) -> IoResult<()> {
        let (ret_tx, ret_rx) = oneshot_channel();
        let path = binding.get_path();
        binding
            .send_command(Command::RemoveHttpRoute {
                path: path.into(),
                opt_return: ret_tx,
            })
            .await?;
        ret_rx.await.map_err(IoError::other)?
    }
}

impl<S> EdgyService<S> {
    /// Creates a new `EdgyService` with the given shared state.
    pub async fn with_state<Addr>(bind_addr: Addr, state: S) -> IoResult<Self>
    where
        Addr: ToSocketAddrs,
        S: Send + Sync + 'static,
    {
        Self::builder_with_state(bind_addr, state).build().await
    }

    /// Creates a builder for configuring the service with the given state.
    pub fn builder_with_state<Addr>(bind_addr: Addr, state: S) -> EdgyServiceBuilder<Addr, S>
    where
        Addr: ToSocketAddrs,
    {
        EdgyServiceBuilder::new(bind_addr, state)
    }

    /// Runs the server, accepting incoming connections until aborted.
    ///
    /// This method blocks and continuously accepts new connections.
    /// Each connection is handled in a separate async task.
    /// The server runs until the internal worker task is finished or aborted.
    pub async fn run(self) -> IoResult<()>
    where
        S: Send + Sync + 'static,
    {
        let listener = TcpListener::bind(&self.bind_addr).await?;
        info!("WebSocket server listening on {}", self.bind_addr);
        while !self.worker_task.is_finished() {
            let (stream, addr) = listener.accept().await?;
            info!("Accept connection from {}", addr);
            let command_tx = self.command.clone();
            let rt = Arc::downgrade(&self.rt);

            self.rt.spawn(async move {
                if let Err(e) = Http1Builder::new()
                    .serve_connection(TokioIo::new(stream), WebHandler::new(addr, command_tx, rt))
                    .with_upgrades()
                    .await
                {
                    error!(?e, "Connection error.");
                };
            });
        }

        Ok(())
    }

    /// Aborts the server and all background tasks immediately.
    ///
    /// This will terminate all active connections and stop accepting
    /// new connections. Use this for graceful shutdown.
    pub fn abort(self) {
        self.worker_task.abort();
    }

    async fn worker(mut command: MpscReceiver<Command>) -> IoResult<()> {
        let mut http_handlers = HashMap::new();
        let mut ws_handlers = HashMap::new();
        let mut pending_requests = HashMap::new();

        while let Some(item) = command.recv().await {
            match item {
                Command::AddWsRoute {
                    path,
                    stream,
                    opt_return,
                    open,
                    close
                } => {
                    opt_return
                        .send(match ws_handlers.entry(path) {
                            Entry::Occupied(e) => {
                                Err(IoError::other(format!("Can't add route: {}", e.key())))
                            }
                            Entry::Vacant(e) => {
                                e.insert((stream, open, close));
                                Ok(())
                            }
                        })
                        .map_or_else(|e| e.map_err(IoError::other), Ok)?;
                }

                Command::RemoveWsRoute { path, opt_return } => {
                    opt_return
                        .send(ws_handlers.remove(&path).map_or(
                            Err(IoError::other(format!("Can't remove route: {}", path))),
                            |_| Ok(()),
                        ))
                        .map_or_else(|e| e, Ok)?;
                }

                Command::AddHttpRoute {
                    task,
                    path,
                    req_tx,
                    opt_return
                } => {
                    opt_return
                        .send(match http_handlers.entry(path) {
                            Entry::Occupied(e) => {
                                Err(IoError::other(format!("Can't add route: {}", e.key())))
                            }
                            Entry::Vacant(e) => {
                                e.insert((req_tx, task));
                                Ok(())
                            }
                        })
                        .map_or_else(|e| e.map_err(IoError::other), Ok)?;
                }

                Command::RemoveHttpRoute { path, opt_return } => {
                    opt_return
                        .send(http_handlers.remove(&path).map_or(
                            Err(IoError::other(format!("Can't remove route: {}", path))),
                            |i| { i.1.abort(); Ok(()) },
                        ))
                        .map_or_else(|e| e, Ok)?;
                }

                Command::Transfer {
                    uri,
                    socket_addr,
                    msg,
                    headers,
                    ret_tx,
                } => {
                    if let Some((handler, _, _)) = ws_handlers.get(uri.path()) {
                        handler.send((uri, socket_addr, headers, msg, ret_tx)).await.map_or_else(|e| Err(IoError::other(e)), Ok)?;
                    }
                }

                Command::CallRemotely {
                    path,
                    socket_addr,
                    id,
                    msg,
                    ret_tx,
                } => {
                    if let Some(conn) = WS_CONNS.lock().await.get(&(path.clone(), socket_addr)) {
                        if let Err(e) = conn.sender.send(msg).await {
                            error!(?e, "Unable to send message.");
                        }
                        pending_requests.insert((path.clone(), socket_addr, id), ret_tx);
                    } else if let Err(e)
                        =ret_tx.send(Err(IoError::other(format!("The connection to the `{}` address may have been closed, or the address does not belong to a connection at the `{}` path and cannot handle this request.", socket_addr, path)))) {
                        error!(?e, "Unable to send error message.");
                    }
                }

                Command::CommitReturn {
                    path,
                    socket_addr,
                    id,
                    msg,
                } => {
                    if let Some(tx) = pending_requests.remove(&(path, socket_addr, id))
                        && let Err(e) = tx.send(Ok(msg))
                    {
                        error!(?e, "Unable to commit return message.");
                    }
                }

                Command::WsOpen { uri, socket_addr, headers, res_tx: response_tx } => {
                    if let Some((_, open, _)) = ws_handlers.get(uri.path()) {
                        let (headers_tx, headers_rx) = watch_channel(Default::default());
                        let (ret_tx, ret_rx) = oneshot_channel();
                        if let Err(e) = open.send((uri, socket_addr, headers, headers_tx, ret_tx)).await {
                            error!(?e, "Unable to open connection.");
                        } else if let Err(e) = ret_rx.await {
                            error!(?e, "Unable to send the response.");
                        } else if let Err(e) = response_tx.send(headers_rx.borrow().clone()) {
                            error!(?e, "Unable to open connection.");
                        }
                    };
                }

                Command::WsClose {uri, socket_addr, headers} => if let Some((_, _, close)) = ws_handlers.get(uri.path())
                    && let Err(e) = close.send((uri, socket_addr, headers, )).await {
                        error!(?e, "Unable to close connection.");
                    }

                Command::Request {uri, socket_addr, headers, body, ret_tx, cancel_token} => {
                    if let Some((handler, _)) = http_handlers.get(uri.path()) {
                        handler.send((uri, socket_addr, headers, body, ret_tx, cancel_token)).await.map_or_else(|e| Err(IoError::other(e)), Ok)?;
                    }
                }
            }
        }

        Ok(())
    }

    async fn get_bind_addr<Addr>(bind_addr: Addr) -> IoResult<SocketAddr>
    where
        Addr: ToSocketAddrs,
    {
        lookup_host(bind_addr).await?.next().ok_or(IoError::new(
            ErrorKind::NotFound,
            "Unable to obtain host address.",
        ))
    }
}

impl EdgyService<()> {
    /// Creates a new `EdgyService` with default settings and no state.
    pub async fn new<Addr>(bind_addr: Addr) -> IoResult<Self>
    where
        Addr: ToSocketAddrs,
    {
        Self::builder(bind_addr).build().await
    }

    /// Creates a builder for configuring the service without state.
    pub fn builder<Addr>(bind_addr: Addr) -> EdgyServiceBuilder<Addr, ()>
    where
        Addr: ToSocketAddrs,
    {
        EdgyServiceBuilder::new(bind_addr, ())
    }
}

impl<S> Deref for EdgyService<S> {
    type Target = RwLock<S>;

    fn deref(&self) -> &Self::Target {
        &self.state
    }
}
