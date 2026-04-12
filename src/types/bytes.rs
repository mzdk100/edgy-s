use hyper::body::Bytes;
#[cfg(feature = "serde_json")]
use serde_json::{Value, from_slice, to_vec};

/// Trait for types that can be created from `Bytes`.
pub trait FromBytes {
    /// Creates this type from a `Bytes` value.
    fn from(value: Bytes) -> Self;
}

impl FromBytes for String {
    fn from(value: Bytes) -> Self {
        String::from_utf8(value.to_vec()).unwrap_or_default()
    }
}

impl FromBytes for Vec<u8> {
    fn from(value: Bytes) -> Self {
        value.into()
    }
}

impl FromBytes for Box<[u8]> {
    fn from(value: Bytes) -> Self {
        value.to_vec().into()
    }
}

/// Trait for types that can be converted into `Bytes`.
pub trait IntoBytes {
    /// Converts this type into `Bytes`.
    fn into(self) -> Bytes;
}

impl<'a> IntoBytes for &'a str {
    fn into(self) -> Bytes {
        Bytes::copy_from_slice(self.as_bytes())
    }
}

impl<'a> IntoBytes for &'a [u8] {
    fn into(self) -> Bytes {
        Bytes::copy_from_slice(self)
    }
}

impl IntoBytes for String {
    fn into(self) -> Bytes {
        Bytes::copy_from_slice(self.as_bytes())
    }
}

#[cfg(feature = "serde_json")]
impl FromBytes for Value {
    fn from(value: Bytes) -> Self {
        from_slice(&value).unwrap_or_default()
    }
}

#[cfg(feature = "serde_json")]
impl IntoBytes for Value {
    fn into(self) -> Bytes {
        to_vec(&self).unwrap_or_default().into()
    }
}
