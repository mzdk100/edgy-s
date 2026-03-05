use {
    super::types::{Accessor, AsyncFun, AtomicReqId, Packet, ReqId, Router},
    futures_util::{
        SinkExt, StreamExt,
        future::{Either, select},
    },
    hyper::{
        Request, Response, StatusCode, Version,
        body::Incoming,
        header::{
            CONNECTION, HeaderMap, HeaderValue, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY,
            SEC_WEBSOCKET_VERSION, UPGRADE,
        },
        server::conn::http1::Builder as Http1Builder,
        service::Service as HyperService,
        upgrade::on,
    },
    hyper_util::rt::TokioIo,
    serde::{Deserialize, Serialize},
    std::{
        collections::HashMap,
        io::{Error as IoError, ErrorKind, Result as IoResult},
        mem::take,
        net::{IpAddr, SocketAddr},
        pin::Pin,
        str::FromStr,
        sync::{Arc, LazyLock, Weak, atomic::Ordering},
    },
    tokio::{
        net::{TcpListener, ToSocketAddrs, lookup_host},
        runtime::{Builder, Runtime},
        sync::{
            Mutex,
            mpsc::{Receiver as MpscReceiver, Sender as MpscSender, channel as mpsc_channel},
            oneshot::{Sender as OneshotSender, channel as oneshot_channel},
        },
        task::JoinHandle,
    },
    tokio_tungstenite::{
        WebSocketStream,
        tungstenite::{Message, handshake::derive_accept_key, protocol::Role},
    },
    tracing::{error, info, warn},
};

static BIND_SENDERS: LazyLock<Mutex<HashMap<String, MpscSender<Command>>>> =
    LazyLock::new(Default::default);
static CONNS: LazyLock<Mutex<HashMap<(String, SocketAddr), MpscSender<Message>>>> =
    LazyLock::new(Default::default);

/// RAII guard to ensure connection cleanup on task abort
struct ConnGuard {
    path: String,
    socket_addr: SocketAddr,
    rt: Weak<Runtime>,
}

impl Drop for ConnGuard {
    fn drop(&mut self) {
        let Some(rt) = self.rt.upgrade() else {
            warn!("Runtime already dropped.");
            return;
        };

        let path = take(&mut self.path);
        let socket_addr = self.socket_addr;
        // Spawn a new task to handle async cleanup since Drop cannot be async
        rt.spawn(async move {
            CONNS.lock().await.remove(&(path, socket_addr));
        });
    }
}

pub struct WebHandler {
    socket_addr: SocketAddr,
    command: MpscSender<Command>,
    rt: Weak<Runtime>,
}

impl HyperService<Request<Incoming>> for WebHandler {
    type Response = Response<String>;
    type Error = IoError;
    type Future = Pin<Box<dyn Future<Output = IoResult<Self::Response>> + Send>>;

    fn call(&self, mut req: Request<Incoming>) -> Self::Future {
        let headers = req.headers();
        let key = headers.get(SEC_WEBSOCKET_KEY);
        let ver = req.version();
        if ver < Version::HTTP_11
            || !headers
                .get(CONNECTION)
                .and_then(|h| h.to_str().ok())
                .map(|h| {
                    h.split(|c| c == ' ' || c == ',')
                        .any(|p| p.eq_ignore_ascii_case(Self::UPGRADE_VALUE))
                })
                .unwrap_or(false)
            || !headers
                .get(UPGRADE)
                .and_then(|h| h.to_str().ok())
                .map(|h| h.eq_ignore_ascii_case(Self::WEBSOCKET_VALUE))
                .unwrap_or(false)
            || !headers
                .get(SEC_WEBSOCKET_VERSION)
                .map(|h| h == "13")
                .unwrap_or(false)
            || key.is_none()
        {
            // 非WebSocket请求，返回400错误
            let mut res = Response::new(String::from("Bad Request: WebSocket upgrade required"));
            *res.status_mut() = StatusCode::BAD_REQUEST;
            return Box::pin(async move { Ok(res) });
        }

        let rt = self.rt.clone();
        let Some(rt2) = rt.upgrade() else {
            return Box::pin(async move {
                let mut res = Response::new(String::from("Internal Server Error"));
                *res.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                Ok(res)
            });
        };

        let derived = key.map(|k| derive_accept_key(k.as_bytes()));
        let path = req.uri().path().to_owned();
        info!("Establish bidi-communication {}", path);
        let command = self.command.clone();
        let socket_addr = self.extract_real_ip(headers);
        rt2.spawn(async move {
            let stream = match on(&mut req).await {
                Ok(upgraded) => {
                    WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, None)
                        .await
                }
                Err(e) => return error!(?e, "Upgrade error."),
            };

            let (mut outgoing, mut incoming) = stream.split();
            let (tx, mut rx) = mpsc_channel(2);
            if CONNS
                .lock()
                .await
                .insert((path.clone(), socket_addr), tx)
                .is_some()
            {
                warn!("Overwrite connection to {}", path);
            }

            // Guard ensures cleanup on task abort
            let _guard = ConnGuard {
                path: path.clone(),
                socket_addr,
                rt,
            };

            loop {
                match select(incoming.next(), Box::pin(rx.recv())).await {
                    Either::Left((Some(Ok(Message::Close(c))), _)) => {
                        dbg!(c);
                        break;
                    }
                    Either::Left((None, _)) | Either::Right((None, _)) => break,

                    Either::Left((Some(Ok(msg)), _)) => {
                        let (ret_tx, ret_rx) = oneshot_channel();
                        if let Err(e) = command
                            .send(Command::Transfer {
                                socket_addr,
                                path: path.clone(),
                                msg,
                                ret_tx,
                            })
                            .await
                        {
                            error!(?e, "Can't handle the message.")
                        }

                        match ret_rx.await {
                            Ok(Some(msg)) => {
                                if let Err(e) = outgoing.send(msg).await {
                                    error!(?e, "Can't send the message.");
                                }
                            }
                            Err(e) => error!(?e, "Can't receive the message."),
                            _ => (),
                        }
                    }

                    Either::Right((Some(msg), _)) => {
                        if let Err(e) = outgoing.send(msg).await {
                            error!(?e, "Can't send the message.");
                        }
                    }

                    Either::Left((Some(Err(e)), _)) => {
                        error!(?e, "Received error.");
                        break;
                    }
                }
            }
        });

        Box::pin(async move {
            let mut res = Response::new(String::new());
            *res.status_mut() = StatusCode::SWITCHING_PROTOCOLS;
            *res.version_mut() = ver;
            res.headers_mut()
                .append(CONNECTION, HeaderValue::from_static(Self::UPGRADE_VALUE));
            res.headers_mut()
                .append(UPGRADE, HeaderValue::from_static(Self::WEBSOCKET_VALUE));
            if let Some(derived) = derived {
                res.headers_mut().append(
                    SEC_WEBSOCKET_ACCEPT,
                    derived
                        .parse()
                        .map_err(|e| IoError::new(ErrorKind::ConnectionRefused, e))?,
                );
            }
            res.headers_mut()
                .insert("Access-Control-Allow-Origin", HeaderValue::from_static("*"));

            Ok(res)
        })
    }
}

impl WebHandler {
    const UPGRADE_VALUE: &str = "Upgrade";
    const WEBSOCKET_VALUE: &str = "websocket";

    fn new(socket_addr: SocketAddr, command: MpscSender<Command>, rt: Weak<Runtime>) -> Self {
        Self {
            socket_addr,
            command,
            rt,
        }
    }

    /// 从请求头中提取真实的客户端IP地址
    ///
    /// 优先级：X-Forwarded-For > X-Real-IP > 直连地址
    fn extract_real_ip(&self, headers: &HeaderMap) -> SocketAddr {
        // 尝试从 X-Forwarded-For 头获取IP
        if let Some(forwarded_for) = headers.get("x-forwarded-for")
            && let Ok(forwarded_str) = forwarded_for.to_str()
            && let Some(first_ip) = forwarded_str.split(',').next()
        {
            let ip_str = first_ip.trim();
            if let Ok(ip) = IpAddr::from_str(ip_str) {
                return SocketAddr::new(ip, self.socket_addr.port());
            }
        }

        // 尝试从 X-Real-IP 头获取IP
        if let Some(real_ip) = headers.get("x-real-ip")
            && let Ok(ip_str) = real_ip.to_str()
            && let Ok(ip) = IpAddr::from_str(ip_str.trim())
        {
            return SocketAddr::new(ip, self.socket_addr.port());
        }

        // 如果都没有，返回原始地址
        self.socket_addr
    }
}

#[derive(Debug)]
enum Command {
    AddRoute {
        path: String,
        stream: MpscSender<(SocketAddr, Message, OneshotSender<Option<Message>>)>,
        opt_return: OneshotSender<IoResult<()>>,
    },

    RemoveRoute {
        path: String,
        opt_return: OneshotSender<IoResult<()>>,
    },

    Transfer {
        path: String,
        socket_addr: SocketAddr,
        msg: Message,
        ret_tx: OneshotSender<Option<Message>>,
    },

    CallRemotely {
        path: String,
        socket_addr: SocketAddr,
        id: ReqId,
        msg: Message,
        ret_tx: OneshotSender<IoResult<Message>>,
    },

    CommitReturn {
        path: String,
        socket_addr: SocketAddr,
        id: ReqId,
        msg: Message,
    },
}

pub type ServiceAccessor = Accessor<Conn>;

impl AsRef<ServiceAccessor> for ServiceAccessor {
    fn as_ref(&self) -> &Self {
        self
    }
}

pub struct Conn {
    path: String,
    socket_addr: SocketAddr,
}

impl Conn {
    pub fn get_addr(&self) -> SocketAddr {
        self.socket_addr
    }

    pub fn get_path(&self) -> &str {
        &self.path
    }

    pub async fn get_other_conns(&self) -> impl Iterator<Item = ServiceAccessor> {
        CONNS
            .lock()
            .await
            .keys()
            .filter(|(path, addr)| path == &self.path && addr != &self.socket_addr)
            .map(|(_, addr)| Accessor::from(Conn::from((self.path.clone(), *addr))))
            .collect::<Vec<_>>()
            .into_iter()
    }
}

impl From<(String, SocketAddr)> for Conn {
    fn from((path, socket_addr): (String, SocketAddr)) -> Self {
        Self { path, socket_addr }
    }
}

pub struct EdgyService {
    bind_addr: SocketAddr,
    rt: Arc<Runtime>,
    command: MpscSender<Command>,
    worker_task: JoinHandle<IoResult<()>>,
}

impl Router<Conn> for EdgyService {
    async fn add_route<F, P, Args, Ret>(&self, path: P, handler: F) -> IoResult<()>
    where
        F: AsyncFun<Args, Ret, Conn>,
        Args: for<'a> Deserialize<'a> + Serialize,
        Ret: for<'a> Deserialize<'a> + Serialize + Send,
        P: AsRef<str>,
    {
        let (stream_tx, mut stream_rx) = mpsc_channel(2);
        let (ret_tx, ret_rx) = oneshot_channel();
        let command = self.command.clone();
        command
            .send(Command::AddRoute {
                path: path.as_ref().into(),
                stream: stream_tx,
                opt_return: ret_tx,
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
            lock.insert(path.as_ref().into(), self.command.clone());
        }

        let path = path.as_ref().to_owned();
        self.rt.spawn(async move {
            while let Some((socket_addr, msg, ret_tx)) = stream_rx.recv().await {
                let packet = match Packet::<Args, Ret>::from_message(&msg) {
                    Ok(args) => args,
                    Err(e) => {
                        error!(?e, "Unable to decode packet.");
                        continue;
                    }
                };

                match packet {
                    Packet::Call(id, args) => {
                        let ret = match Packet::<Args, Ret>::make_ret_message(
                            id,
                            handler.call((path.clone(), socket_addr).into(), args).await,
                        ) {
                            Ok(r) => r,
                            Err(e) => {
                                error!(?e, "Unable to encode message.");
                                continue;
                            }
                        };
                        if let Err(e) = ret_tx.send(Some(ret.into())) {
                            error!(?e, "Unable to send data.");
                        }
                    }

                    Packet::Ret(id, _ret) => {
                        if let Err(e) = command
                            .send(Command::CommitReturn {
                                path: path.clone(),
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
        Ok(())
    }

    async fn remove_route<P>(&self, path: P) -> IoResult<()>
    where
        P: AsRef<str>,
    {
        BIND_SENDERS.lock().await.remove(path.as_ref());
        let (ret_tx, ret_rx) = oneshot_channel();
        self.command
            .send(Command::RemoveRoute {
                path: path.as_ref().into(),
                opt_return: ret_tx,
            })
            .await
            .map_err(IoError::other)?;
        ret_rx.await.map_err(IoError::other)??;

        Ok(())
    }
}

impl EdgyService {
    pub async fn new<Addr>(bind_addr: Addr, num_workers: usize) -> IoResult<Self>
    where
        Addr: ToSocketAddrs,
    {
        let rt = Builder::new_multi_thread()
            .worker_threads(num_workers)
            .enable_all()
            .build()?;

        let (command_tx, command_rx) = mpsc_channel(2);
        let worker_task = rt.spawn(Self::worker(command_rx));

        Ok(Self {
            command: command_tx,
            bind_addr: Self::get_bind_addr(bind_addr).await?,
            rt: rt.into(),
            worker_task,
        })
    }

    pub async fn run(self) -> IoResult<Self> {
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

        Ok(self)
    }

    pub fn abort(self) {
        self.worker_task.abort();
    }

    async fn worker(mut command: MpscReceiver<Command>) -> IoResult<()> {
        let mut handlers = HashMap::new();
        let mut pending_requests = HashMap::new();

        while let Some(item) = command.recv().await {
            match item {
                Command::AddRoute {
                    path,
                    stream,
                    opt_return,
                } => {
                    opt_return
                        .send(if handlers.contains_key(&path) {
                            Err(IoError::other(format!("Can't add route: {}", path)))
                        } else {
                            handlers.insert(path, stream);
                            Ok(())
                        })
                        .map_or_else(|e| e.map_err(IoError::other), Ok)?;
                }

                Command::RemoveRoute { path, opt_return } => {
                    opt_return
                        .send(handlers.remove(&path).map_or(
                            Err(IoError::other(format!("Can't remove route: {}", path))),
                            |_| Ok(()),
                        ))
                        .map_or_else(|e| e, Ok)?;
                }

                Command::Transfer {
                    path,
                    socket_addr,
                    msg,
                    ret_tx,
                } => {
                    if let Some(handler) = handlers.get(&path) {
                        handler
                            .send((socket_addr, msg, ret_tx))
                            .await
                            .map_or_else(|e| Err(IoError::other(e)), Ok)?;
                    }
                }

                Command::CallRemotely {
                    path,
                    socket_addr,
                    id,
                    msg,
                    ret_tx,
                } => {
                    if let Some(conn) = CONNS.lock().await.get(&(path.clone(), socket_addr)) {
                        if let Err(e) = conn.send(msg).await {
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
            }
        }

        Ok(())
    }

    fn get_bind_addr<Addr>(bind_addr: Addr) -> impl Future<Output = IoResult<SocketAddr>>
    where
        Addr: ToSocketAddrs,
    {
        async {
            lookup_host(bind_addr).await?.next().ok_or(IoError::new(
                ErrorKind::NotFound,
                "Unable to obtain host address.",
            ))
        }
    }
}

pub trait ClientCaller<Args, Ret> {
    fn call_remotely<T>(self, target: T) -> impl Future<Output = IoResult<Ret>>
    where
        T: AsRef<ServiceAccessor>;
}

impl<Args, Ret> ClientCaller<Args, Ret> for Args
where
    Args: for<'a> Deserialize<'a> + Serialize,
    Ret: for<'a> Deserialize<'a> + Serialize,
{
    async fn call_remotely<T>(self, target: T) -> IoResult<Ret>
    where
        T: AsRef<ServiceAccessor>,
    {
        static AUTO_ID: AtomicReqId = AtomicReqId::new(0);

        let path = target.as_ref().get_path();
        let socket_addr = target.as_ref().get_addr();
        let Some(command) = BIND_SENDERS.lock().await.get(path).map(|i| i.clone()) else {
            return Err(IoError::other(format!(
                "The connection with the `{}` address cannot be sent to the `{}` path to call the remote function, because the function is not bound to the server.",
                socket_addr, path,
            )));
        };

        let req_id = AUTO_ID
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
                Some(v.wrapping_add(1))
            })
            .unwrap_or_default();
        let (tx, rx) = oneshot_channel();
        command
            .send(Command::CallRemotely {
                path: path.to_owned(),
                socket_addr,
                id: req_id,
                msg: Packet::<Args, Ret>::make_call_message(req_id, self)?,
                ret_tx: tx,
            })
            .await
            .map_err(IoError::other)?;

        match Packet::<Args, Ret>::from_message(&rx.await.map_err(IoError::other)??)? {
            Packet::Ret(id, _) if id != req_id => Err(IoError::other(format!(
                "The received message ID does not match the request ID: {} != {}",
                id, req_id
            ))),
            Packet::Ret(_id, ret) => Ok(ret),
            _ => Err(IoError::other("Unable to decode packet.")),
        }
    }
}
