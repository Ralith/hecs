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

use std::alloc::{alloc, Layout};
use std::any::TypeId;
use std::mem::{self, MaybeUninit};

use crate::archetype::TypeInfo;
use crate::{Component, DynamicBundle};

/// Helper for incrementally constructing a bundle of components with dynamic component types
///
/// Prefer reusing the same builder over creating new ones repeatedly.
///
/// ```
/// # use hecs::*;
/// let mut world = World::new();
/// let mut builder = EntityBuilder::new();
/// builder.add(123).add("abc");
/// let e = world.spawn(builder.build()); // builder can now be reused
/// assert_eq!(*world.get::<i32>(e).unwrap(), 123);
/// assert_eq!(*world.get::<&str>(e).unwrap(), "abc");
/// ```
pub struct EntityBuilder {
    storage: Box<[MaybeUninit<u8>]>,
    // Backwards from the end!
    cursor: *mut u8,
    info: Vec<(TypeInfo, *mut u8)>,
    ids: Vec<TypeId>,
}

impl EntityBuilder {
    /// Create a builder representing an entity with no components
    pub fn new() -> Self {
        let mut storage: Box<[MaybeUninit<u8>]> = Box::new([]);
        Self {
            cursor: storage.as_mut_ptr().cast::<u8>(), // Not null!
            storage,
            info: Vec::new(),
            ids: Vec::new(),
        }
    }

    /// Add `component` to the entity
    pub fn add<T: Component>(&mut self, component: T) -> &mut Self {
        unsafe {
            if let Some(cursor) = (self.cursor as usize)
                .checked_sub(mem::size_of::<T>())
                .map(|x| (x & !(mem::align_of::<T>() - 1)) as *mut u8)
                .filter(|&x| x >= self.storage.as_mut_ptr().cast())
            {
                self.cursor = cursor;
            } else {
                self.grow(mem::size_of::<T>().max(mem::align_of::<T>()));
                self.cursor = (self.cursor.sub(mem::size_of::<T>()) as usize
                    & !(mem::align_of::<T>() - 1)) as *mut u8;
            }
            self.cursor.cast::<T>().write(component);
        }
        self.info.push((TypeInfo::of::<T>(), self.cursor));
        self
    }

    fn grow(&mut self, min_increment: usize) {
        let new_len = (self.storage.len() + min_increment)
            .next_power_of_two()
            .max(self.storage.len() * 2)
            .max(64);
        unsafe {
            let alloc =
                alloc(Layout::from_size_align(new_len, 16).unwrap()).cast::<MaybeUninit<u8>>();
            let mut new_storage = Box::from_raw(std::slice::from_raw_parts_mut(alloc, new_len));
            new_storage[new_len - self.storage.len()..].copy_from_slice(&self.storage);
            self.cursor = new_storage
                .as_mut_ptr()
                .add(
                    new_len - self.storage.len()
                        + (self.cursor as usize - self.storage.as_ptr() as usize),
                )
                .cast();
            self.storage = new_storage;
        }
    }

    /// Construct a `Bundle` suitable for spawning
    pub fn build(&mut self) -> BuiltEntity<'_> {
        self.info.sort_unstable_by(|x, y| x.0.cmp(&y.0));
        self.ids.extend(self.info.iter().map(|x| x.0.id()));
        BuiltEntity { builder: self }
    }

    /// Drop previously `add`ed components
    ///
    /// The builder is cleared implicitly when an entity is built, so this doesn't usually need to
    /// be called.
    pub fn clear(&mut self) {
        self.ids.clear();
        unsafe {
            for (ty, component) in self.info.drain(..) {
                ty.drop(component);
            }
            self.cursor = self.storage.as_mut_ptr().add(self.storage.len()).cast();
        }
    }
}

unsafe impl Send for EntityBuilder {}
unsafe impl Sync for EntityBuilder {}

impl Drop for EntityBuilder {
    fn drop(&mut self) {
        // Ensure buffered components aren't leaked
        self.clear();
    }
}

impl Default for EntityBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// The output of an `EntityBuilder`, suitable for passing to `World::spawn` or `World::insert`
pub struct BuiltEntity<'a> {
    builder: &'a mut EntityBuilder,
}

impl DynamicBundle for BuiltEntity<'_> {
    fn with_ids<T>(&self, f: impl FnOnce(&[TypeId]) -> T) -> T {
        f(&self.builder.ids)
    }

    #[doc(hidden)]
    fn type_info(&self) -> Vec<TypeInfo> {
        self.builder.info.iter().map(|x| x.0).collect()
    }

    unsafe fn put(self, mut f: impl FnMut(*mut u8, TypeId, usize) -> bool) {
        for (ty, component) in self.builder.info.drain(..) {
            if !f(component, ty.id(), ty.layout().size()) {
                ty.drop(component);
            }
        }
    }
}

impl Drop for BuiltEntity<'_> {
    fn drop(&mut self) {
        // Ensures components aren't leaked if `store` was never called, and prepares the builder
        // for reuse.
        self.builder.clear();
    }
}
