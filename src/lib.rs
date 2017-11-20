// Copyright 2017 Thomas de Zeeuw
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT
// or http://opensource.org/licenses/MIT>, at your option. This file may not be
// used, copied, modified, or distributed except according to those terms.

//! Optimized channels for Rust. This crate contains verious version of the same
//! concept, channels. The available versions are:
//!
//! - oneshot, allows a single value to be send and received.
//! - mpsc, allows for multiple producers (senders) and a single consumer
//! (receiver).
//!
//! All these different versions work in the same way. All modules will have a
//! `channel` function which returns a `Sender` and a `Receiver`. The `Sender`
//! can only send values and the `Receiver` can only receive values. The remaing
//! API are errors, which are a bit more channel version specific, and adapters
//! for the `futures` crate, see [below](#futures).
//!
//! # Example
//!
//! We'll use the oneshot channel for this example, but all versions will have
//! roughly the same type and structure setup.
//!
//! ```
//! # extern crate tchannel;
//! # fn main() {
//! use tchannel::oneshot::{channel, Sender, Receiver, ReceiveError};
//!
//! // Create a new channel, which has a sender and a receiver.
//! let (sender, receiver): (Sender<_>, Receiver<_>) = channel();
//!
//! // First we'll try to receive a value, but the channel should be empty so
//! // it will return an error. We can recover the receiver (`try_receive`
//! // consumes `Receiver` in case of an oneshot channel) via the `Empty` error.
//! let receiver = match receiver.try_receive() {
//!     Err(ReceiveError::Empty(receiver)) => receiver,
//!     _ => panic!("expected the channel to be empty"),
//! };
//!
//! // Now we'll send our value along the channel. In case of a oneshot channel
//! // this will consume the `Sender`.
//! //
//! // This will only return an error if the receiving side of the channel is
//! // disconnected, in which case it will return a `SendError` which allows
//! // the send value to retrieved.
//! assert_eq!(sender.send("Hello world"), Ok(()));
//!
//! // Here we'll receive the value, making sure it's the same as what we send.
//! assert_eq!(receiver.try_receive().unwrap(), "Hello world");
//! # }
//! ```
//!
//! # Futures
//!
//! Support for the [`futures`] crate is available if the `with-futures` feature
//! is enabled (disabled by default).
//!
//! TODO: add example usage.
//!
//! [`futures`]: https://crates.io/crates/futures

#![warn(missing_docs)]
#![warn(missing_debug_implementations)]
#![warn(unused_results)]

#[cfg(feature = "futures")]
extern crate futures;

mod atomic_arc;
mod atomic_cell;

pub mod mpsc;
pub mod oneshot;

/// This error will be returned if the receiving side of the channel is
/// disconnected, while trying to send a value across the channel.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct SendError<T>(pub T);

/// The error returned by trying to receive a value.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum ReceiveError {
    /// The channel is empty, more values are possible.
    Empty,
    /// The channel is empty and the sending side of the channel is
    /// disconnected, which means that no more values are coming.
    Disconnected,
}
