use {
    super::{super::types::ReqId, StreamingBody},
    hyper::{HeaderMap, Uri},
    std::{io::Result as IoResult, net::SocketAddr},
    tokio::sync::watch::Sender as WatchSender,
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
        stream: MpscSender<(
            Uri,
            SocketAddr,
            HeaderMap,
            Message,
            OneshotSender<Option<Message>>,
        )>,
        opt_return: OneshotSender<IoResult<()>>,
        open: MpscSender<(
            Uri,
            SocketAddr,
            HeaderMap,
            WatchSender<HeaderMap>,
            OneshotSender<()>,
        )>,
        close: MpscSender<(Uri, SocketAddr, HeaderMap)>,
    },

    RemoveWsRoute {
        path: String,
        opt_return: OneshotSender<IoResult<()>>,
    },

    AddHttpRoute {
        path: String,
        req_tx: MpscSender<(
            Uri,
            SocketAddr,
            HeaderMap,
            StreamingBody,
            OneshotSender<(HeaderMap, StreamingBody)>,
        )>,
        opt_return: OneshotSender<IoResult<()>>,
        task: JoinHandle<()>,
    },

    RemoveHttpRoute {
        path: String,
        opt_return: OneshotSender<IoResult<()>>,
    },

    Transfer {
        uri: Uri,
        socket_addr: SocketAddr,
        msg: Message,
        headers: HeaderMap,
        ret_tx: OneshotSender<Option<Message>>,
    },

    CallRemotely {
        path: String,
        socket_addr: SocketAddr,
        id: ReqId,
        msg: Message,
        ret_tx: OneshotSender<IoResult<Message>>,
    },

    CommitReturn {
        path: String,
        socket_addr: SocketAddr,
        id: crate::types::ReqId,
        msg: Message,
    },

    Request {
        uri: Uri,
        socket_addr: SocketAddr,
        headers: HeaderMap,
        body: StreamingBody,
        ret_tx: OneshotSender<(HeaderMap, StreamingBody)>,
    },

    WsOpen {
        uri: Uri,
        socket_addr: SocketAddr,
        headers: HeaderMap,
        res_tx: OneshotSender<HeaderMap>,
    },

    WsClose {
        uri: Uri,
        socket_addr: SocketAddr,
        headers: HeaderMap,
    },
}
