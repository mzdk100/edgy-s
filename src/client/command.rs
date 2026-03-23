use {
    super::{super::types::ReqId, handler::WsCall},
    std::io::Result as IoResult,
    tokio::{
        sync::{mpsc::Sender as MpscSender, oneshot::Sender as OneshotSender},
        task::JoinHandle,
    },
    tokio_tungstenite::tungstenite::Message,
};

#[derive(Debug)]
pub enum Command {
    AddWsRoute {
        path: String,
        stream: MpscSender<WsCall>,
        task: JoinHandle<()>,
        opt_return: OneshotSender<IoResult<()>>,
    },

    RemoveWsRoute {
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
