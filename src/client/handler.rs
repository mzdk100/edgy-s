use {
    super::{
        super::types::{IntoStreamingBody, Packet, ReqId, StreamingBody, WsAsyncFn},
        super::utils::append_query_params,
        ErrorKind, IoError, IoResult, MpscReceiver, OneshotSender, RequestAccessor, RequestConn,
        ResponseConn, WsAccessor,
        conn::WsConnGuard,
        oneshot_channel,
    },
    futures_util::{
        SinkExt, StreamExt,
        future::{Either, select},
    },
    hyper::{
        Method, Request,
        header::HeaderMap,
        http::{StatusCode, Uri},
    },
    hyper_util::{
        client::legacy::{Client, connect::HttpConnector},
        rt::TokioExecutor,
    },
    serde::{Deserialize, Serialize},
    std::collections::HashMap,
    tokio::{
        sync::watch::channel as watch_channel,
        time::{Duration, sleep},
    },
    tokio_tungstenite::{
        connect_async,
        tungstenite::{Message, client::IntoClientRequest, protocol::CloseFrame},
    },
    tracing::{error, info, warn},
};

pub(super) type WsCall = (ReqId, Message, OneshotSender<Message>);

/// Single WebSocket connection dispatch without reconnection logic.
/// Sends open signal immediately after successful connection if provided.
pub(super) async fn ws_dispatch<F, Args, Ret>(
    uri: Uri,
    mut headers: HeaderMap,
    call_rx: &mut MpscReceiver<WsCall>,
    handler: F,
    open_tx: &mut Option<OneshotSender<WsAccessor>>,
) -> IoResult<Option<CloseFrame>>
where
    F: WsAsyncFn<Args, Ret, ResponseConn>,
    Args: for<'a> Deserialize<'a> + Serialize,
    Ret: for<'a> Deserialize<'a> + Serialize,
{
    let mut request = uri
        .clone()
        .into_client_request()
        .map_err(|e| IoError::new(ErrorKind::HostUnreachable, e))?;
    for (name, value) in request.headers() {
        headers.insert(name, value.to_owned());
    }
    *request.headers_mut() = headers;
    let (stream, response) = connect_async(request).await.map_err(IoError::other)?;

    // Extract status code and headers from the WebSocket handshake response
    let status = response.status();
    let response_headers = response.headers().clone();

    // Send open signal immediately after successful connection
    if let Some(tx) = open_tx.take() {
        let accessor: ResponseConn = (uri.clone(), status, response_headers).into();
        if let Err(e) = tx.send(accessor.into()) {
            error!(?e, "Failed to send open signal.");
        } else {
            info!("WebSocket connection opened: {}", uri);
        }
    }

    let (mut write, mut read) = stream.split();
    let mut pending_requests = HashMap::<_, OneshotSender<_>>::new();

    loop {
        match select(read.next(), Box::pin(call_rx.recv())).await {
            Either::Left((Some(Ok(Message::Close(c))), _)) => break Ok(c),
            Either::Left((None, _)) | Either::Right((None, _)) => break Ok(None),

            Either::Left((Some(Ok(msg)), _)) => match Packet::<Args, Ret>::from_message(&msg) {
                Ok(Packet::Call(id, args)) => {
                    let ret = match Packet::<Args, Ret>::make_ret_message(
                        id,
                        handler.call(uri.clone().into(), args).await,
                    ) {
                        Ok(r) => r,
                        Err(e) => {
                            error!(?e, "Unable to encode message.");
                            continue;
                        }
                    };
                    if let Err(e) = write.send(ret).await {
                        error!(?e, "Can't send the message.");
                    }
                }

                Ok(Packet::Ret(id, _ret)) => {
                    if let Some(tx) = pending_requests.remove(&id) {
                        if let Err(e) = tx.send(msg) {
                            error!(?e, "Can't send the return message.");
                        }
                    }
                }
                Err(e) => error!(?e, "Unable to decode packet."),
            },

            Either::Right((Some((id, msg, opt_return)), _)) => {
                if let Err(e) = write.send(msg).await {
                    error!(?e, "Can't send the message.");
                }
                pending_requests.insert(id, opt_return);
            }

            Either::Left((Some(Err(e)), _)) => {
                break Err(IoError::new(ErrorKind::NetworkUnreachable, e));
            }
        };
    }
}

/// WebSocket dispatch with automatic reconnection.
/// Manages open/close signals: sends open on first successful connection, close on final exit.
pub(super) async fn ws_dispatch_with_auto_reconnection<F, Args, Ret>(
    uri: Uri,
    request_tx: OneshotSender<(RequestAccessor, OneshotSender<()>)>,
    mut call_rx: MpscReceiver<WsCall>,
    handler: F,
    open_tx: OneshotSender<WsAccessor>,
    close_tx: OneshotSender<WsAccessor>,
    max_retries: usize,
    retry_interval: Duration,
) where
    F: WsAsyncFn<Args, Ret, ResponseConn>,
    Args: for<'a> Deserialize<'a> + Serialize,
    Ret: for<'a> Deserialize<'a> + Serialize,
{
    let (header_tx, header_rx) = watch_channel::<HeaderMap>(Default::default());
    let (query_tx, query_rx) = watch_channel::<HashMap<String, String>>(Default::default());
    let accessor: RequestConn = (uri.clone(), header_tx, query_tx).into();
    let (ret_tx, ret_rx) = oneshot_channel();
    let headers = if let Err(e) = request_tx.send((accessor.into(), ret_tx)) {
        error!(?e, "Can't send the request.");
        HeaderMap::new()
    } else if let Err(e) = ret_rx.await {
        error!(?e, "Can't receive the ret signal of the request.");
        HeaderMap::new()
    } else {
        header_rx.borrow().clone()
    };
    // Apply query params to URI
    let query_params = query_rx.borrow().clone();
    let uri = append_query_params(&uri, &query_params);

    // Guard ensures close signal is sent on final exit
    let mut guard = WsConnGuard::new(open_tx, close_tx, uri.clone());

    for attempt in 0..max_retries {
        // Pass mutable reference so open_tx is only consumed on successful connection
        match ws_dispatch(
            uri.clone(),
            headers.clone(),
            &mut call_rx,
            handler,
            &mut guard.open_tx,
        )
        .await
        {
            Ok(close_frame) => {
                // Normal close, exit the reconnection loop
                if let Some(frame) = close_frame {
                    info!(?frame, "Connection closed normally.");
                }
                return;
            }
            Err(e) => {
                if guard.open_tx.is_some() {
                    // open_tx not consumed means connection never succeeded
                    warn!(?e, "Connection attempt failed, retrying...");
                } else {
                    // open_tx consumed means connection was once successful but now disconnected
                    warn!(?e, attempt, "Connection lost, reconnecting...");
                }
                // Wait before retrying (except on last attempt)
                if attempt < max_retries - 1 {
                    sleep(retry_interval).await;
                }
            }
        }
    }

    error!(
        "The maximum number {} of reconnections has been reached.",
        max_retries
    );
}

pub type HttpCall = (
    Uri,
    Method,
    HeaderMap,
    StreamingBody,
    OneshotSender<IoResult<(StreamingBody, StatusCode, HeaderMap)>>,
);

pub(super) async fn http_dispatch(mut request_rx: MpscReceiver<HttpCall>) {
    let client: Client<HttpConnector, StreamingBody> =
        Client::builder(TokioExecutor::new()).build_http();

    while let Some((uri, method, headers, body, response_tx)) = request_rx.recv().await {
        let mut builder = Request::builder().method(method).uri(uri);

        // Apply custom headers
        for (name, value) in &headers {
            builder = builder.header(name, value);
        }

        let request = match builder.body(body) {
            Ok(request) => request,
            Err(err) => {
                if let Err(e) = response_tx.send(Err(IoError::other(err))) {
                    error!(?e, "Can't send the response.");
                }
                continue;
            }
        };

        let response = match client.request(request).await {
            Ok(res) => res,
            Err(e) => {
                if let Err(e) = response_tx.send(Err(IoError::other(e))) {
                    error!(?e, "Can't send the response.");
                }
                continue;
            }
        };

        let (parts, incoming) = response.into_parts();
        let status = parts.status;
        let response_headers = parts.headers;

        if let Err(e) = response_tx.send(Ok((
            incoming.into_streaming_body(),
            status,
            response_headers,
        ))) {
            error!(?e, "Can't send the response.");
        }
    }
}
