// Copyright 2019 Google LLC
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use crate::atomic::{AtomicUsize, Ordering};

/// A bit mask used to signal the `AtomicBorrow` has an active mutable borrow.
const UNIQUE_BIT: usize = !(usize::max_value() >> 1);

const COUNTER_MASK: usize = usize::max_value() >> 1;

/// An atomic integer used to dynamicaly enforce borrowing rules
///
/// The most significant bit is used to track mutable borrow, and the rest is a
/// counter for immutable borrows.
///
/// It has four possible states:
///  - `0b00000000...` the counter isn't mut borrowed, and ready for borrowing
///  - `0b0_______...` the counter isn't mut borrowed, and currently borrowed
///  - `0b10000000...` the counter is mut borrowed
///  - `0b1_______...` the counter is mut borrowed, and some other thread is trying to borrow
pub struct AtomicBorrow(AtomicUsize);

impl AtomicBorrow {
    pub const fn new() -> Self {
        Self(AtomicUsize::new(0))
    }

    pub fn borrow(&self) -> bool {
        // Add one to the borrow counter
        let prev_value = self.0.fetch_add(1, Ordering::Acquire);

        // If the previous counter had all of the immutable borrow bits set,
        // the immutable borrow counter overflowed.
        if prev_value & COUNTER_MASK == COUNTER_MASK {
            core::panic!("immutable borrow counter overflowed")
        }

        // If the mutable borrow bit is set, immutable borrow can't occur. Roll back.
        if prev_value & UNIQUE_BIT != 0 {
            self.0.fetch_sub(1, Ordering::Release);
            false
        } else {
            true
        }
    }

    pub fn borrow_mut(&self) -> bool {
        self.0
            .compare_exchange(0, UNIQUE_BIT, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }

    pub fn release(&self) {
        let value = self.0.fetch_sub(1, Ordering::Release);
        debug_assert!(value != 0, "unbalanced release");
        debug_assert!(value & UNIQUE_BIT == 0, "shared release of unique borrow");
    }

    pub fn release_mut(&self) {
        let value = self.0.fetch_and(!UNIQUE_BIT, Ordering::Release);
        debug_assert_ne!(value & UNIQUE_BIT, 0, "unique release of shared borrow");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "immutable borrow counter overflowed")]
    fn test_borrow_counter_overflow() {
        let counter = AtomicBorrow(AtomicUsize::new(COUNTER_MASK));
        counter.borrow();
    }

    #[test]
    #[should_panic(expected = "immutable borrow counter overflowed")]
    fn test_mut_borrow_counter_overflow() {
        let counter = AtomicBorrow(AtomicUsize::new(COUNTER_MASK | UNIQUE_BIT));
        counter.borrow();
    }

    #[test]
    fn test_borrow() {
        let counter = AtomicBorrow::new();
        assert!(counter.borrow());
        assert!(counter.borrow());
        assert!(!counter.borrow_mut());
        counter.release();
        counter.release();

        assert!(counter.borrow_mut());
        assert!(!counter.borrow());
        counter.release_mut();
        assert!(counter.borrow());
    }
}
