// Copyright 2017 Thomas de Zeeuw
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT
// or http://opensource.org/licenses/MIT>, at your option. This file may not be
// used, copied, modified, or distributed except according to those terms.

use std::{mem, ptr};
use std::sync::Arc;
use std::sync::atomic::{AtomicPtr, Ordering};

/// `AtomicArc` that can only be set once, by multiple threads, and then read
/// atomically multiple times, by multiple threads. And once a single thread has
/// access to it can it be `reset`.
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

    /// Set the `AtomicArc` to the provided `arc`. If this is empty it will
    /// succeed. If it's not empty however it will return an error that returns
    /// a copy of the current Arc (same as `get_ref`) and the provided `arc`.
    pub fn set(&self, arc: Arc<T>) -> Result<(), (Arc<T>, Arc<T>)> {
        let new_ptr = Arc::into_raw(arc);
        match self.ptr.compare_exchange(ptr::null_mut(), new_ptr as *mut T,
            Ordering::SeqCst, Ordering::Relaxed)
        {
            Ok(_) => Ok(()),
            Err(current_ptr) => {
                let current = unsafe { copy_arc(current_ptr) };
                let new = unsafe { Arc::from_raw(new_ptr) };
                Err((current, new))
            },
        }
    }

    /// Get a copy of the current `Arc`.
    pub fn get_ref(&self) -> Option<Arc<T>> {
        match self.ptr.load(Ordering::Acquire) {
            ptr if ptr.is_null() => None,
            ptr => unsafe { Some(copy_arc(ptr)) },
        }
    }

    /// Reset the `AtomicArc` for reuse. This requires mutable access to make
    /// sure only a single thread has access to it.
    pub fn reset(&mut self) -> Option<Arc<T>> {
        match self.ptr.swap(ptr::null_mut(), Ordering::Acquire) {
            ptr if ptr.is_null() => None,
            ptr => Some(unsafe { Arc::from_raw(ptr) }),
        }
    }
}

/// Create a copy of an raw Arc, without deallocating the Arc of the pointer.
///
/// # Safety
/// The pointer must be created by `Arc::into_raw`, and may not be null.
unsafe fn copy_arc<T>(ptr: *mut T) -> Arc<T> {
    debug_assert!(!ptr.is_null(), "null pointer passed to copy_arc");
    let arc: Arc<T> = Arc::from_raw(ptr);
    let copy = Arc::clone(&arc);
    mem::forget(arc); // Don't dealloce the `Arc` at the pointer.
    copy
}

impl<T> Drop for AtomicArc<T> {
    fn drop(&mut self) {
        let ptr = self.ptr.load(Ordering::Acquire);
        if !ptr.is_null() {
            mem::drop(unsafe { Arc::from_raw(ptr) });
        }
    }
}
