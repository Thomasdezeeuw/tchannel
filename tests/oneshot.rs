// Copyright 2017 Thomas de Zeeuw
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT
// or http://opensource.org/licenses/MIT>, at your option. This file may not be
// used, copied, modified, or distributed except according to those terms.

extern crate channel;
#[cfg(feature = "futures")]
extern crate futures;

use std::{mem, thread};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

#[cfg(feature = "futures")]
use futures::IntoFuture;
#[cfg(feature = "futures")]
use futures::executor::spawn;

use channel::oneshot::*;

fn assert_sync<T: Sync>() {}
fn assert_send<T: Send>() {}

#[test]
fn assertions() {
    assert_sync::<Sender<u8>>();
    assert_send::<Sender<u8>>();
    assert_sync::<Receiver<u8>>();
    assert_send::<Receiver<u8>>();
    #[cfg(feature = "futures")]
    assert_sync::<ReceiveFuture<u8>>();
    #[cfg(feature = "futures")]
    assert_send::<ReceiveFuture<u8>>();
}

#[test]
fn simple() {
    let value = "Hello world".to_owned();
    let (sender, receiver) = channel();
    let receiver = match receiver.try_receive() {
        Err(ReceiveError::Empty(receiver)) => receiver,
        _ => panic!("expected receiver empty"),
    };
    assert!(sender.send(value.clone()).is_ok(), "expected the send to be ok");
    assert_eq!(receiver.try_receive().unwrap(), value);
}

#[test]
fn from_another_thread() {
    let value = "Hello world".to_owned();
    let value_c = value.clone();
    let (sender, receiver) = channel();
    thread::spawn(move || {
        assert!(sender.send(value_c).is_ok(), "expected the send to be ok");
    });
    thread::sleep(Duration::from_millis(10));
    assert_eq!(receiver.try_receive().unwrap(), value);
}

#[test]
fn closed_sender() {
    let (sender, receiver) = channel::<u8>();
    mem::drop(sender);
    match receiver.try_receive() {
        Err(ReceiveError::Disconnected) => (),
        _ => panic!("expected sender disconnected error"),
    }
}

#[test]
fn closed_receiver() {
    let value = "Hello world".to_owned();
    let (sender, receiver) = channel();
    mem::drop(receiver);
    match sender.send(value.clone()) {
        Err(SendError(got)) => assert_eq!(got, value),
        _ => panic!("expected receiver disconnected error"),
    }
}

#[test]
fn drop_test_nothing_send() {
    // Shouldn't panic or segfault.
    let (sender, receiver) = channel::<String>();
    mem::drop(sender);
    mem::drop(receiver);
}

#[test]
fn drop_test_receiver_send() {
    let (sender, receiver) = channel();
    let (value_send, value) = DropTest::new();
    assert_eq!(value.load(Ordering::SeqCst), 0);
    assert!(sender.send(value_send).is_ok(), "expected send to be ok");
    mem::drop(receiver);
    // The value should be dropped.
    assert_eq!(value.load(Ordering::SeqCst), 1);
}

struct DropTest { value: Arc<AtomicUsize> }
impl DropTest {
    fn new() -> (DropTest, Arc<AtomicUsize>) {
        let value = Arc::new(AtomicUsize::new(0));
        (DropTest { value: value.clone() }, value)
    }
}
impl Drop for DropTest {
    fn drop(&mut self) {
        self.value.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
#[cfg(feature = "futures")]
fn simple_future() {
    let value = "Hello world".to_owned();
    let value_c = value.clone();
    let (sender, receiver) = channel();
    thread::spawn(move || {
        // Make sure the first try of the executor returns `Async::NotReady`,
        // this way we also test if the receiving side gets woken up.
        thread::sleep(Duration::from_millis(10));
        assert!(sender.send(value_c).is_ok());
    });
    // First we should receive `Async::NotReady`, then we should get a signal
    // once the value has been send.
    let receive_future = receiver.into_future();
    assert_eq!(spawn(receive_future).wait_future(), Ok(value));
}

#[test]
#[cfg(feature = "futures")]
fn future_dropped_receiver() {
    let (sender, receiver) = channel::<u8>();
    thread::spawn(move || {
        // Make sure the first try of the executor returns `Async::NotReady`,
        // this way we also test if the receiving side gets woken up.
        thread::sleep(Duration::from_millis(10));
        mem::drop(sender);
    });
    // First we should receive `Async::NotReady`, then we should get a signal
    // once the sender has been dropped.
    let receive_future = receiver.into_future();
    assert_eq!(spawn(receive_future).wait_future(), Err(SenderDisconnectedError));
}

#[test]
#[should_panic(expected = "tried to poll ReceiveFuture after completion")]
#[cfg(feature = "futures")]
fn double_poll_should_panic() {
    let (sender, receiver) = channel::<u8>();
    mem::drop(sender);
    let receive_future = receiver.into_future();
    let mut executor = spawn(receive_future);
    assert_eq!(executor.wait_future(), Err(SenderDisconnectedError));
    // Poll after completion, should panic.
    assert!(executor.wait_future().is_err());
}
