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

use std::mem;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(feature = "futures")]
use futures::{Async, AsyncSink, StartSend, Poll, Stream, Sink};
#[cfg(feature = "futures")]
use futures::task::AtomicTask;

use self::segment::{Segment, SEGMENT_SIZE, Expanded};
pub use super::{SendError, ReceiveError};

/// Create a new multiple producers, single consumer channel.
pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
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
        old_segment: None,
        #[cfg(feature = "futures")]
        registered: false,
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
    /// The current `Segment` we're reading from.
    current: Arc<Segment<T>>,
    /// The current read index, may never be bigger then `SEGMENT_SIZE`.
    read_index: usize,
    /// Shared state between the `Receiver` and `Sender`.
    shared: Arc<Shared>,
    /// This a linked list (what `Segment`s are) of old segments that have been
    /// completely written to and read from, but are not yet ready for reuse,
    /// e.g. `Sender` still has a reference to it, see `try_reuse`.
    old_segment: Option<Arc<Segment<T>>>,
    /// Wether or not the sender has registered its task.
    #[cfg(feature = "futures")]
    registered: bool,
}

/// The maximum number of `Segment`s reused in a single receive call.
const MAX_REUSE_PER_RECV: usize = 10;

impl<T> Receiver<T> {
    /// Try to receive a single value, none blocking. If the channel is empty or
    /// if the sender side is disconnected this will return an error.
    pub fn try_receive(&mut self) -> Result<T, ReceiveError> {
        self.try_reuse();
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
        match self.current.next_segment() {
            Some(next_segment) => {
                let mut old_segment = mem::replace(&mut self.current, next_segment);
                self.read_index = 1;

                // Before we can actually reuse the `old_segment`, we need to
                // make sure we have alone access to it. If `self.old_segment`
                // is something or we can't get mutable acess to the
                // `old_segment` Arc it means that we won't have alone access.
                if self.old_segment.is_none() {
                    let can_reuse = if let Some(old_segment) = Arc::get_mut(&mut old_segment) {
                        let next_segment = old_segment.reset();
                        // We already have a reference to the next segment, so
                        // we'll drop it.
                        mem::drop(next_segment);
                        true
                    } else {
                        false
                    };

                    if can_reuse {
                        self.current.expand_with_segment(old_segment);
                    } else {
                        // We'll try to reuse it later.
                        self.old_segment = Some(old_segment);
                    }
                }
                // Else we have a previous segment already stored, so we'll get
                // to it.
                true
            },
            None => false
        }
    }

    /// Try to reuse old segments in `self.old_segment`, if any.
    fn try_reuse(&mut self) {
        for _ in 0..MAX_REUSE_PER_RECV {
            // Try to reset the `self.old_segment` Segment and return it's next
            // segment. This can fail for various reasons and will return (to
            // the caller) instead.
            let next_segment = match self.old_segment {
                None => return,
                Some(ref mut old_segment) => {
                    // Try to get alone access to the segment.
                    if let Some(old_segment) = Arc::get_mut(old_segment) {
                        let next_segment = old_segment.reset();
                        // This is safe because we checked in
                        // `update_current_segment` if the segment has a next
                        // one and we started using that before we could ever
                        // get here.
                        debug_assert!(next_segment.is_some(),
                            "old segment has no next segment");
                        next_segment.unwrap()
                    } else {
                        // A `Sender` still has a reference.
                        return;
                    }
                },
            };

            if Arc::ptr_eq(&next_segment, &self.current) {
                // If the next segment points to the segment we currently
                // processing then we can reuse the segment and we're done.
                let processed_segment = mem::replace(&mut self.old_segment,
                    None);
                self.current.expand_with_segment(processed_segment.unwrap());
                return;
            } else {
                // Else we set the `old_segment` to `next_segment` and try to
                // reuse that.
                let processed_segment = mem::replace(&mut self.old_segment,
                    Some(next_segment));
                self.current.expand_with_segment(processed_segment.unwrap());
            }
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
        // We register our interest only once before doing anything else. This
        // way we're sure not to miss any notifications.
        if !self.registered {
            self.shared.task.register();
        }
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
    /// Wether or not the receiver is alive.
    receiver_alive: AtomicBool,
    /// A task to wake up once a value is send. The `Receiver` may register
    /// itself, while the `Sender` must notify the task once a value is send.
    #[cfg(feature = "futures")]
    task: AtomicTask,
}
