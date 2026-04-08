use {
    std::{
        ops::{Deref, DerefMut},
        sync::Arc,
    },
    tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard},
};

/// Weak reference to shared state wrapped in RwLock
pub type State<S> = Arc<RwLock<S>>;

/// Read guard for shared state - allows multiple concurrent readers
///
/// Multiple `Ref` instances can coexist (shared borrow)
pub struct Ref<'a, S> {
    guard: RwLockReadGuard<'a, S>,
}

impl<'a, S> Ref<'a, S> {
    pub(crate) fn new(guard: RwLockReadGuard<'a, S>) -> Self {
        Self { guard }
    }
}

impl<'a, S> Deref for Ref<'a, S> {
    type Target = S;

    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

/// Write guard for shared state - exclusive access
///
/// Only one `RefMut` can exist at a time (mutable borrow)
pub struct RefMut<'a, S> {
    guard: RwLockWriteGuard<'a, S>,
}

impl<'a, S> RefMut<'a, S> {
    pub(crate) fn new(guard: RwLockWriteGuard<'a, S>) -> Self {
        Self { guard }
    }
}

impl<'a, S> Deref for RefMut<'a, S> {
    type Target = S;

    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

impl<'a, S> DerefMut for RefMut<'a, S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.guard
    }
}

/// A wrapper type that provides accessor functionality for connection types.
#[derive(Clone, Debug)]
pub struct Accessor<I> {
    inner: I,
}

impl<I> Deref for Accessor<I> {
    type Target = I;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<I> DerefMut for Accessor<I> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<I> From<I> for Accessor<I> {
    fn from(inner: I) -> Self {
        Self { inner }
    }
}
