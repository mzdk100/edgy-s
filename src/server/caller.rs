use {
    super::{
        super::types::{AtomicReqId, Packet},
        Command, IoError, IoResult, WsAccessor,
    },
    serde::{Deserialize, Serialize},
    std::{
        collections::HashMap,
        sync::{LazyLock, atomic::Ordering},
    },
    tokio::sync::{Mutex, mpsc::WeakSender, oneshot::channel as oneshot_channel},
};

pub(super) static BIND_SENDERS: LazyLock<Mutex<HashMap<String, WeakSender<Command>>>> =
    LazyLock::new(Default::default);

/// Trait for calling remote functions via WebSocket from the server side.
///
/// This trait allows the server to call a function on a connected client
/// through a WebSocket connection. The arguments are serialized and sent
/// to the client, and the return value is received back.
///
/// # Example
/// ```ignore
/// let result: i32 = (1, 2).call_remotely(&ws_accessor).await?;
/// ```
pub trait WsCaller<Args, Ret> {
    /// Calls a remote function on a specific client connection.
    ///
    /// The `target` specifies which client connection to send the request to.
    fn call_remotely<T>(self, target: T) -> impl Future<Output = IoResult<Ret>>
    where
        T: AsRef<WsAccessor>;
}

impl<Args, Ret> WsCaller<Args, Ret> for Args
where
    Args: for<'a> Deserialize<'a> + Serialize,
    Ret: for<'a> Deserialize<'a> + Serialize,
{
    async fn call_remotely<T>(self, target: T) -> IoResult<Ret>
    where
        T: AsRef<WsAccessor>,
    {
        static AUTO_ID: AtomicReqId = AtomicReqId::new(0);

        let path = target.as_ref().path();
        let socket_addr = target.as_ref().get_addr();
        let Some(command) = BIND_SENDERS.lock().await.get(path).map(|i| i.clone()) else {
            return Err(IoError::other(format!(
                "The connection with the `{}` address cannot be sent to the `{}` path to call the remote function, because the function is not bound to the server.",
                socket_addr, path,
            )));
        };
        let Some(command) = command.upgrade() else {
            return Err(IoError::other("The command channel has been dropped."));
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
                msg: crate::types::Packet::<Args, Ret>::make_call_message(req_id, self)?,
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
