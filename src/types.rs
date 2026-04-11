mod accessor;
mod function;
mod packet;
mod router;
mod stream;

pub use {accessor::*, function::*, packet::*, router::*, stream::*};

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
