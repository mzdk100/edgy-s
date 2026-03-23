#[cfg(feature = "client")]
use super::HttpClientAsyncFn;
#[cfg(feature = "server")]
use super::HttpServerAsyncFn;
use {
    super::WsAsyncFn,
    serde::{Deserialize, Serialize},
    std::io::{Error as IoError, Result as IoResult},
    tokio::sync::mpsc::WeakSender,
};

/// Trait for types that can be unbound from a router.
///
/// Implemented by binding types to allow removal of routes.
pub trait Binding {
    fn unbind(self) -> impl Future<Output = IoResult<()>>;
}

/// Base binding type that holds path and command channel information.
///
/// This is the common base for all binding types, providing
/// path access and command sending capabilities.
pub struct BaseBinding<Cmd> {
    path: String,
    command: WeakSender<Cmd>,
}

impl<Cmd> BaseBinding<Cmd> {
    /// Creates a new binding with the given path and command sender.
    pub fn new<P>(path: P, command: WeakSender<Cmd>) -> Self
    where
        P: AsRef<str>,
    {
        Self {
            path: path.as_ref().to_owned(),
            command,
        }
    }

    /// Returns the path this binding is registered to.
    pub fn get_path(&self) -> &str {
        &self.path
    }

    /// Sends a command through the binding's command channel.
    ///
    /// # Errors
    /// Returns an error if the command channel is closed.
    pub async fn send_command(&self, cmd: Cmd) -> IoResult<()>
    where
        Cmd: Send + Sync + 'static,
    {
        let Some(command) = self.command.upgrade() else {
            return Err(IoError::other("Command channel is closed."));
        };

        command.send(cmd).await.map_err(IoError::other)
    }
}

/// Trait for routers that handle WebSocket routes.
///
/// Implemented by types that can register and manage WebSocket routes.
pub trait WsRouter<Acc> {
    type Binding;

    fn add_route<F, P, Args, Ret>(
        &self,
        path: P,
        handler: F,
    ) -> impl Future<Output = IoResult<Self::Binding>>
    where
        F: WsAsyncFn<Args, Ret, Acc>,
        Args: for<'a> Deserialize<'a> + Serialize + 'static,
        Ret: for<'a> Deserialize<'a> + Serialize + Send + 'static,
        P: AsRef<str>;

    fn remove_route(binding: Self::Binding) -> impl Future<Output = IoResult<()>>;
}

/// Trait for HTTP request routing.
#[cfg(feature = "client")]
pub trait HttpClientRouter<Acc> {
    type Binding;

    fn add_route<F, P>(&self, path: P, handler: F) -> impl Future<Output = IoResult<Self::Binding>>
    where
        F: HttpClientAsyncFn<Acc>,
        P: AsRef<str>;

    fn remove_route(binding: Self::Binding) -> impl Future<Output = IoResult<()>>;
}

/// Trait for routers that handle HTTP routes (server-side).
#[cfg(feature = "server")]
pub trait HttpServerRouter<Acc> {
    type Binding;

    fn add_route<F, P, Body, Ret>(
        &self,
        path: P,
        handler: F,
    ) -> impl Future<Output = IoResult<Self::Binding>>
    where
        F: HttpServerAsyncFn<Body, Ret, Acc>,
        Body: From<crate::types::stream::StreamingBody>,
        P: AsRef<str>,
        Ret: crate::types::stream::IntoStreamingBody;

    fn remove_route(binding: Self::Binding) -> impl Future<Output = IoResult<()>>;
}
