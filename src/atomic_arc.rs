// Copyright 2017 Thomas de Zeeuw
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT
// or http://opensource.org/licenses/MIT>, at your option. This file may not be
// used, copied, modified, or distributed except according to those terms.

use std::{mem, ptr};
use std::sync::Arc;
use std::sync::atomic::{AtomicPtr, Ordering};

/// `AtomicArc` that can only be set once and then read atomically multiple
/// times, by multiple threads.
#[derive(Debug)]
pub struct AtomicArc<T> {
    /// Pointer created by `Arc::into_raw`.
    ptr: AtomicPtr<T>,
}

impl<T> AtomicArc<T> {
    /// Create a new empty `AtomicArc`.
    pub fn empty() -> AtomicArc<T> {
        AtomicArc {
            ptr: AtomicPtr::new(ptr::null_mut()),
        }
    }

    /// Set the `AtomicArc` to the provided `arc`. If it's not empty it will
    /// return an error that returns the provided `arc`.
    pub fn set(&self, arc: Arc<T>) -> Result<(), Arc<T>> {
        let ptr = Arc::into_raw(arc);
        match self.ptr.compare_exchange(ptr::null_mut(), ptr as *mut T,
            Ordering::SeqCst, Ordering::Relaxed)
        {
            Ok(_) => Ok(()),
            Err(_) => Err(unsafe { Arc::from_raw(ptr) }),
        }
    }

    /// Get a copy of the current `Arc`.
    pub fn get(&self) -> Option<Arc<T>> {
        match self.ptr.load(Ordering::Acquire) {
            ptr if ptr.is_null() => None,
            ptr => {
                let arc: Arc<T> = unsafe { Arc::from_raw(ptr) };
                let c = Arc::clone(&arc);
                mem::forget(arc);
                Some(c)
            },
        }
    }
}

impl<T> Drop for AtomicArc<T> {
    fn drop(&mut self) {
        let ptr = self.ptr.load(Ordering::Relaxed);
        if !ptr.is_null() {
            mem::drop(unsafe { Arc::from_raw(ptr) });
        }
    }
}
