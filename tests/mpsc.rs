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

use channel::mpsc::*;

// Keep in sync with the one in src/mpsc/segment.rs.
const SEGMENT_SIZE: usize = 32;

fn assert_sync<T: Sync>() {}
fn assert_send<T: Send>() {}
fn assert_clone<T: Clone>() {}

#[test]
fn assertions() {
    assert_sync::<Sender<u8>>();
    assert_send::<Sender<u8>>();
    assert_clone::<Sender<u8>>();
    assert_sync::<Receiver<u8>>();
    assert_send::<Receiver<u8>>();
}

#[test]
fn simple() {
    let value1 = "Hello world".to_owned();
    let (mut sender, mut receiver) = channel();
    assert!(sender.send(value1.clone()).is_ok(), "expected send ok");
    assert_eq!(receiver.try_receive().unwrap(), value1);
}

#[test]
fn send_alot() {
    const NUM_OPS: usize = 10 * SEGMENT_SIZE;
    let (mut sender, mut receiver) = channel();
    for n in 0..NUM_OPS {
        assert!(sender.send(format!("value_{}", n)).is_ok(), "expected send ok");
    }
    for n in 0..NUM_OPS {
        assert_eq!(receiver.try_receive().unwrap(), format!("value_{}", n));
    }
}

#[test]
fn closed_sender() {
    let (sender, mut receiver) = channel::<String>();
    assert_eq!(receiver.try_receive(), Err(ReceiveError::Empty));
    mem::drop(sender);
    assert_eq!(receiver.try_receive(), Err(ReceiveError::Disconnected));
}

#[test]
fn closed_receiver() {
    let (mut sender, receiver) = channel();
    assert!(sender.send(123).is_ok(), "expected send ok");
    mem::drop(receiver);
    assert_eq!(sender.send(456), Err(SendError(456)));
}

#[test]
fn drop_test_drop_receiver_first() {
    let (mut sender, receiver) = channel();
    let (value_send, value) = DropTest::new();
    assert_eq!(value.load(Ordering::SeqCst), 0);
    assert!(sender.send(value_send).is_ok(), "expected send to be ok");
    // Still active.
    assert_eq!(value.load(Ordering::SeqCst), 0);
    mem::drop(receiver);
    // Still active.
    assert_eq!(value.load(Ordering::SeqCst), 0);
    mem::drop(sender);
    // The value should be dropped now.
    assert_eq!(value.load(Ordering::SeqCst), 1);
}

#[test]
fn drop_test_drop_sender_first() {
    let (mut sender, receiver) = channel();
    let (value_send, value) = DropTest::new();
    assert_eq!(value.load(Ordering::SeqCst), 0);
    assert!(sender.send(value_send).is_ok(), "expected send to be ok");
    // Still active.
    assert_eq!(value.load(Ordering::SeqCst), 0);
    mem::drop(sender);
    // Still active.
    assert_eq!(value.load(Ordering::SeqCst), 0);
    mem::drop(receiver);
    // The value should be dropped now.
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

const NUM_THREADS: usize = 16;
// TODO: increase to 1_000_000, currently overflows its stack.
const NUM_MESSAGES: usize = 10_000;

#[test]
fn stress_test() {
    let (sender, mut receiver) = channel();
    for n in 0..NUM_THREADS {
        let mut sender = sender.clone();
        thread::Builder::new()
            .name(format!("multi_threaded_{}", n))
            .spawn(move || for m in 0..NUM_MESSAGES {
                if m % 1000 == 0 { thread::sleep(Duration::from_millis(1)); }
                assert_eq!(sender.send(format!("value{}_{}", n, m)), Ok(()));
            }).unwrap();
    }

    thread::sleep(Duration::from_millis(10));
    receive_values(NUM_THREADS * NUM_MESSAGES, &mut receiver);

    assert_eq!(receiver.try_receive(), Err(ReceiveError::Empty));
    mem::drop(sender);
    assert_eq!(receiver.try_receive(), Err(ReceiveError::Disconnected));
}

const MAX_TRIES: usize = 10;

fn receive_values(num_values: usize, receiver: &mut Receiver<String>) {
    for n in 0..num_values {
        receive_one(n, MAX_TRIES, receiver);
    }
}

fn receive_one(num: usize, tries_left: usize, receiver: &mut Receiver<String>) {
    if tries_left == 0 {
        panic!("giving up on retrieving a value");
    }

    match receiver.try_receive() {
        Ok(_) => return,
        Err(ReceiveError::Empty) => {
            thread::sleep(Duration::from_millis(1));
            receive_one(num, tries_left - 1, receiver);
        },
        Err(ReceiveError::Disconnected) => panic!("the sender is disconnected"),
    }
}

#[test]
#[cfg(feature = "futures")]
fn futures() {
    use futures::{Async, AsyncSink, Stream, Sink};

    let value1 = "Hello world";
    let value1_c = value1.clone();
    let value2 = "Hey earth";
    let value2_c = value2.clone();

    let (mut sender, receiver) = channel();

    let handle = thread::spawn(move || {
        thread::sleep(Duration::from_millis(5));
        assert_eq!(sender.start_send(value1_c), Ok(AsyncSink::Ready));
        thread::sleep(Duration::from_millis(5));
        assert_eq!(sender.start_send(value2_c), Ok(AsyncSink::Ready));
        assert_eq!(sender.poll_complete(), Ok(Async::Ready(())));
    });

    // This should be woken up by the sender for both values. To make sure of
    // that we'll wait before sending any values.
    for (n, value) in receiver.wait().enumerate() {
        match n {
            0 => assert_eq!(value, Ok(value1)),
            1 => assert_eq!(value, Ok(value2)),
            _ => panic!("the receiver wasn't notified that the sender was closed"),
        }
    }
    handle.join().unwrap();
}
