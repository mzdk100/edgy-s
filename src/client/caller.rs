use {
    super::{
        super::{
            types::{AtomicReqId, Packet, WsAsyncFn},
            utils::get_path,
        },
        Command, IoError, IoResult,
    },
    serde::{Deserialize, Serialize},
    std::{
        any::type_name,
        collections::HashMap,
        sync::{LazyLock, atomic::Ordering},
    },
    tokio::sync::{Mutex, mpsc::WeakSender, oneshot::channel as oneshot_channel},
};

pub(super) static WS_BINDING_SENDERS: LazyLock<
    Mutex<HashMap<String, WeakSender<crate::client::command::Command>>>,
> = LazyLock::new(Default::default);

/// Trait for calling remote functions via WebSocket from the client side.
///
/// This trait allows calling a function on the server through a WebSocket
/// connection. The arguments are serialized and sent to the server,
/// and the return value is received back.
///
/// # Example
/// ```ignore
/// let result: i32 = (1, 2).call_remotely(add_function).await?;
/// ```
pub trait WsCaller<Args, Ret, Acc> {
    /// Calls a remote function with these arguments.
    ///
    /// The function `f` determines the target path on the server.
    fn call_remotely<F>(self, f: F) -> impl Future<Output = IoResult<Ret>>
    where
        F: WsAsyncFn<Args, Ret, Acc>;
}

impl<Args, Ret, Acc> WsCaller<Args, Ret, Acc> for Args
where
    Args: for<'a> Deserialize<'a> + Serialize,
    Ret: for<'a> Deserialize<'a> + Serialize,
{
    async fn call_remotely<F>(self, _f: F) -> IoResult<Ret>
    where
        F: WsAsyncFn<Args, Ret, Acc>,
    {
        static AUTO_ID: AtomicReqId = AtomicReqId::new(0);

        let path = get_path::<F>();
        let Some(command) = WS_BINDING_SENDERS.lock().await.get(&path).cloned() else {
            return Err(IoError::other(format!(
                "The `{}` path used by the current `{}` function has not been bound to the client.",
                path,
                type_name::<F>()
            )));
        };
        let Some(command) = command.upgrade() else {
            return Err(IoError::other("The command channel is closed."));
        };

        let req_id = AUTO_ID
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
                Some(v.wrapping_add(1))
            })
            .unwrap_or_default();
        let (tx, rx) = oneshot_channel();
        command
            .send(Command::CallRemotely {
                path,
                id: req_id,
                msg: Packet::<Args, Ret>::make_call_message(req_id, self)?,
                opt_return: tx,
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
