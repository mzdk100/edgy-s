#[cfg(any(feature = "postcard", feature = "cbor4", feature = "serde_json"))]
use ser_impl::{from_bytes, to_bytes};
use {
    super::ReqId,
    serde::{Deserialize, Serialize},
    std::io::{Error as IoError, Result as IoResult},
    tokio_tungstenite::tungstenite::Message,
};

/// --- postcard serialization backend (highest priority) ---
#[cfg(feature = "postcard")]
mod ser_impl {
    use {
        super::*,
        postcard::{from_bytes as from_slice, to_allocvec},
    };

    pub(super) fn to_bytes<T: Serialize>(value: &T) -> IoResult<Vec<u8>> {
        to_allocvec(value).map_err(IoError::other)
    }

    pub(super) fn from_bytes<'de, T: Deserialize<'de>>(bytes: &'de [u8]) -> IoResult<T> {
        from_slice(bytes).map_err(IoError::other)
    }
}

/// --- cbor4 serialization backend (medium priority) ---
#[cfg(all(feature = "cbor4", not(feature = "postcard")))]
mod ser_impl {
    use {
        super::*,
        cbor4ii::serde::{from_slice, to_vec},
    };

    pub(super) fn to_bytes<T: Serialize>(value: &T) -> IoResult<Vec<u8>> {
        to_vec(Vec::new(), value).map_err(IoError::other)
    }

    pub(super) fn from_bytes<'de, T: Deserialize<'de>>(bytes: &'de [u8]) -> IoResult<T> {
        from_slice(bytes).map_err(IoError::other)
    }
}

/// --- serde_json serialization backend (lowest priority) ---
#[cfg(all(
    feature = "serde_json",
    not(any(feature = "postcard", feature = "cbor4"))
))]
mod ser_impl {
    use {
        super::*,
        serde_json::{from_slice, to_vec},
    };

    pub(super) fn to_bytes<T: Serialize>(value: &T) -> IoResult<Vec<u8>> {
        to_vec(value).map_err(IoError::other)
    }

    pub(super) fn from_bytes<'de, T: Deserialize<'de>>(bytes: &'de [u8]) -> IoResult<T> {
        from_slice(bytes).map_err(IoError::other)
    }
}

// --- No serialization backend enabled ---
#[cfg(not(any(feature = "postcard", feature = "cbor4", feature = "serde_json")))]
compile_error!(
    "No serialization backend enabled. \
     Please enable one of: `postcard` (default, compact), `cbor4` (CBOR format), or `serde_json` (JSON format)."
);

#[cfg_attr(
    any(feature = "postcard", feature = "cbor4", feature = "serde_json"),
    derive(Deserialize, Serialize)
)]
#[derive(Debug, PartialEq)]
pub enum Packet<A, B> {
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
            #[cfg(any(feature = "postcard", feature = "cbor4", feature = "serde_json"))]
            Message::Binary(bytes) => from_bytes(bytes),
            _ => Err(IoError::other(
                "Unsupported message. Please try to change other serialization backend.",
            )),
        }
    }

    #[cfg(any(feature = "postcard", feature = "cbor4", feature = "serde_json"))]
    pub fn make_ret_message(id: ReqId, data: B) -> IoResult<Message>
    where
        A: Serialize,
        B: Serialize,
    {
        Ok(Message::Binary(to_bytes(&Self::Ret(id, data))?.into()))
    }

    #[cfg(not(any(feature = "postcard", feature = "cbor4", feature = "serde_json")))]
    pub fn make_ret_message(_id: ReqId, _data: B) -> IoResult<Message>
    where
        A: Serialize,
        B: Serialize,
    {
        unimplemented!("Please enable `postcard`, `cbor4` or `serde_json` feature.")
    }

    #[cfg(any(feature = "postcard", feature = "cbor4", feature = "serde_json"))]
    pub fn make_call_message(id: ReqId, data: A) -> IoResult<Message>
    where
        A: Serialize,
        B: Serialize,
    {
        Ok(Message::Binary(to_bytes(&Self::Call(id, data))?.into()))
    }

    #[cfg(not(any(feature = "postcard", feature = "cbor4", feature = "serde_json")))]
    pub fn make_call_message(_id: ReqId, _data: A) -> IoResult<Message>
    where
        A: Serialize,
        B: Serialize,
    {
        unimplemented!("Please enable `postcard`, `cbor4` or `serde_json` feature.")
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
