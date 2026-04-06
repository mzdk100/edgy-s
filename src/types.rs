mod accessor;
mod function;
mod router;
mod stream;

use {
    postcard::{from_bytes, to_allocvec},
    serde::{Deserialize, Serialize},
    std::{
        fmt::Debug,
        io::{Error as IoError, Result as IoResult},
    },
    tokio_tungstenite::tungstenite::Message,
};

pub use {accessor::*, function::*, router::*, stream::*};

/// Request ID type based on selected feature flag
/// Priority: u64 > u32 > u16 > u8 (default)
#[cfg(feature = "req_id_u64")]
pub(super) type ReqId = u64;
#[cfg(all(feature = "req_id_u32", not(feature = "req_id_u64")))]
pub(super) type ReqId = u32;
#[cfg(all(
    feature = "req_id_u16",
    not(any(feature = "req_id_u32", feature = "req_id_u64"))
))]
pub(super) type ReqId = u16;
#[cfg(not(any(feature = "req_id_u16", feature = "req_id_u32", feature = "req_id_u64")))]
pub(super) type ReqId = u8;

/// Atomic version of ReqId for thread-safe ID generation
#[cfg(feature = "req_id_u64")]
pub(super) type AtomicReqId = std::sync::atomic::AtomicU64;
#[cfg(all(feature = "req_id_u32", not(feature = "req_id_u64")))]
pub(super) type AtomicReqId = std::sync::atomic::AtomicU32;
#[cfg(all(
    feature = "req_id_u16",
    not(any(feature = "req_id_u32", feature = "req_id_u64"))
))]
pub(super) type AtomicReqId = std::sync::atomic::AtomicU16;
#[cfg(not(any(feature = "req_id_u16", feature = "req_id_u32", feature = "req_id_u64")))]
pub(super) type AtomicReqId = std::sync::atomic::AtomicU8;

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub(super) enum Packet<A, B> {
    Call(ReqId, A),
    Ret(ReqId, B),
}

unsafe impl<A, B> Send for Packet<A, B> {}

impl<A, B> Packet<A, B> {
    pub fn from_message(message: &Message) -> IoResult<Self>
    where
        A: for<'a> Deserialize<'a>,
        B: for<'a> Deserialize<'a>,
    {
        match message {
            Message::Binary(bytes) => from_bytes(bytes).map_err(IoError::other),
            _ => Err(IoError::other("Unsupported message.")),
        }
    }

    pub fn make_ret_message(id: ReqId, data: B) -> IoResult<Message>
    where
        A: Serialize,
        B: Serialize,
    {
        Ok(Message::Binary(
            to_allocvec(&Self::Ret(id, data))
                .map_err(IoError::other)?
                .into(),
        ))
    }

    pub fn make_call_message(id: ReqId, data: A) -> IoResult<Message>
    where
        A: Serialize,
        B: Serialize,
    {
        Ok(Message::Binary(
            to_allocvec(&Self::Call(id, data))
                .map_err(IoError::other)?
                .into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_packet() -> anyhow::Result<()> {
        let data = Packet::<(), i32>::make_ret_message(100 as ReqId, 3)?;
        let ret = Packet::<(), i32>::from_message(&data)?;
        assert_eq!(ret, Packet::Ret(100 as ReqId, 3));

        Ok(())
    }
}
