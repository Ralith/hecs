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
    pub(crate) ent: Vec<EntityIndex>,
    storage: NonNull<u8>,
    layout: Layout,
    cursor: usize,
    info: Vec<InfoBuffer>,
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
        self.info.push(InfoBuffer {
            ty_info: ty,
            size: offset,
        });
        self.cursor = end;
    }

    /// Buffer 'reserved_entity' with 'bundle'
    ///
    /// Can also be used with 'EntityBuilder'
    pub fn spawn_at(&mut self, ent: Entity, bundle: impl DynamicBundle) {
        let len = bundle.type_info().len();
        unsafe {
            bundle.put(|ptr, ty| self.add_inner(ptr, ty));
        }
        let beg = self.get_beg();
        let end = beg + len;
        self.ent.push(EntityIndex {
            entity: ent,
            beg,
            end,
        });
    }

    fn get_beg(&self) -> usize {
        match self.ent.len() {
            0 => 0,
            _ => self.ent[self.ent.len() - 1].end,
        }
    }

    /// Spawn every `entity` recorded with their components
    ///
    /// Useful for recording and spawning entities at some point in the future
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
        let mut mark: usize = 0;
        self.sort_buffered();
        (0..self.ent.len()).for_each(|_| {
            let (ent, comps) = self.build(mark);
            world.spawn_at(ent, comps);
            mark += 1;
        });
        self.clear();
    }

    pub(crate) fn sort_buffered(&mut self) {
        for index in self.ent.iter() {
            self.info[index.beg..index.end].sort_unstable_by_key(|z| z.ty_info);
        }
    }

    pub(crate) fn build(&mut self, mark: usize) -> (Entity, ReadyBuffer<'_>) {
        let end = self.ent[0].end - self.ent[0].beg;
        self.ids
            .extend(self.info[0..end].iter().map(|x| x.ty_info.id()));
        let ent = self.ent[mark].entity;
        (ent, ReadyBuffer { buffer: self })
    }

    /// Drop previously `recorded` entities and their components
    ///
    /// Recorder is cleared implicitly when entities are spawned, so usually this doesn't need to
    /// be called
    pub fn clear(&mut self) {
        self.ids.clear();
        self.ent.clear();
        self.cursor = 0;
        unsafe {
            for info in self.info.drain(..) {
                info.ty_info.drop(self.storage.as_ptr().add(info.size));
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
            ent: Vec::new(),
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
pub struct ReadyBuffer<'a> {
    buffer: &'a mut CommandBuffer,
}
unsafe impl DynamicBundle for ReadyBuffer<'_> {
    fn with_ids<T>(&self, f: impl FnOnce(&[TypeId]) -> T) -> T {
        f(&self.buffer.ids)
    }

    fn type_info(&self) -> Vec<TypeInfo> {
        let end = self.buffer.ent[0].end - self.buffer.ent[0].beg;
        self.buffer.info[0..end].iter().map(|x| x.ty_info).collect()
    }

    unsafe fn put(self, mut f: impl FnMut(*mut u8, TypeInfo)) {
        let end = self.buffer.ent[0].end - self.buffer.ent[0].beg;
        for info in self.buffer.info.drain(0..end) {
            let ptr = self.buffer.storage.as_ptr().add(info.size);
            f(ptr, info.ty_info);
        }
    }
}

/// Data required to store components and their offset  
pub struct InfoBuffer {
    pub ty_info: TypeInfo,
    pub size: usize,
}

/// Data of buffered 'entity' and its relative position in component data
pub struct EntityIndex {
    pub entity: Entity,
    pub beg: usize,
    pub end: usize,
}
