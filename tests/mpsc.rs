// Copyright 2017 Thomas de Zeeuw
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT
// or http://opensource.org/licenses/MIT>, at your option. This file may not be
// used, copied, modified, or distributed except according to those terms.

extern crate tchannel;
#[cfg(feature = "futures")]
extern crate futures;

use std::{mem, thread};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tchannel::mpsc::*;

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
    assert_eq!(sender.send(value1.clone()), Ok(()));
    assert_eq!(receiver.try_receive().unwrap(), value1);
}

#[test]
fn send_alot() {
    const NUM_OPS: usize = 10 * SEGMENT_SIZE;
    let (mut sender, mut receiver) = channel();
    for n in 0..NUM_OPS {
        assert_eq!(sender.send(format!("value_{}", n)), Ok(()));
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
    assert_eq!(sender.send(123), Ok(()));
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
        (DropTest { value: Arc::clone(&value) }, value)
    }
}
impl Drop for DropTest {
    fn drop(&mut self) {
        self.value.fetch_add(1, Ordering::SeqCst);
    }
}

const NUM_THREADS: usize = 8;
const NUM_MESSAGES: usize = 1_000_000;

#[test]
#[ignore]
fn stress_test() {
    let (sender, receiver) = channel();

    let mut handles = Vec::with_capacity(NUM_THREADS + 1);
    for n in 0..NUM_THREADS {
        let mut sender = sender.clone();
        let handle = thread::Builder::new()
            .name(format!("stress_test_send{}", n))
            .spawn(move || for m in 0..NUM_MESSAGES {
                if m % 500 == 0 { thread::yield_now(); }
                assert_eq!(sender.send(format!("value{}_{}", n, m)), Ok(()));
            }).unwrap();
        handles.push(handle);
    }

    let handle = thread::Builder::new()
        .name("stress_test_recv".to_owned())
        .spawn(move || {
            receive_values(NUM_THREADS * NUM_MESSAGES, receiver, sender);
        }).unwrap();
    handles.push(handle);

    for handle in handles {
        handle.join().expect("error in stress test thread");
    }
}

fn receive_values(num_values: usize, mut receiver: Receiver<String>, sender: Sender<String>) {
    for _ in 0..num_values {
        receive_one(&mut receiver);
    }

    assert_eq!(receiver.try_receive(), Err(ReceiveError::Empty));
    mem::drop(sender);
    assert_eq!(receiver.try_receive(), Err(ReceiveError::Disconnected));
}

const MAX_TRIES: usize = 10;

fn receive_one(receiver: &mut Receiver<String>) {
    for _ in 0..MAX_TRIES {
        match receiver.try_receive() {
            Ok(_) => return,
            Err(ReceiveError::Empty) => {
                thread::yield_now();
                continue;
            },
            Err(ReceiveError::Disconnected) => panic!("the sender is disconnected"),
        }
    }

    panic!("giving up on retrieving a value");
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
        thread::sleep(Duration::from_millis(100));
        assert_eq!(sender.start_send(value1_c), Ok(AsyncSink::Ready));
        thread::sleep(Duration::from_millis(100));
        assert_eq!(sender.start_send(value2_c), Ok(AsyncSink::Ready));
        thread::sleep(Duration::from_millis(100));
        assert_eq!(sender.poll_complete(), Ok(Async::Ready(())));
    });

    // This should be woken up by the sender for both values.
    let mut stream = receiver.wait();
    assert_eq!(stream.next(), Some(Ok(value1)));
    assert_eq!(stream.next(), Some(Ok(value2)));
    assert_eq!(stream.next(), None);
    handle.join().unwrap();
}

#[test]
#[cfg(feature = "futures")]
fn futures_regular_send() {
    use futures::Stream;

    let value1 = "Hello world";
    let value1_c = value1.clone();

    let (mut sender, receiver) = channel();

    let handle = thread::spawn(move || {
        thread::sleep(Duration::from_millis(100));
        // Regular send should also notify the receiver.
        assert_eq!(sender.send(value1_c), Ok(()), "expected send ok");
    });

    // This should be woken up by the sender for both values.
    let mut stream = receiver.wait();
    assert_eq!(stream.next(), Some(Ok(value1)));
    assert_eq!(stream.next(), None);
    handle.join().unwrap();
}
