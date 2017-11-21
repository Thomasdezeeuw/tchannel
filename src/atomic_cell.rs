// Copyright 2017 Thomas de Zeeuw
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT
// or http://opensource.org/licenses/MIT>, at your option. This file may not be
// used, copied, modified, or distributed except according to those terms.

use std::mem::{self, ManuallyDrop};
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::cell::UnsafeCell;

/// This is an `AtomicCell` used for a single reader and single writer.
///
/// # Unsafety
///
/// This struct is very unsafe in it's usage, it has the following contracts
/// that have to be up hold by the user(s):
///
/// 1. Only a single thread may write to this value, **once**.
/// 2. The value may only be written to when empty.
///
/// Any breaking of the above defined contract will result in undefined
/// behaviour.
#[derive(Debug)]
pub struct AtomicCell<T> {
    /// The state of the value, see the `STATE_*` constants in the
    /// implementation.
    state: AtomicUsize,
    // TODO: use Crossbeam's `CachePadded` for `data` field, benchmark various T
    // types, also inside `Segment`.
    data: ManuallyDrop<UnsafeCell<T>>,
}

const STATE_EMPTY: usize = 0;
const STATE_FULL:  usize = 1;

impl<T> AtomicCell<T> {
    /// Create a new, empty `AtomicCell`.
    pub fn empty() -> AtomicCell<T> {
        let empty_data = unsafe { mem::uninitialized() };
        AtomicCell {
            state: AtomicUsize::new(STATE_EMPTY),
            data: ManuallyDrop::new(UnsafeCell::new(empty_data)),
        }
    }

    /// Write a value.
    ///
    /// # Unsafety
    ///
    /// This function is unsafe because it's up to the caller to make sure it's
    /// the only thread writing to this value. If it's not the casethis will
    /// result in undefined behaviour.
    ///
    /// Another cause of undefined behaviour is if the value is not empty. Only
    /// call this function if the value is empty.
    pub unsafe fn write(&self, value: T) {
        ptr::write(self.data.get(), value);
        self.state.store(STATE_FULL, Ordering::Release);
    }

    // TODO: doc, include safety stuff.
    pub unsafe fn read(&self) -> Option<T> {
        match self.state.compare_exchange(STATE_FULL, STATE_EMPTY,
            Ordering::Acquire, Ordering::Relaxed)
        {
            Ok(_) => Some(ptr::read(self.data.get())),
            Err(_) => None,
        }
    }
}

impl<T> Default for AtomicCell<T> {
    /// Same as calling `AtomicCell::empty`.
    fn default() -> AtomicCell<T> {
        AtomicCell::empty()
    }
}

impl<T> Drop for AtomicCell<T> {
    fn drop(&mut self) {
        if self.state.load(Ordering::Relaxed) == STATE_FULL {
            unsafe { ManuallyDrop::drop(&mut self.data); }
        }
    }
}

// TODO: review this.
unsafe impl<T: Send> Send for AtomicCell<T> {}
unsafe impl<T: Sync> Sync for AtomicCell<T> {}
