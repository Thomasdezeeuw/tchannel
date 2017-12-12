// Copyright 2017 Thomas de Zeeuw
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT
// or http://opensource.org/licenses/MIT>, at your option. This file may not be
// used, copied, modified, or distributed except according to those terms.

//! Oneshot channels.
//!
//! See the [root of the crate] for more documentation.
//!
//! [root of the crate]: ../index.html

use std::{ptr, mem};
use std::sync::Arc;
use std::sync::atomic::{AtomicPtr, Ordering};

#[cfg(feature = "futures")]
use futures::{Async, Poll, Future, IntoFuture};
#[cfg(feature = "futures")]
use futures::task::AtomicTask;

pub use super::SendError;

/// The error returned by trying to receive a value.
#[derive(Debug)]
pub enum ReceiveError<T> {
    /// The channel is currently empty, try again later.
    Empty(Receiver<T>),
    /// The channel is empty and the sending side of the channel is
    /// disconnected, which means that no value is coming.
    Disconnected,
}

/// This error is returned by the [`ReceiveFuture`], to indicate that the
/// sending side of the channel was dropped without sending a value. This is the
/// same error as [`ReceiveError::Disconnected`].
///
/// [`ReceiveFuture`]: struct.ReceiveFuture.html
/// [`ReceiveError::Disconnected`]: enum.ReceiveError.html#variant.Disconnected
#[cfg(feature = "futures")]
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct SenderDisconnectedError;

/// Create a new oneshot channel.
pub fn channel<T: Send + Sync>() -> (Sender<T>, Receiver<T>) {
    let inner = Arc::new(Inner::new());
    (Sender { inner: Arc::clone(&inner) }, Receiver { inner: inner })
}

/// The sending side of the channel. See [`channel`] to create a channel.
///
/// [`channel`]: fn.channel.html
#[derive(Debug)]
pub struct Sender<T> {
    inner: Arc<Inner<T>>,
}

impl<T> Sender<T> {
    /// Send a `value` across the channel. If this returns an error it means
    /// the receiving end has been disconnected.
    pub fn send(self, value: T) -> Result<(), SendError<T>> {
        if self.is_disconnected() {
            Err(SendError(value))
        } else {
            let value_ptr = Box::into_raw(Box::new(value));
            self.inner.value.store(value_ptr, Ordering::Relaxed);
            // The task will be notified once `Sender` gets dropped.
            Ok(())
        }
    }

    /// Check if the other side is disconnected.
    fn is_disconnected(&self) -> bool {
        // If this `Sender` is the only one with a reference to `inner` it means
        // the `Receiver` is disconnected.
        Arc::strong_count(&self.inner) == 1
    }
}

#[cfg(feature = "futures")]
impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        self.inner.task.notify();
    }
}

/// The receiving side of the channel. See [`channel`] to create a channel.
///
/// This `Receiver` may be turned into a [`Future`] using the [`IntoFuture`]
/// trait (requires the `with-futures` feature).
///
/// [`channel`]: fn.channel.html
/// [`Future`]: ../../futures/future/trait.IntoFuture.html
/// [`IntoFuture`]: ../../futures/future/trait.Future.html
#[derive(Debug)]
pub struct Receiver<T> {
    inner: Arc<Inner<T>>,
}

impl<T> Receiver<T> {
    /// Try to receive a single value, none blocking. If the channel is empty or
    /// if the sender side is disconnected this will return an error.
    pub fn try_receive(self) -> Result<T, ReceiveError<T>> {
        let value_ptr = self.inner.value.swap(ptr::null_mut(), Ordering::Relaxed);
        if !value_ptr.is_null() {
            // Safe because of the contract in the `value` field.
            let value = unsafe { Box::from_raw(value_ptr) };
            Ok(*value)
        } else if self.is_disconnected() {
            Err(ReceiveError::Disconnected)
        } else  {
            Err(ReceiveError::Empty(self))
        }
    }

    /// Check if the other side is disconnected.
    fn is_disconnected(&self) -> bool {
        // If this `Receiver` is the only one with a reference to `inner` it
        // means the `Sender` is disconnected.
        Arc::strong_count(&self.inner) == 1
    }
}

#[cfg(feature = "futures")]
impl<T> IntoFuture for Receiver<T> {
    type Future = ReceiveFuture<T>;
    type Item = T;
    type Error = SenderDisconnectedError;
    fn into_future(self) -> ReceiveFuture<T> {
        ReceiveFuture {
            receiver: Some(self),
        }
    }
}

/// A future to resolve a single value from a oneshot channel, see [`Receiver`].
///
/// The execution of this future will panic if it's not in the context of a
/// [`Task`].
///
/// [`Receiver`]: struct.Receiver.html
/// [`Task`]: ../../futures/task/struct.Task.html
#[cfg(feature = "futures")]
#[derive(Debug)]
#[must_use = "futures do nothing unless polled"]
pub struct ReceiveFuture<T> {
    /// If this is `None` it means a value was already received.
    receiver: Option<Receiver<T>>,
}

#[cfg(feature = "futures")]
impl<T> Future for ReceiveFuture<T> {
    type Item = T;
    type Error = SenderDisconnectedError;
    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        use self::ReceiveError::*;
        match self.receiver.take() {
            Some(receiver) => {
                // We register our interest before we trying receiving a value
                // to make sure if the send happens first, returns nothing, and
                // then the sender sends the value before `try_receive` returns
                // the task still gets notified.
                receiver.inner.task.register();
                match receiver.try_receive() {
                    Ok(value) => Ok(Async::Ready(value)),
                    Err(Empty(receiver)) => {
                        self.receiver = Some(receiver);
                        Ok(Async::NotReady)
                    },
                    Err(Disconnected) => Err(SenderDisconnectedError),
                }
            },
            None => panic!("tried to poll ReceiveFuture after completion"),
        }
    }
}

/// The inner data structure backing the `Sender` and `Receiver`.
#[derive(Debug)]
struct Inner<T> {
    /// The value shared between the `Sender` and `Receiver`. It starts as null,
    /// then only the `Sender` may set it a none null value, which **must point
    /// to valid memory**, once. Then the `Receiver` may replace that pointer
    /// will null again, only once, to indicate the value was received.
    value: AtomicPtr<T>,
    /// A possible task to wake up once a value is send. The `Receiver` may swap
    /// this to a none null value, making sure to drop old tasks, and the
    /// `Sender` may set this to null again once it notified the task.
    #[cfg(feature = "futures")]
    task: AtomicTask,
}

impl<T> Inner<T> {
    fn new() -> Inner<T> {
        Inner {
            value: AtomicPtr::default(),
            #[cfg(feature = "futures")]
            task: AtomicTask::new()
        }
    }
}

impl<T> Drop for Inner<T> {
    fn drop(&mut self) {
        let value_ptr = self.value.load(Ordering::Relaxed);
        if !value_ptr.is_null() {
            mem::drop(unsafe { Box::from_raw(value_ptr) })
        }
    }
}
