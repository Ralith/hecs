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

use crate::alloc::alloc::{alloc, dealloc, Layout};
use crate::alloc::boxed::Box;
use crate::alloc::{vec, vec::Vec};
use core::any::TypeId;
use core::mem::{self, MaybeUninit};
use core::ptr;

use hashbrown::hash_map::{Entry, HashMap};

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
    cursor: usize,
    info: Vec<(TypeInfo, usize)>,
    ids: Vec<TypeId>,
    indices: HashMap<TypeId, usize>,
}

impl EntityBuilder {
    /// Create a builder representing an entity with no components
    pub fn new() -> Self {
        Self {
            cursor: 0,
            storage: Box::new([]),
            info: Vec::new(),
            ids: Vec::new(),
            indices: HashMap::new(),
        }
    }

    /// Add `component` to the entity.
    ///
    /// If the bundle already contains a component of type `T`, it will
    /// be dropped and replaced with the most recently added one.
    pub fn add<T: Component>(&mut self, component: T) -> &mut Self {
        match self.indices.entry(TypeId::of::<T>()) {
            Entry::Occupied(occupied) => {
                let index = *occupied.get();
                let (_, offset) = self.info[index];
                unsafe {
                    let storage_ptr = self
                        .storage
                        .as_mut_ptr()
                        .cast::<u8>()
                        .add(offset)
                        .cast::<T>();

                    // Drop the old value.
                    let _ = storage_ptr.read_unaligned();
                    // Overwrite the old value with our new one.
                    storage_ptr.write_unaligned(component);
                }
                self
            }
            Entry::Vacant(vacant) => {
                let end = self.cursor + mem::size_of::<T>();
                if end > self.storage.len() {
                    Self::grow(end, self.cursor, &mut self.storage);
                }
                unsafe {
                    self.storage
                        .as_mut_ptr()
                        .add(self.cursor)
                        .cast::<T>()
                        .write_unaligned(component);
                }
                vacant.insert(self.info.len());
                self.info.push((TypeInfo::of::<T>(), self.cursor));
                self.cursor += mem::size_of::<T>();
                self
            }
        }
    }

    /// Add all components in `bundle` to the entity.
    ///
    /// If the bundle contains any component which matches the type of a component
    /// already in the `EntityBuilder`, the newly added component from the bundle
    /// will replace the old component and the old component will be dropped.
    pub fn add_bundle(&mut self, bundle: impl DynamicBundle) -> &mut Self {
        unsafe {
            bundle.put(|ptr, ty| {
                match self.indices.entry(ty.id()) {
                    Entry::Occupied(occupied) => {
                        let index = *occupied.get();
                        let (ty, offset) = self.info[index];

                        let storage_ptr = self.storage.as_mut_ptr().cast::<u8>().add(offset);
                        // alloc a properly aligned tmp buffer and copy in the old value
                        // so we can drop it safely
                        let tmp = alloc(ty.layout());
                        ptr::copy_nonoverlapping(storage_ptr, tmp, ty.layout().size());
                        ty.drop(tmp);
                        dealloc(tmp, ty.layout());
                        // Overwrite the old value with our new one.
                        ptr::copy_nonoverlapping(ptr, storage_ptr, ty.layout().size());
                    }
                    Entry::Vacant(vacant) => {
                        let end = self.cursor + ty.layout().size();
                        if end > self.storage.len() {
                            Self::grow(end, self.cursor, &mut self.storage);
                        }

                        ptr::copy_nonoverlapping(
                            ptr,
                            self.storage.as_mut_ptr().add(self.cursor).cast(),
                            ty.layout().size(),
                        );

                        vacant.insert(self.info.len());
                        self.info.push((ty, self.cursor));
                        self.cursor += ty.layout().size();
                    }
                }
            });
        }
        self
    }

    fn grow(min_size: usize, cursor: usize, storage: &mut Box<[MaybeUninit<u8>]>) {
        let new_len = min_size.next_power_of_two().max(64);
        let mut new_storage = vec![MaybeUninit::uninit(); new_len].into_boxed_slice();
        new_storage[..cursor].copy_from_slice(&storage[..cursor]);
        *storage = new_storage;
    }

    /// Construct a `Bundle` suitable for spawning
    pub fn build(&mut self) -> BuiltEntity<'_> {
        self.info.sort_unstable_by_key(|x| x.0);
        self.ids.extend(self.info.iter().map(|x| x.0.id()));
        BuiltEntity { builder: self }
    }

    /// Drop previously `add`ed components
    ///
    /// The builder is cleared implicitly when an entity is built, so this doesn't usually need to
    /// be called.
    pub fn clear(&mut self) {
        self.ids.clear();
        self.indices.clear();
        self.cursor = 0;
        let max_size = self
            .info
            .iter()
            .map(|x| x.0.layout().size())
            .max()
            .unwrap_or(0);
        let max_align = self
            .info
            .iter()
            .map(|x| x.0.layout().align())
            .max()
            .unwrap_or(0);
        unsafe {
            // Suitably aligned storage for drop
            let tmp = if max_size > 0 {
                alloc(Layout::from_size_align(max_size, max_align).unwrap()).cast()
            } else {
                max_align as *mut _
            };
            for (ty, offset) in self.info.drain(..) {
                ptr::copy_nonoverlapping(
                    self.storage[offset..offset + ty.layout().size()]
                        .as_ptr()
                        .cast(),
                    tmp,
                    ty.layout().size(),
                );
                ty.drop(tmp);
            }
            if max_size > 0 {
                dealloc(tmp, Layout::from_size_align(max_size, max_align).unwrap())
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

    unsafe fn put(self, mut f: impl FnMut(*mut u8, TypeInfo)) {
        for (ty, offset) in self.builder.info.drain(..) {
            let ptr = self.builder.storage.as_mut_ptr().add(offset).cast();
            f(ptr, ty);
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
