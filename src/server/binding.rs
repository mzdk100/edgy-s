use {
    super::{
        super::types::{BaseBinding, Binding},
        Accessor, Command, EdgyService, HttpConn, HttpServerRouter, IoError, IoResult,
        MpscReceiver, OneshotSender, WsConn, WsRouter,
    },
    std::{
        ops::Deref,
        pin::Pin,
        sync::{Arc, Weak},
    },
    tokio::{
        runtime::Runtime,
        select,
        sync::{Mutex, mpsc::WeakSender},
    },
    tracing::error,
};

type Handler<T> = Box<dyn Fn(Accessor<T>) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>;

/// WebSocket binding for server-side connections.
///
/// Manages incoming WebSocket connections to a server path and provides
/// hooks for handling connection open and close events.
pub struct WsBinding<O, C> {
    base: BaseBinding<Command>,
    open_close: Arc<Mutex<(Handler<O>, Handler<C>)>>,
}

impl<O, C> WsBinding<O, C> {
    pub(super) fn new<P>(
        path: P,
        command: WeakSender<Command>,
        rt: Weak<Runtime>,
        open_rx: MpscReceiver<(Accessor<O>, OneshotSender<()>)>,
        close_rx: MpscReceiver<Accessor<C>>,
    ) -> IoResult<Self>
    where
        O: Send + 'static,
        C: Send + 'static,
        P: AsRef<str>,
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
                            Some((accessor, ret)) => {
                                let lock = open_close_clone.lock().await;
                                lock.0(accessor).await;
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
                            Some(accessor) => {
                                let lock = open_close_clone.lock().await;
                                lock.1(accessor).await;
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

impl Binding for WsBinding<HttpConn, WsConn> {
    async fn unbind(self) -> IoResult<()> {
        <EdgyService as WsRouter<WsConn>>::remove_route(self).await
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
pub struct HttpBinding {
    base: BaseBinding<Command>,
}

impl HttpBinding {
    pub(super) fn new<P>(path: P, command: WeakSender<Command>) -> Self
    where
        P: AsRef<str>,
    {
        Self {
            base: BaseBinding::new(path, command),
        }
    }
}

impl Binding for HttpBinding {
    async fn unbind(self) -> IoResult<()> {
        <EdgyService as HttpServerRouter<HttpConn>>::remove_route(self).await
    }
}

impl Deref for HttpBinding {
    type Target = BaseBinding<Command>;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}
