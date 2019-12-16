// Copyright 2019 Google LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use core::any::{type_name, TypeId};
use core::ops::{Deref, DerefMut};
use core::ptr::NonNull;
use core::sync::atomic::{AtomicUsize, Ordering};

use hashbrown::HashMap;

use crate::archetype::Archetype;
use crate::world::Component;

/// Tracks which components of a world are borrowed in what ways
#[derive(Default)]
pub struct BorrowState {
    states: HashMap<TypeId, AtomicBorrow>,
}

impl BorrowState {
    pub(crate) fn ensure(&mut self, ty: TypeId) {
        self.states.entry(ty).or_insert_with(AtomicBorrow::new);
    }

    /// Acquire a shared borrow
    pub fn borrow(&self, ty: TypeId, name: &str) {
        if self.states.get(&ty).map_or(false, |x| !x.borrow()) {
            panic!("{} already borrowed uniquely", name);
        }
    }

    /// Acquire a unique borrow
    pub fn borrow_mut(&self, ty: TypeId, name: &str) {
        if self.states.get(&ty).map_or(false, |x| !x.borrow_mut()) {
            panic!("{} already borrowed", name);
        }
    }

    /// Release a shared borrow
    pub fn release(&self, ty: TypeId) {
        if let Some(x) = self.states.get(&ty) {
            x.release();
        }
    }

    /// Release a unique borrow
    pub fn release_mut(&self, ty: TypeId) {
        if let Some(x) = self.states.get(&ty) {
            x.release_mut();
        }
    }
}

struct AtomicBorrow(AtomicUsize);

impl AtomicBorrow {
    const fn new() -> Self {
        Self(AtomicUsize::new(0))
    }

    fn borrow(&self) -> bool {
        let value = self.0.fetch_add(1, Ordering::Acquire).wrapping_add(1);
        if value == 0 {
            // Wrapped, this borrow is invalid!
            core::panic!()
        }
        if value & UNIQUE_BIT != 0 {
            self.0.fetch_sub(1, Ordering::Release);
            false
        } else {
            true
        }
    }

    fn borrow_mut(&self) -> bool {
        self.0
            .compare_exchange(0, UNIQUE_BIT, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }

    fn release(&self) {
        let value = self.0.fetch_sub(1, Ordering::Release);
        debug_assert!(value & UNIQUE_BIT == 0);
    }

    fn release_mut(&self) {
        self.0.store(0, Ordering::Release);
    }
}

const UNIQUE_BIT: usize = !(usize::max_value() >> 1);

/// Shared borrow of an entity's component
#[derive(Clone)]
pub struct Ref<'a, T: Component> {
    borrow: &'a BorrowState,
    target: NonNull<T>,
}

impl<'a, T: Component> Ref<'a, T> {
    pub(crate) unsafe fn new(borrow: &'a BorrowState, target: NonNull<T>) -> Self {
        borrow.borrow(TypeId::of::<T>(), type_name::<T>());
        Self { borrow, target }
    }
}

impl<'a, T: Component> Drop for Ref<'a, T> {
    fn drop(&mut self) {
        self.borrow.release(TypeId::of::<T>());
    }
}

impl<'a, T: Component> Deref for Ref<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { self.target.as_ref() }
    }
}

/// Unique borrow of an entity's component
pub struct RefMut<'a, T: Component> {
    borrow: &'a BorrowState,
    target: NonNull<T>,
}

impl<'a, T: Component> RefMut<'a, T> {
    pub(crate) fn new(borrow: &'a BorrowState, target: NonNull<T>) -> Self {
        borrow.borrow_mut(TypeId::of::<T>(), type_name::<T>());
        Self { borrow, target }
    }
}

impl<'a, T: Component> Drop for RefMut<'a, T> {
    fn drop(&mut self) {
        self.borrow.release_mut(TypeId::of::<T>());
    }
}

impl<'a, T: Component> Deref for RefMut<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { self.target.as_ref() }
    }
}

impl<'a, T: Component> DerefMut for RefMut<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { self.target.as_mut() }
    }
}

/// Handle to an entity with any component types
#[derive(Copy, Clone)]
pub struct EntityRef<'a> {
    borrow: &'a BorrowState,
    archetype: &'a Archetype,
    index: u32,
}

impl<'a> EntityRef<'a> {
    pub(crate) fn new(borrow: &'a BorrowState, archetype: &'a Archetype, index: u32) -> Self {
        Self {
            borrow,
            archetype,
            index,
        }
    }

    /// Borrow the component of type `T`, if it exists
    ///
    /// Panics if a component of type `T` is already uniquely borrowed from the world
    pub fn get<T: Component>(&self) -> Option<Ref<'a, T>> {
        Some(unsafe { Ref::new(self.borrow, self.archetype.get(self.index)?) })
    }

    /// Uniquely borrow the component of type `T`, if it exists
    ///
    /// Panics if a component of type `T` is already borrowed from the world
    pub fn get_mut<T: Component>(&self) -> Option<RefMut<'a, T>> {
        Some(unsafe { RefMut::new(self.borrow, self.archetype.get(self.index)?) })
    }
}
