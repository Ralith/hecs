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
use crate::{align, Component, DynamicBundle};

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
    cursor: usize,
    info: Vec<(TypeInfo, usize)>,
    ids: Vec<TypeId>,
}

impl EntityBuilder {
    /// Create a builder representing an entity with no components
    pub fn new() -> Self {
        Self {
            cursor: 0,
            storage: Box::new([]),
            info: Vec::new(),
            ids: Vec::new(),
        }
    }

    /// Add `component` to the entity
    pub fn add<T: Component>(&mut self, component: T) -> &mut Self {
        let aligned = self.next_cursor(mem::align_of::<T>());
        if aligned + mem::size_of::<T>() > self.storage.len() {
            self.grow(mem::size_of::<T>(), mem::align_of::<T>());
            self.cursor = self.next_cursor(mem::align_of::<T>());
        } else {
            self.cursor = aligned;
        }
        if mem::size_of::<T>() != 0 {
            unsafe {
                self.storage[self.cursor]
                    .as_mut_ptr()
                    .cast::<T>()
                    .write(component);
            }
        }
        self.info.push((TypeInfo::of::<T>(), self.cursor));
        self.cursor += mem::size_of::<T>();
        self
    }

    fn next_cursor(&self, alignment: usize) -> usize {
        align(self.storage.as_ptr() as usize + self.cursor, alignment)
            - self.storage.as_ptr() as usize
    }

    fn grow(&mut self, increment_size: usize, increment_align: usize) {
        self.info.sort_unstable_by(|x, y| x.0.cmp(&y.0));
        let new_len = (self.storage.len() + increment_size)
            .next_power_of_two()
            .max(64);
        let old_storage = mem::replace(&mut self.storage, unsafe {
            Box::from_raw(std::slice::from_raw_parts_mut(
                alloc(
                    Layout::from_size_align(
                        new_len,
                        self.info.first().map_or(increment_align, |x| {
                            x.0.layout().align().max(increment_align)
                        }),
                    )
                    .unwrap(),
                )
                .cast(),
                new_len,
            ))
        });
        let components = self.info.len();
        let old_info = mem::replace(&mut self.info, Vec::with_capacity(components));
        self.cursor = 0;
        for (info, offset) in old_info {
            let new_offset = self.next_cursor(info.layout().align());
            self.cursor = new_offset + info.layout().size();
            self.storage[new_offset..self.cursor]
                .copy_from_slice(&old_storage[offset..offset + info.layout().size()]);
            self.info.push((info, new_offset));
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
        self.cursor = 0;
        unsafe {
            for (ty, component) in self.info.drain(..) {
                ty.drop(self.storage[component].as_mut_ptr().cast::<u8>());
            }
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
            let ptr = self.builder.storage[component].as_mut_ptr().cast();
            if !f(ptr, ty.id(), ty.layout().size()) {
                ty.drop(ptr);
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
