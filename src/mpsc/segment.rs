// Copyright 2017 Thomas de Zeeuw
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT
// or http://opensource.org/licenses/MIT>, at your option. This file may not be
// used, copied, modified, or distributed except according to those terms.

// TODO: update docs and comments.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::super::atomic_arc::AtomicArc;
use super::super::atomic_cell::AtomicCell;

/// The number of items in a single [`Segment`]. 32 is chosen somewhat
/// arbitrarily.
///
/// [`Segment`]: struct.Segment.html
pub const SEGMENT_SIZE: usize = 32;

/// The result from trying to append to a [`Segment`], see [`append`].
///
/// [`Segment`]: struct.Segment.html
/// [`append`]: struct.Segment.html#method.append
#[derive(Debug)]
pub enum Expanded<T> {
    /// The segment was not expanded.
    No,
    /// The segment was expanded and a reference to it is provided.
    Expanded(Arc<Segment<T>>),
}

/// `Segment` is an array that can hold [`n`] number of items `T`. The write
/// operations will be deligated to a next `Segment`, if any.
///
/// # Safety
///
/// The `Segment` may have multiple writers, but only a single reader. Hence the
/// `Segment` will keep track of the write index, but not the read index (that
/// is up to the user).
///
/// [`n`]: constant.SEGMENT_SIZE.html
#[derive(Debug)]
pub struct Segment<T> {
    /// The data this segment is responible for.
    // TODO: benchmark using `crossbeam::CachePadded<AtomicCell<T>>`.
    data: [AtomicCell<T>; SEGMENT_SIZE],
    /// The current writing index, if this is bigger then `SEGMENT_SIZE` it
    /// means the `Segment` is full, see `get_write_index`.
    write_index: AtomicUsize,
    /// A pointer to the next `Segment`, this will initially be empty. However
    /// after this is set it may **NOT** be changed, the only expection being
    /// `reset` (which must have lone access to `Segment`).
    next: AtomicArc<Segment<T>>,
}

impl<T> Segment<T> {
    /// Create new empty `Segment`.
    pub fn empty() -> Segment<T> {
        Segment {
            data: Default::default(),
            write_index: AtomicUsize::new(0),
            next: AtomicArc::empty(),
        }
    }

    /// Try to pop a value on the provided `index`. If this returns `None` it
    /// means that the `index` is current empty (or being written to).
    ///
    /// # Panic
    ///
    /// This will panic if `index` is bigger then `SEGMENT_SIZE` - 1.
    pub fn try_pop(&self, index: usize) -> Option<T> {
        unsafe {
            self.data[index].read()
        }
    }

    /// Append a value to the end of the `Segment`. The append will never fail,
    /// but it will return an enum that indicates wether the `Segment` was
    /// expanded.
    pub fn append(&self, value: T) -> Expanded<T> {
        match self.get_write_index() {
            Some(index) => {
                // Here we're the only writer to the index, but there may be
                // also be a reader. So we're upholding the contract.
                unsafe { self.data[index].write(value) }
                Expanded::No
            },
            None => {
                let next_segment = match self.next.get() {
                    Some(next_segment) => next_segment,
                    None => {
                        // The expand gaurentees the `next` field will be set.
                        self.expand();
                        self.next.get().unwrap()
                    },
                };
                // Make sure the next segment is full already, so we always
                // return the correct tail segment.
                let tail_segment = match next_segment.append(value) {
                    Expanded::No => next_segment,
                    Expanded::Expanded(tail_segment) => tail_segment,
                };
                Expanded::Expanded(tail_segment)
            },
        }
    }

    /// Get a unique write index in this `Segment`, if `None` is returned it
    /// means this `Segment` is full.
    fn get_write_index(&self) -> Option<usize> {
        match self.write_index.fetch_add(1, Ordering::SeqCst) {
            index if index >= SEGMENT_SIZE => None,
            index => Some(index),
        }
    }

    /// Expand this `Segment` with a new segment, see `expand_with_segment`. It
    /// guarantees that the `next` field will be set to something after this
    /// call.
    fn expand(&self) {
        let new_segment = Arc::new(Segment::empty());
        self.expand_with_segment(new_segment)
    }

    /// Expand this `Segment` with the `new_segment` provided. This will use the
    /// provided `new_segment` as the next `Segment` if this `Segment` doesn't
    /// have any. If however this `Segment` already has a next `Segment` it will
    /// be added to that `Segment` to not waste the allocation.
    pub fn expand_with_segment(&self, new_segment: Arc<Segment<T>>) {
        match self.next.set(new_segment) {
            Ok(()) => (),
            Err(new_segment) => {
                // We couldn't write to the `AtomicArc`, thus it means it's
                // already set.
                let next_segment = self.next.get().unwrap();
                next_segment.expand_with_segment(new_segment);
            }
        }
    }

    /// Get a refernce to the next `Segment` in the linked list, if any.
    pub fn next_segment(&self) -> Option<Arc<Segment<T>>> {
        self.next.get()
    }

    /// Reset the segment for reuse, it returns an `Arc` to the next `Segment`.
    ///
    /// # Note
    ///
    /// This doesn't check if all the items are empty!
    pub fn reset(&mut self) -> Option<Arc<Segment<T>>> {
        self.write_index.store(0, Ordering::Release);
        self.next.reset()
    }
}
