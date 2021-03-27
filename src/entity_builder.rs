// Copyright 2019 Google LLC
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use crate::alloc::alloc::{alloc, dealloc, Layout};
use crate::alloc::vec::Vec;
use core::any::TypeId;
use core::ptr::{self, NonNull};

use hashbrown::hash_map::Entry;

use crate::archetype::{TypeIdMap, TypeInfo};
use crate::{align, ArchetypeSet, Component, DynamicBundle};

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
    storage: NonNull<u8>,
    layout: Layout,
    cursor: usize,
    info: Vec<(TypeInfo, usize)>,
    ids: Vec<TypeId>,
    indices: TypeIdMap<usize>,
}

impl EntityBuilder {
    /// Create a builder representing an entity with no components
    pub fn new() -> Self {
        Self {
            storage: NonNull::dangling(),
            layout: Layout::from_size_align(0, 8).unwrap(),
            cursor: 0,
            info: Vec::new(),
            ids: Vec::new(),
            indices: Default::default(),
        }
    }

    /// Add `component` to the entity.
    ///
    /// If the bundle already contains a component of type `T`, it will
    /// be dropped and replaced with the most recently added one.
    pub fn add<T: Component>(&mut self, component: T) -> &mut Self {
        self.add_bundle((component,))
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
                        let storage = self.storage.as_ptr().add(offset);

                        // Drop the existing value
                        ty.drop(storage);

                        // Overwrite the old value with our new one.
                        ptr::copy_nonoverlapping(ptr, storage, ty.layout().size());
                    }
                    Entry::Vacant(vacant) => {
                        let offset = align(self.cursor, ty.layout().align());
                        let end = offset + ty.layout().size();
                        if end > self.layout.size() || ty.layout().align() > self.layout.align() {
                            let new_align = self.layout.align().max(ty.layout().align());
                            let (new_storage, new_layout) =
                                Self::grow(end, self.cursor, new_align, self.storage);
                            if self.layout.size() != 0 {
                                dealloc(self.storage.as_ptr(), self.layout);
                            }
                            self.storage = new_storage;
                            self.layout = new_layout;
                        }

                        let addr = self.storage.as_ptr().add(offset);
                        ptr::copy_nonoverlapping(ptr, addr, ty.layout().size());

                        vacant.insert(self.info.len());
                        self.info.push((ty, offset));
                        self.cursor = end;
                    }
                }
            });
        }
        self
    }

    /// Checks to see if the component of type `T` exists
    pub fn has<T: Component>(&self) -> bool {
        self.indices.contains_key(&TypeId::of::<T>())
    }

    /// Borrow the component of type `T`, if it exists
    pub fn get<T: Component>(&self) -> Option<&T> {
        let index = self.indices.get(&TypeId::of::<T>())?;
        let (_, offset) = self.info[*index];
        unsafe {
            let storage = self.storage.as_ptr().add(offset).cast::<T>();
            Some(&*storage)
        }
    }

    /// Uniquely borrow the component of type `T`, if it exists
    pub fn get_mut<T: Component>(&mut self) -> Option<&mut T> {
        let index = self.indices.get(&TypeId::of::<T>())?;
        let (_, offset) = self.info[*index];
        unsafe {
            let storage = self.storage.as_ptr().add(offset).cast::<T>();
            Some(&mut *storage)
        }
    }

    /// Enumerate the types of the entity builder's components
    pub fn component_types(&self) -> impl Iterator<Item = TypeId> + '_ {
        self.info.iter().map(|(info, _)| info.id())
    }

    unsafe fn grow(
        min_size: usize,
        cursor: usize,
        align: usize,
        storage: NonNull<u8>,
    ) -> (NonNull<u8>, Layout) {
        let layout = Layout::from_size_align(min_size.next_power_of_two().max(64), align).unwrap();
        let new_storage = NonNull::new_unchecked(alloc(layout));
        ptr::copy_nonoverlapping(storage.as_ptr(), new_storage.as_ptr(), cursor);
        (new_storage, layout)
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
        unsafe {
            for (ty, offset) in self.info.drain(..) {
                ty.drop(self.storage.as_ptr().add(offset));
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
        if self.layout.size() != 0 {
            unsafe {
                dealloc(self.storage.as_ptr(), self.layout);
            }
        }
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

unsafe impl DynamicBundle for BuiltEntity<'_> {
    fn find_archetype(&self, archetypes: &mut ArchetypeSet) -> u32 {
        archetypes.get(&self.builder.ids, &|| {
            self.builder.info.iter().map(|x| x.0).collect()
        })
    }

    #[doc(hidden)]
    fn type_info(&self) -> Vec<TypeInfo> {
        self.builder.info.iter().map(|x| x.0).collect()
    }

    unsafe fn put(self, mut f: impl FnMut(*mut u8, TypeInfo)) {
        for (ty, offset) in self.builder.info.drain(..) {
            let ptr = self.builder.storage.as_ptr().add(offset);
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
