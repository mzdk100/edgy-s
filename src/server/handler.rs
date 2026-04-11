use {
    super::{
        Command, IntoStreamingBody, StreamingBody, TokioIo,
        conn::{ConnInfo, WS_CONNS, WsConnGuard},
    },
    futures_util::{
        SinkExt, StreamExt,
        future::{Either, select},
    },
    hyper::{
        HeaderMap, Request, Response, StatusCode, Uri, Version,
        body::Incoming,
        header::{
            CONNECTION, HeaderValue, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY,
            SEC_WEBSOCKET_VERSION, UPGRADE,
        },
        service::Service as HyperService,
        upgrade::on,
    },
    std::{
        io::{Error as IoError, ErrorKind, Result as IoResult},
        net::{IpAddr, SocketAddr},
        pin::Pin,
        str::FromStr,
        sync::Weak,
    },
    tokio::{
        runtime::Runtime,
        sync::{
            mpsc::{Sender as MpscSender, channel as mpsc_channel},
            oneshot::{Sender as OneshotSender, channel as oneshot_channel},
        },
    },
    tokio_tungstenite::{
        WebSocketStream,
        tungstenite::{Message, handshake::derive_accept_key, protocol::Role},
    },
    tokio_util::sync::CancellationToken,
    tracing::{error, info, warn},
};

pub(super) struct WebHandler {
    socket_addr: SocketAddr,
    command: MpscSender<Command>,
    rt: Weak<Runtime>,
    cancel_token: CancellationToken,
}

impl HyperService<Request<Incoming>> for WebHandler {
    type Response = Response<StreamingBody>;
    type Error = IoError;
    type Future = Pin<Box<dyn Future<Output = IoResult<Self::Response>> + Send>>;

    fn call(&self, req: Request<Incoming>) -> Self::Future {
        let headers = req.headers();
        let key = headers.get(SEC_WEBSOCKET_KEY);
        let ver = req.version();
        let socket_addr = self.extract_real_ip(headers);

        if ver < Version::HTTP_11
            || !headers
                .get(CONNECTION)
                .and_then(|h| h.to_str().ok())
                .map(|h| {
                    h.split([' ', ','])
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
            let command = self.command.clone();
            let uri = req.uri().to_owned();
            let headers = req.headers().clone();
            let body = req.into_body().into_streaming_body();
            let cancel_token = self.cancel_token.child_token();

            return Box::pin(async move {
                Ok(
                    Self::http_dispatch(command, uri, socket_addr, headers, body, cancel_token)
                        .await
                        .unwrap_or_else(|e| {
                            let mut res = Response::new(
                                format!("Internal Server Error: {}", e).into_streaming_body(),
                            );
                            *res.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                            res
                        }),
                )
            });
        }

        let derived = key.map(|k| derive_accept_key(k.as_bytes()));
        let rt = self.rt.clone();
        let (tx, rx) = oneshot_channel();
        if let Some(rt2) = rt.upgrade() {
            rt2.spawn(Self::ws_dispatch(
                rt,
                self.command.clone(),
                socket_addr,
                req,
                tx,
            ));
        };

        Box::pin(async move {
            let mut res = Response::new(Default::default());
            let (headers, status) = rx.await.map_err(IoError::other)?;
            *res.headers_mut() = headers;
            *res.status_mut() = status;
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

impl Drop for WebHandler {
    fn drop(&mut self) {
        // Cancel all pending HTTP handlers when connection is closed
        self.cancel_token.cancel();
    }
}

impl WebHandler {
    const UPGRADE_VALUE: &str = "Upgrade";
    const WEBSOCKET_VALUE: &str = "websocket";

    pub(super) fn new(
        socket_addr: SocketAddr,
        command: MpscSender<Command>,
        rt: Weak<Runtime>,
    ) -> Self {
        Self {
            socket_addr,
            command,
            rt,
            cancel_token: CancellationToken::new(),
        }
    }

    async fn http_dispatch(
        command: MpscSender<Command>,
        uri: Uri,
        socket_addr: SocketAddr,
        headers: HeaderMap,
        body: StreamingBody,
        cancel_token: CancellationToken,
    ) -> IoResult<Response<StreamingBody>> {
        let (tx, rx) = oneshot_channel();
        command
            .send(Command::Request {
                uri,
                socket_addr,
                headers,
                body,
                ret_tx: tx,
                cancel_token,
            })
            .await
            .map_err(IoError::other)?;
        let (response_headers, response_status, response_body) =
            rx.await.map_err(IoError::other)?;
        let mut res = Response::new(response_body);
        *res.headers_mut() = response_headers;
        *res.status_mut() = response_status;

        Ok(res)
    }

    async fn ws_dispatch(
        rt: Weak<Runtime>,
        command: MpscSender<Command>,
        socket_addr: SocketAddr,
        req: Request<Incoming>,
        open_tx: OneshotSender<(HeaderMap, StatusCode)>,
    ) {
        let uri = req.uri().to_owned();
        info!("Establish bidi-communication {}", uri);

        let headers = req.headers().clone();
        if let Err(e) = command
            .send(Command::WsOpen {
                uri: uri.to_owned(),
                socket_addr,
                headers: headers.clone(),
                res_tx: open_tx,
            })
            .await
        {
            return error!(?e, "Failed to send websocket open command.");
        }

        let stream = match on(req).await {
            Ok(upgraded) => {
                WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, None).await
            }
            Err(e) => return error!(?e, "Upgrade error."),
        };
        let (mut outgoing, mut incoming) = stream.split();
        let (tx, mut rx) = mpsc_channel(2);
        if WS_CONNS
            .lock()
            .await
            .insert(
                (uri.path().to_owned(), socket_addr),
                ConnInfo {
                    uri: uri.clone(),
                    headers: headers.clone(),
                    sender: tx,
                },
            )
            .is_some()
        {
            warn!("Overwrite connection to {}", uri);
        }

        // Guard ensures cleanup on task abort
        let _guard = WsConnGuard {
            uri: uri.clone(),
            socket_addr,
            headers: headers.clone(),
            rt,
            command: command.downgrade(),
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
                            uri: uri.clone(),
                            headers: headers.clone(),
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
