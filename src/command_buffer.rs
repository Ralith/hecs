// Copyright 2019 Google LLC
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use core::any::TypeId;
use core::ptr::{self, NonNull};

use crate::alloc::alloc::{alloc, dealloc, Layout};
use crate::alloc::vec::Vec;
use crate::archetype::TypeInfo;
use crate::Entity;
use crate::World;
use crate::{align, DynamicBundle};

/// Allows spawn operations to be buffered for future application to a ['World']
///
/// ```
/// # use hecs::*;
/// let mut world = World::new();
/// let reserved_ent = world.reserve_entity();
/// let mut entity_buffer = CommandBuffer::new();
/// entity_buffer.spawn_at(reserved_ent,(true,0.10));
/// entity_buffer.run_on(&mut world); // buffer can now be reused
/// assert!(world.contains(reserved_ent));
/// ```
pub struct CommandBuffer {
    entities: Vec<EntityIndex>,
    storage: NonNull<u8>,
    layout: Layout,
    cursor: usize,
    info: Vec<ComponentInfo>,
    ids: Vec<TypeId>,
}

impl CommandBuffer {
    /// Create an empty buffer
    pub fn new() -> Self {
        Self::default()
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

    unsafe fn add_inner(&mut self, ptr: *mut u8, ty: TypeInfo) {
        let offset = align(self.cursor, ty.layout().align());
        let end = offset + ty.layout().size();

        if end > self.layout.size() || ty.layout().align() > self.layout.align() {
            let new_align = self.layout.align().max(ty.layout().align());
            let (new_storage, new_layout) = Self::grow(end, self.cursor, new_align, self.storage);
            if self.layout.size() != 0 {
                dealloc(self.storage.as_ptr(), self.layout);
            }
            self.storage = new_storage;
            self.layout = new_layout;
        }

        let addr = self.storage.as_ptr().add(offset);
        ptr::copy_nonoverlapping(ptr, addr, ty.layout().size());
        self.info.push(ComponentInfo {
            ty_info: ty,
            offset,
        });
        self.cursor = end;
    }

    /// Record an entity spawn operation
    pub fn spawn_at(&mut self, ent: Entity, bundle: impl DynamicBundle) {
        let len = bundle.type_info().len();
        unsafe {
            bundle.put(|ptr, ty| self.add_inner(ptr, ty));
        }
        let begin = self.entities.last().map_or(0, |x| x.end);
        let end = begin + len;
        self.entities.push(EntityIndex {
            entity: ent,
            begin,
            end,
        });
    }

    /// Spawn every entity recorded with their components
    ///
    /// # Example
    /// ```
    /// # use hecs::*;
    /// let mut world = World::new();
    /// let a = world.reserve_entity();
    /// let mut recorder = CommandBuffer::new();
    /// recorder.spawn_at(a, (false,0.0));
    /// recorder.run_on(&mut world);
    /// assert!(world.contains(a));
    /// ```
    pub fn run_on(&mut self, world: &mut World) {
        let mut mark = self.entities.len() - 1;

        for index in self.entities.iter() {
            self.info[index.begin..index.end].sort_unstable_by_key(|z| z.ty_info);
        }

        for _ in 0..self.entities.len() {
            let (ent, comps) = self.build(mark);
            world.spawn_at(ent, comps);
            if mark != 0 {
                mark -= 1;
            }
        }
        self.clear();
    }

    fn build(&mut self, mark: usize) -> (Entity, ReadyBuffer<'_>) {
        self.ids.clear();
        self.ids.extend(
            self.info[self.entities[mark].begin..self.entities[mark].end]
                .iter()
                .map(|x| x.ty_info.id()),
        );
        let entity = self.entities[mark].entity;
        (entity, ReadyBuffer { buffer: self, mark })
    }

    /// Drop previously `recorded` entities and their components
    ///
    /// Recorder is cleared implicitly when entities are spawned, so usually this doesn't need to
    /// be called
    pub fn clear(&mut self) {
        self.ids.clear();
        self.entities.clear();
        self.cursor = 0;
        unsafe {
            for info in self.info.drain(..) {
                info.ty_info.drop(self.storage.as_ptr().add(info.offset));
            }
        }
    }
}

unsafe impl Send for CommandBuffer {}
unsafe impl Sync for CommandBuffer {}

impl Drop for CommandBuffer {
    fn drop(&mut self) {
        self.clear();
        if self.layout.size() != 0 {
            unsafe {
                dealloc(self.storage.as_ptr(), self.layout);
            }
        }
    }
}

impl Default for CommandBuffer {
    /// Create an empty buffer
    fn default() -> Self {
        Self {
            entities: Vec::new(),
            storage: NonNull::dangling(),
            layout: Layout::from_size_align(0, 8).unwrap(),
            cursor: 0,
            info: Vec::new(),
            ids: Vec::new(),
        }
    }
}

/// The output of an '[CommandBuffer]` suitable for passing to
/// [`World::spawn_into`](crate::World::spawn_into)
struct ReadyBuffer<'a> {
    buffer: &'a mut CommandBuffer,
    mark: usize,
}
unsafe impl DynamicBundle for ReadyBuffer<'_> {
    fn with_ids<T>(&self, f: impl FnOnce(&[TypeId]) -> T) -> T {
        f(&self.buffer.ids)
    }

    fn type_info(&self) -> Vec<TypeInfo> {
        self.buffer.info[self.buffer.entities[self.mark].begin..self.buffer.entities[self.mark].end]
            .iter()
            .map(|x| x.ty_info)
            .collect()
    }

    unsafe fn put(self, mut f: impl FnMut(*mut u8, TypeInfo)) {
        for info in self
            .buffer
            .info
            .drain(self.buffer.entities[self.mark].begin..self.buffer.entities[self.mark].end)
        {
            let ptr = self.buffer.storage.as_ptr().add(info.offset);
            f(ptr, info.ty_info);
        }
    }
}

/// Data required to store components and their offset  
struct ComponentInfo {
    ty_info: TypeInfo,
    // Position in 'storage'
    offset: usize,
}

/// Data of buffered 'entity' and its relative position in component data
struct EntityIndex {
    entity: Entity,
    // Range of associated indices in `CommandBuffer`'s `info` member
    begin: usize,
    end: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn populate_archetypes() {
        let mut world = World::new();
        let mut buffer = CommandBuffer::new();
        let ent = world.reserve_entity();
        let enta = world.reserve_entity();
        let entb = world.reserve_entity();
        let entc = world.reserve_entity();
        buffer.spawn_at(ent, (true, "a"));
        buffer.spawn_at(entc, (true, "a"));
        buffer.spawn_at(enta, (1, 1.0));
        buffer.spawn_at(entb, (1.0, "a"));
        buffer.run_on(&mut world);
        assert_eq!(world.archetypes().len(), 4);
    }
}
