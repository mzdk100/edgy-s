use {
    super::{
        super::{
            types::{IntoStreamingBody, Packet, ReqId, State, StreamingBody, WsAsyncFn},
            utils::append_query_params,
        },
        ErrorKind, HttpAccessor, IoError, IoResult, MpscReceiver, OneshotSender, RequestAccessor,
        RequestConn, ResponseConn,
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
    hyper_tls::HttpsConnector,
    hyper_util::{client::legacy::Client, rt::TokioExecutor},
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
pub(super) async fn ws_dispatch<F, Args, Ret, S>(
    uri: Uri,
    mut headers: HeaderMap,
    call_rx: &mut MpscReceiver<WsCall>,
    handler: F,
    response_tx: OneshotSender<(StatusCode, HeaderMap)>,
    state: State<S>,
) -> IoResult<Option<CloseFrame>>
where
    F: WsAsyncFn<Args, Ret, ResponseConn<S>, S>,
    Args: for<'a> Deserialize<'a> + Serialize,
    Ret: for<'a> Deserialize<'a> + Serialize,
    S: std::fmt::Debug,
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
    response_tx
        .send((status, response_headers.clone()))
        .map_err(|_| IoError::other("Failed to send response"))?;

    let (mut write, mut read) = stream.split();
    let mut pending_requests = HashMap::<_, OneshotSender<_>>::new();

    loop {
        match select(read.next(), Box::pin(call_rx.recv())).await {
            Either::Left((Some(Ok(Message::Close(c))), _)) => break Ok(c),
            Either::Left((None, _)) | Either::Right((None, _)) => break Ok(None),

            Either::Left((Some(Ok(msg)), _)) => match Packet::<Args, Ret>::from_message(&msg) {
                Ok(Packet::Call(id, args)) => {
                    let accessor = ResponseConn::from((
                        uri.clone(),
                        status,
                        response_headers.clone(),
                        state.clone(),
                    ))
                    .into();
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
                    if let Err(e) = write.send(ret).await {
                        error!(?e, "Can't send the message.");
                    }
                }

                Ok(Packet::Ret(id, _ret)) => {
                    if let Some(tx) = pending_requests.remove(&id)
                        && let Err(e) = tx.send(msg)
                    {
                        error!(?e, "Can't send the return message.");
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
#[allow(clippy::too_many_arguments)]
pub(super) async fn ws_dispatch_with_auto_reconnection<F, Args, Ret, S>(
    uri: Uri,
    request_tx: OneshotSender<(RequestAccessor<S>, OneshotSender<()>)>,
    mut call_rx: MpscReceiver<WsCall>,
    handler: F,
    open_tx: OneshotSender<HttpAccessor<S>>,
    close_tx: OneshotSender<HttpAccessor<S>>,
    max_retries: usize,
    retry_interval: Duration,
    state: State<S>,
) where
    F: WsAsyncFn<Args, Ret, ResponseConn<S>, S>,
    Args: for<'a> Deserialize<'a> + Serialize,
    Ret: for<'a> Deserialize<'a> + Serialize,
    S: Send + std::fmt::Debug + 'static,
{
    let (header_tx, header_rx) = watch_channel::<HeaderMap>(Default::default());
    let (query_tx, query_rx) = watch_channel::<HashMap<String, String>>(Default::default());
    let accessor = RequestConn::<S>::from((uri.clone(), header_tx, query_tx, state.clone())).into();
    let (ret_tx, ret_rx) = oneshot_channel();
    let headers = if let Err(e) = request_tx.send((accessor, ret_tx)) {
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
    let mut guard = WsConnGuard::new();
    let mut close_tx = Some(close_tx);
    let mut open_tx = Some(open_tx);

    for attempt in 0..max_retries {
        let (response_tx, response_rx) = oneshot_channel();
        let mut response_fut = Box::pin(ws_dispatch(
            uri.clone(),
            headers.clone(),
            &mut call_rx,
            handler,
            response_tx,
            state.clone(),
        ));
        let (status_code, response_headers) = match select(response_rx, response_fut).await {
            Either::Left((Ok(r), fut)) => {
                response_fut = fut;
                r
            }
            Either::Left((Err(e), fut)) => {
                warn!(?e, "Connection attempt failed, retrying...");
                sleep(retry_interval).await;
                response_fut = fut;
                continue;
            }
            Either::Right((Err(e), _)) => {
                error!(?e, "Error in WebSocket connection.");
                sleep(retry_interval).await;
                continue;
            }
            _ => {
                warn!("Connection finished before receiving response.");
                break;
            }
        };

        // Send open signal immediately after successful connection
        if let Some(tx) = open_tx.take() {
            if let Err(e) = tx.send(
                ResponseConn::from((
                    uri.clone(),
                    status_code,
                    response_headers.clone(),
                    state.clone(),
                ))
                .into(),
            ) {
                warn!(?e, "Can't send the open signal.");
            } else {
                info!("WebSocket connection opened: {}", uri);
            }
        }
        if let Some(tx) = close_tx.take() {
            guard.close_tx = Some((
                tx,
                ResponseConn::from((
                    uri.clone(),
                    status_code,
                    response_headers.clone(),
                    state.clone(),
                ))
                .into(),
            ));
        }

        // Pass mutable reference so open_tx is only consumed on successful connection
        match response_fut.await {
            Ok(close_frame) => {
                // Normal close, exit the reconnection loop
                if let Some(frame) = close_frame {
                    info!(?frame, "Connection closed normally.");
                }
                return;
            }
            Err(e) => {
                warn!(?e, attempt, "Connection lost, reconnecting...");
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
    let connector = HttpsConnector::new();
    let client: Client<_, StreamingBody> = Client::builder(TokioExecutor::new()).build(connector);

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
