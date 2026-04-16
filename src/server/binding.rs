use {
    super::{
        super::types::{BaseBinding, Binding, State},
        Accessor, Command, EdgyService, HttpConn, HttpServerRouter, IoError, IoResult,
        MpscReceiver, OneshotSender, WsConn, WsRouter,
    },
    hyper::{HeaderMap, StatusCode, Uri},
    std::{
        marker::PhantomData,
        net::SocketAddr,
        ops::Deref,
        pin::Pin,
        sync::{Arc, Weak},
    },
    tokio::{
        runtime::Runtime,
        select,
        sync::{
            Mutex, mpsc::WeakSender, oneshot::channel as oneshot_channel,
            watch::Sender as WatchSender,
        },
    },
    tracing::error,
};

type Handler<T> = Box<dyn Fn(Accessor<T>) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>;
type OpenPayload = (
    Uri,
    SocketAddr,
    HeaderMap,
    WatchSender<HeaderMap>,
    WatchSender<StatusCode>,
    OneshotSender<()>,
);
type ClosePayload = (Uri, SocketAddr, HeaderMap);

/// WebSocket binding for server-side connections.
///
/// Manages incoming WebSocket connections to a server path and provides
/// hooks for handling connection open and close events.
pub struct WsBinding<O, C> {
    base: BaseBinding<Command>,
    open_close: Arc<Mutex<(Handler<O>, Handler<C>)>>,
}

impl<O, C> WsBinding<O, C> {
    pub(super) fn new<P, S>(
        path: P,
        command: WeakSender<Command>,
        rt: Weak<Runtime>,
        open_rx: MpscReceiver<OpenPayload>,
        close_rx: MpscReceiver<ClosePayload>,
        state: State<S>,
    ) -> IoResult<Self>
    where
        O: From<(
                Uri,
                SocketAddr,
                HeaderMap,
                State<S>,
                WatchSender<HeaderMap>,
                WatchSender<StatusCode>,
            )> + Send
            + 'static,
        C: From<(Uri, SocketAddr, HeaderMap, State<S>)> + Send + 'static,
        P: AsRef<str>,
        S: Send + Sync + 'static,
    {
        let rt = rt
            .upgrade()
            .ok_or(IoError::other("Runtime already dropped."))?;
        let open_close: Arc<Mutex<(Handler<O>, Handler<C>)>> = Arc::new(Mutex::new((
            Box::new(|_| Box::pin(async {}) as _),
            Box::new(|_| Box::pin(async {}) as _),
        )));
        let open_close_clone = open_close.clone();

        // Single task to handle both open and close signals in parallel
        rt.spawn(async move {
            let mut open_rx = open_rx;
            let mut close_rx = close_rx;

            loop {
                select! {
                    result = open_rx.recv() => {
                        match result {
                            Some((uri, addr, request_headers, response_headers, response_status, ret)) => {
                                let lock = open_close_clone.lock().await;
                                lock.0(O::from((uri, addr, request_headers, state.clone(), response_headers, response_status)).into()).await;
                                drop(lock);
                                if let Err(e) = ret.send(()) {
                                    error!(?e, "Failed to send open ret signal.");
                                }
                            }
                            None => break,
                        }
                    }
                    result = close_rx.recv() => {
                        match result {
                            Some((uri, addr, headers,)) => {
                                let lock = open_close_clone.lock().await;
                                lock.1(C::from((uri, addr, headers, state.clone())).into()).await;
                            }
                            None => break,
                        }
                    }
                }
            }
        });

        Ok(Self {
            base: BaseBinding::new(path, command),
            open_close,
        })
    }

    /// Sets a handler for connection open events.
    ///
    /// This handler is called when a new WebSocket connection is established.
    pub async fn on_open<F, Fut>(self, open: F) -> Self
    where
        Fut: Future<Output = ()> + Send + 'static,
        F: Fn(Accessor<O>) -> Fut + Send + 'static,
        O: Send + 'static,
    {
        let mut lock = self.open_close.lock().await;
        lock.0 = Box::new(move |acc| Box::pin(open(acc)));
        drop(lock);

        self
    }

    /// Sets a handler for connection close events.
    ///
    /// This handler is called when a WebSocket connection is closed.
    pub async fn on_close<F, Fut>(self, close: F) -> Self
    where
        Fut: Future<Output = ()> + Send + 'static,
        F: Fn(Accessor<C>) -> Fut + Send + 'static,
        C: Send + 'static,
    {
        let mut lock = self.open_close.lock().await;
        lock.1 = Box::new(move |acc| Box::pin(close(acc)));
        drop(lock);

        self
    }
}

impl<S> Binding for WsBinding<HttpConn<S>, WsConn<S>>
where
    S: Send + Sync + 'static,
{
    async fn unbind(self) -> IoResult<()> {
        <EdgyService<S> as WsRouter<WsConn<S>, S>>::remove_route(self).await
    }
}

impl<O, C> Deref for WsBinding<O, C> {
    type Target = BaseBinding<Command>;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

/// HTTP binding for server-side request handlers.
///
/// Manages an HTTP route registration on the server side.
pub struct HttpBinding<S> {
    base: BaseBinding<Command>,
    is_default: bool,
    _s: PhantomData<S>,
}

impl<S> HttpBinding<S> {
    pub(super) fn new<P>(path: P, command: WeakSender<Command>) -> Self
    where
        P: AsRef<str>,
    {
        Self {
            base: BaseBinding::new(path, command),
            is_default: false,
            _s: Default::default(),
        }
    }

    pub(super) fn new_default(command: WeakSender<Command>) -> Self {
        Self {
            base: BaseBinding::new("", command),
            is_default: true,
            _s: Default::default(),
        }
    }
}

impl<S> Binding for HttpBinding<S>
where
    S: Send + Sync + 'static,
{
    async fn unbind(self) -> IoResult<()> {
        if self.is_default {
            let (ret_tx, ret_rx) = oneshot_channel();
            self.send_command(Command::RemoveDefaultHttpRoute { opt_return: ret_tx })
                .await?;
            ret_rx.await.map_err(IoError::other)?
        } else {
            <EdgyService<S> as HttpServerRouter<HttpConn<S>, S>>::remove_route(self).await
        }
    }
}

impl<S> Deref for HttpBinding<S> {
    type Target = BaseBinding<Command>;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}
