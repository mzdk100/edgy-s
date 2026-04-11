use {
    super::{
        super::types::{BaseBinding, Binding, HttpClientRouter},
        Accessor, Command, EdgyClient, IoError, IoResult, OneshotSender, RequestConn, ResponseConn,
        WsRouter,
    },
    std::{
        fmt::Debug,
        ops::Deref,
        pin::Pin,
        sync::{Arc, Weak},
    },
    tokio::{
        runtime::Runtime,
        select,
        sync::{Mutex, mpsc::WeakSender, oneshot::Receiver as OneshotReceiver},
        task::JoinHandle,
    },
    tracing::error,
};

type Handler<T> = Box<dyn Fn(Accessor<T>) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>;
type RequestOpenClose<Req, Res> = Arc<Mutex<(Handler<Req>, Handler<Res>, Handler<Res>)>>;

/// WebSocket binding for client-side connections.
///
/// Manages a WebSocket connection to a server path and provides
/// hooks for handling incoming requests, connection open, and close events.
pub struct WsBinding<Req, Res> {
    base: BaseBinding<Command>,
    request_open_close: RequestOpenClose<Req, Res>,
}

impl<Req, Res> WsBinding<Req, Res> {
    pub(super) fn new<P>(
        path: P,
        command: WeakSender<Command>,
        rt: Weak<Runtime>,
        request_rx: OneshotReceiver<(Accessor<Req>, OneshotSender<()>)>,
        open_rx: OneshotReceiver<Accessor<Res>>,
        close_rx: OneshotReceiver<Accessor<Res>>,
    ) -> IoResult<Self>
    where
        Req: Send + 'static,
        Res: Send + 'static,
        P: AsRef<str>,
    {
        let rt = rt
            .upgrade()
            .ok_or(IoError::other("Runtime already dropped."))?;
        let request_open_close: RequestOpenClose<Req, Res> = Arc::new(Mutex::new((
            Box::new(|_| Box::pin(async {}) as _),
            Box::new(|_| Box::pin(async {}) as _),
            Box::new(|_| Box::pin(async {}) as _),
        )));
        let request_open_close_clone = request_open_close.clone();

        // Single task to handle all signals in parallel
        rt.spawn(async move {
            let mut request_rx = request_rx;
            let mut open_rx = open_rx;
            let mut close_rx = close_rx;
            let mut request_done = false;
            let mut open_done = false;
            let mut close_done = false;

            while !request_done || !open_done || !close_done {
                select! {
                    result = &mut request_rx, if !request_done => {
                        request_done = true;
                        match result {
                            Ok((accessor, ret)) => {
                                let lock = request_open_close_clone.lock().await;
                                lock.0(accessor).await;
                                drop(lock);
                                if let Err(e) = ret.send(()) {
                                    error!(?e, "Failed to send request ret signal.");
                                }
                            }
                            Err(e) => error!(?e, "Failed to receive request signal."),
                        }
                    }
                    result = &mut open_rx, if !open_done => {
                        open_done = true;
                        match result {
                            Ok(accessor) => {
                                let lock = request_open_close_clone.lock().await;
                                lock.1(accessor).await;
                            }
                            Err(e) => error!(?e, "Failed to receive open signal."),
                        }
                    }
                    result = &mut close_rx, if !close_done => {
                        close_done = true;
                        match result {
                            Ok(accessor) => {
                                let lock = request_open_close_clone.lock().await;
                                lock.2(accessor).await;
                            }
                            Err(e) => error!(?e, "Failed to receive close signal."),
                        }
                    }
                }
            }
        });

        Ok(Self {
            base: BaseBinding::new(path, command),
            request_open_close,
        })
    }

    /// Sets a handler for incoming requests from the server.
    ///
    /// This handler is called when the server sends a request to this client.
    pub async fn on_request<F, Fut>(self, request: F) -> Self
    where
        F: Fn(Accessor<Req>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let mut lock = self.request_open_close.lock().await;
        lock.0 = Box::new(move |acc| Box::pin(request(acc)));
        drop(lock);

        self
    }

    /// Sets a handler for connection open events.
    ///
    /// This handler is called when the WebSocket connection is established.
    pub async fn on_open<F, Fut>(self, open: F) -> Self
    where
        F: Fn(Accessor<Res>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let mut lock = self.request_open_close.lock().await;
        lock.1 = Box::new(move |acc| Box::pin(open(acc)));
        drop(lock);

        self
    }

    /// Sets a handler for connection close events.
    ///
    /// This handler is called when the WebSocket connection is closed.
    pub async fn on_close<F, Fut>(self, close: F) -> Self
    where
        F: Fn(Accessor<Res>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let mut lock = self.request_open_close.lock().await;
        lock.2 = Box::new(move |acc| Box::pin(close(acc)));
        drop(lock);

        self
    }
}

impl<S> Binding for WsBinding<RequestConn<S>, ResponseConn<S>>
where
    S: Debug + Send + Sync + 'static,
{
    async fn unbind(self) -> IoResult<()> {
        <EdgyClient<S> as WsRouter<ResponseConn<S>, S>>::remove_route(self).await
    }
}

impl<O, C> Deref for WsBinding<O, C> {
    type Target = BaseBinding<Command>;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

/// HTTP binding for client-side request handlers.
///
/// Manages an HTTP route registration on the client side.
pub struct HttpBinding {
    base: BaseBinding<Command>,
    task: JoinHandle<()>,
}

impl HttpBinding {
    pub(super) fn new<P>(path: P, command: WeakSender<Command>, task: JoinHandle<()>) -> Self
    where
        P: AsRef<str>,
    {
        Self {
            base: BaseBinding::new(path, command),
            task,
        }
    }
}

impl Binding for HttpBinding {
    async fn unbind(self) -> IoResult<()> {
        self.task.abort();
        <EdgyClient<()> as HttpClientRouter<RequestConn, ()>>::remove_route(self).await
    }
}

impl Deref for HttpBinding {
    type Target = BaseBinding<Command>;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}
