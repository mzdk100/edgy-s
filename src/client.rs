use {
    super::types::{Accessor, AsyncFun, AtomicReqId, Packet, ReqId, Router},
    futures_util::{
        SinkExt, StreamExt,
        future::{Either, select},
    },
    serde::{Deserialize, Serialize},
    std::{
        any::type_name,
        collections::HashMap,
        io::{Error as IoError, ErrorKind, Result as IoResult},
        sync::{LazyLock, atomic::Ordering},
    },
    tokio::{
        runtime::{Builder, Runtime},
        sync::{
            Mutex,
            mpsc::{Receiver as MpscReceiver, Sender as MpscSender, channel as mpsc_channel},
            oneshot::{Sender as OneshotSender, channel as oneshot_channel},
        },
        task::JoinHandle,
    },
    tokio_tungstenite::{
        connect_async,
        tungstenite::{Message, client::IntoClientRequest},
    },
    tracing::{error, info},
};

static BIND_SENDERS: LazyLock<Mutex<HashMap<String, MpscSender<Command>>>> =
    LazyLock::new(Default::default);

pub type ClientAccessor = Accessor<String>;

#[derive(Debug)]
enum Command {
    AddRoute {
        path: String,
        stream: MpscSender<(ReqId, Message, OneshotSender<Message>)>,
        task: JoinHandle<()>,
        opt_return: OneshotSender<IoResult<()>>,
    },

    RemoveRoute {
        path: String,
        opt_return: OneshotSender<IoResult<()>>,
    },

    CallRemotely {
        path: String,
        id: ReqId,
        msg: Message,
        opt_return: OneshotSender<IoResult<Message>>,
    },
}

pub struct EdgyClient<U> {
    base_url: U,
    rt: Runtime,
    command: MpscSender<Command>,
    task: Option<JoinHandle<IoResult<()>>>,
}

impl<U> Router<String> for EdgyClient<U>
where
    U: AsRef<str>,
{
    async fn add_route<F, P, Args, Ret>(&self, path: P, handler: F) -> IoResult<()>
    where
        F: AsyncFun<Args, Ret, String>,
        Args: for<'a> Deserialize<'a> + Serialize,
        Ret: for<'a> Deserialize<'a> + Serialize,
        P: AsRef<str>,
    {
        let url = if self.base_url.as_ref().ends_with("/") {
            format!(
                "{}{}",
                self.base_url.as_ref().trim_end_matches('/'),
                path.as_ref()
            )
        } else {
            format!("{}{}", self.base_url.as_ref(), path.as_ref())
        };
        info!("Connect to {}", url);
        let (stream, _response) = connect_async(
            url.into_client_request()
                .map_err(|e| IoError::new(ErrorKind::HostUnreachable, e))?,
        )
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
        let path2 = path.clone();

        let (mut write, mut read) = stream.split();
        let (tx, mut rx) = mpsc_channel(2);
        let task = self.rt.spawn(async move {
            let mut pending_requests = HashMap::<_, OneshotSender<_>>::new();

            loop {
                match select(read.next(), Box::pin(rx.recv())).await {
                    Either::Left((Some(Ok(Message::Close(c))), _)) => {
                        dbg!(c);
                        break;
                    }
                    Either::Left((None, _)) | Either::Right((None, _)) => break,

                    Either::Left((Some(Ok(msg)), _)) => {
                        match Packet::<Args, Ret>::from_message(&msg) {
                            Ok(Packet::Call(id, args)) => {
                                let ret = match Packet::<Args, Ret>::make_ret_message(
                                    id,
                                    handler.call(path.clone().into(), args).await,
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
                        }
                    }

                    Either::Right((Some((id, msg, opt_return)), _)) => {
                        if let Err(e) = write.send(msg).await {
                            error!(?e, "Can't send the message.");
                        }
                        pending_requests.insert(id, opt_return);
                    }

                    Either::Left((Some(Err(e)), _)) => {
                        error!(?e, "Can't receive the message.")
                    }
                };
            }
        });

        let (ret_tx, ret_rx) = oneshot_channel();
        self.command
            .send(Command::AddRoute {
                path: path2,
                stream: tx,
                task,
                opt_return: ret_tx,
            })
            .await
            .map_err(IoError::other)?;
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

impl<U> EdgyClient<U>
where
    U: 'static,
{
    pub fn new(base_url: U, num_workers: usize) -> IoResult<Self> {
        let rt = Builder::new_multi_thread()
            .worker_threads(num_workers)
            .enable_all()
            .build()?;
        let (tx, rx) = mpsc_channel(2);
        let task = rt.spawn(Self::worker(rx));

        Ok(Self {
            base_url,
            rt,
            command: tx,
            task: Some(task),
        })
    }

    pub async fn run(mut self) -> IoResult<Self> {
        if let Some(task) = self.task.take() {
            task.await.map_err(IoError::other)??;
        }

        Ok(self)
    }

    async fn worker(mut command: MpscReceiver<Command>) -> IoResult<()> {
        let mut tasks = HashMap::new();

        while let Some(item) = command.recv().await {
            match item {
                Command::AddRoute {
                    path,
                    stream,
                    task,
                    opt_return,
                } => {
                    opt_return
                        .send(if tasks.contains_key(&path) {
                            Err(IoError::other(format!("Can't add route: {}", path)))
                        } else {
                            tasks.insert(path, (stream, task));
                            Ok(())
                        })
                        .map_or_else(|e| e.map_err(IoError::other), Ok)?;
                }

                Command::RemoveRoute { path, opt_return } => opt_return
                    .send(tasks.remove(&path).map_or(
                        Err(IoError::other(format!("Can't remove route: {}", path))),
                        |(_, i)| Ok(i.abort()),
                    ))
                    .map_or_else(|e| e, Ok)?,

                Command::CallRemotely {
                    path,
                    id,
                    msg,
                    opt_return,
                } => {
                    if let Some((sender, _)) = tasks.get(path.as_str()) {
                        let (tx, rx) = oneshot_channel();
                        sender.send((id, msg, tx)).await.map_err(IoError::other)?;
                        if let Err(e) = opt_return.send(rx.await.map_err(IoError::other)) {
                            error!(?e, "Can't send the message.");
                        }
                    }
                }
            }
        }

        Ok(())
    }

    pub fn abort(self) {
        if let Some(task) = self.task {
            task.abort();
        }
    }
}

pub trait ServiceCaller<Args, Ret, Acc> {
    fn call_remotely<F>(self, f: F) -> impl Future<Output = IoResult<Ret>>
    where
        F: AsyncFun<Args, Ret, Acc>;
}

impl<Args, Ret, Acc> ServiceCaller<Args, Ret, Acc> for Args
where
    Args: for<'a> Deserialize<'a> + Serialize,
    Ret: for<'a> Deserialize<'a> + Serialize,
{
    async fn call_remotely<F>(self, _f: F) -> IoResult<Ret>
    where
        F: AsyncFun<Args, Ret, Acc>,
    {
        static AUTO_ID: AtomicReqId = AtomicReqId::new(0);

        let path = F::get_path();
        let Some(command) = BIND_SENDERS.lock().await.get(&path).map(|i| i.clone()) else {
            return Err(IoError::other(format!(
                "The `{}` path used by the current `{}` function has not been bound to the client.",
                path,
                type_name::<F>()
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
