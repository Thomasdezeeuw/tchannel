// Copyright 2017 Thomas de Zeeuw
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT
// or http://opensource.org/licenses/MIT>, at your option. This file may not be
// used, copied, modified, or distributed except according to those terms.

// TODO: update docs and comments.

//! Multiple producers, single consumer channels.
//!
//! See the [root of the crate] for more documentation.
//!
//! [root of the crate]: ../index.html

mod segment;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(feature = "futures")]
use futures::{Async, AsyncSink, StartSend, Poll, Stream, Sink};
#[cfg(feature = "futures")]
use futures::task::AtomicTask;

use self::segment::{Segment, SEGMENT_SIZE, Expanded};
pub use super::{SendError, ReceiveError};

/// Create a new multiple producers, single consumer channel.
pub fn channel<T: Send + Sync>() -> (Sender<T>, Receiver<T>) {
    let segment = Arc::new(Segment::empty());
    let shared = Arc::new(Shared{
        receiver_alive: AtomicBool::new(true),
        #[cfg(feature = "futures")]
        task: AtomicTask::new(),
    });
    let sender = Sender {
        current: Arc::clone(&segment),
        shared: Arc::clone(&shared),
    };
    let receiver = Receiver {
        current: segment,
        read_index: 0,
        shared: shared,
    };
    (sender, receiver)
}

/// The sending side of the channel. See [`channel`] to create a channel. This
/// can be safely and efficiently cloned to send values across the channel from
/// multiple threads, or locations in your code.
///
/// [`channel`]: fn.channel.html
#[derive(Debug)]
pub struct Sender<T> {
    /// The `Segment` is `Sender` is currently working with. This will always
    /// point to a `Segment` that is still in working order, however it doesn't
    /// have to be the tail of the linked list.
    current: Arc<Segment<T>>,
    /// Shared state between the `Receiver` and `Sender`.
    shared: Arc<Shared>,
}

impl<T> Sender<T> {
    /// Send a value across the channel. If this returns an error it means the
    /// receiving end has been disconnected.
    pub fn send(&mut self, value: T) -> Result<(), SendError<T>> {
        // We first check if the receiver is still connected. However since it
        // may very well be disconnected after this call, while we're adding a
        // value, we don't depend on it to be accurate, hence the relaxed
        // ordering.
        if !self.shared.receiver_alive.load(Ordering::Relaxed) {
            // Receiver is disconnected.
            return Err(SendError(value));
        }

        // Append the value to the linked list of `Segment`s, since `Segment`
        // itself can deal with overflowing we just have to update our tail.
        if let Expanded::Expanded(next_segment) = self.current.append(value) {
            self.current = next_segment;
        }

        #[cfg(feature = "futures")]
        self.shared.task.notify();
        Ok(())
    }
}

#[cfg(feature = "futures")]
impl<T> Sink for Sender<T> {
    type SinkItem = T;
    type SinkError = SendError<T>;
    fn start_send(&mut self, item: T) -> StartSend<T, SendError<T>> {
        match self.send(item) {
            Ok(()) => Ok(AsyncSink::Ready),
            Err(err) => Err(err),
        }
    }
    fn poll_complete(&mut self) -> Poll<(), SendError<T>> {
        Ok(Async::Ready(()))
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Sender<T> {
        Sender {
            current: Arc::clone(&self.current),
            shared: Arc::clone(&self.shared),
        }
    }
}

#[cfg(feature = "futures")]
impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        self.shared.task.notify();
    }
}

/// The receiving side of the channel. See [`channel`] to create a channel.
///
/// [`channel`]: fn.channel.html
#[derive(Debug)]
pub struct Receiver<T> {
    current: Arc<Segment<T>>,
    /// The current read index, may never be bigger then `SEGMENT_SIZE`.
    read_index: usize,
    /// Shared state between the `Receiver` and `Sender`.
    shared: Arc<Shared>,
}

impl<T> Receiver<T> {
    /// Try to receive a single value, none blocking. If the channel is empty or
    /// if the sender side is disconnected this will return an error.
    pub fn try_receive(&mut self) -> Result<T, ReceiveError> {
        let index = match self.get_read_index() {
            Some(index) => index,
            None => {
                if self.update_current_segment() {
                    0
                } else if self.is_disconnected() {
                    return Err(ReceiveError::Disconnected);
                } else {
                    return Err(ReceiveError::Empty);
                }
            },
        };

        match self.current.try_pop(index) {
            Some(value) => Ok(value),
            None => {
                self.read_index -= 1;
                if self.is_disconnected() {
                    Err(ReceiveError::Disconnected)
                } else {
                    Err(ReceiveError::Empty)
                }
            },
        }
    }

    /// Get the next read index of the `Segment`, if `None` is returned it
    /// means the `Segment` is empty.
    fn get_read_index(&mut self) -> Option<usize> {
        match self.read_index {
            current if current >= SEGMENT_SIZE => None,
            current => {
                self.read_index += 1;
                Some(current)
            },
        }
    }

    /// Update the `current` segment, returning a boolean wether or not a next
    /// segment exists.
    fn update_current_segment(&mut self) -> bool {
        // TODO: check if the current `Segment` can be reused, by calling
        // `reset`.
        //
        // Arc::strong_count(&self.current) == 1 => alone access.
        // Otherwise store store it somewhere maybe? Or also let the Sender
        // append it to the back, but it can still be dropped without reuse
        // then.
        match self.current.next_segment() {
            Some(next_segment) => {
                self.current = next_segment;
                self.read_index = 1;
                true
            },
            None => false
        }
    }

    /// Check if the other side is disconnected.
    fn is_disconnected(&self) -> bool {
        // If this receiver is the only one with a reference to the queue it
        // means there are no senders left.
        Arc::strong_count(&self.shared) == 1
    }
}

#[cfg(feature = "futures")]
impl<T> Stream for Receiver<T> {
    type Item = T;
    /// No error will ever be returned and it can be safely ignored.
    type Error = ();
    fn poll(&mut self) -> Poll<Option<T>, ()> {
        // We register our interest before anything so what ever happens we get
        // a notification.
        self.shared.task.register();
        match self.try_receive() {
            Ok(value) => Ok(Async::Ready(Some(value))),
            Err(err) => match err {
                ReceiveError::Empty => Ok(Async::NotReady),
                ReceiveError::Disconnected => Ok(Async::Ready(None)),
            },
        }
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        self.shared.receiver_alive.store(false, Ordering::Relaxed);
    }
}

/// Shared state between the `Receiver` and the `Sender`.
#[derive(Debug)]
struct Shared {
    /// Wether or not the receiver alive.
    receiver_alive: AtomicBool,
    /// A task to wake up once a value is send. The `Receiver` may register
    /// itself, while the `Sender` must notify the task once a value is send.
    #[cfg(feature = "futures")]
    task: AtomicTask,
}
