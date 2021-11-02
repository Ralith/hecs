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

use crate::archetype::TypeInfo;
use crate::Entity;
use crate::{align, DynamicBundle};

/// Mechanism for allowing recording of entities for the purpose of spawning them on future
///
/// ```
/// # use hecs::*;
/// let mut world = World::new();
/// let reserved_ent = world.reserve_entity();
/// let mut entity_recorder = EntityRecorder::new();
/// entity_recorder.record_entity(reserved_ent,(true,0.10));
/// world.spawn_recorded(&mut entity_recorder); // recorder can now be reused
/// assert_eq!(world.contains(reserved_ent), true);
/// ```
pub struct EntityRecorder {
    pub(crate) ent: Vec<(Entity, usize)>,
    storage: NonNull<u8>,
    layout: Layout,
    cursor: usize,
    info: Vec<(TypeInfo, usize, Entity)>,
    ids: Vec<TypeId>,
    pub(crate) mark: usize,
}

impl EntityRecorder {
    /// Create an empty recorder
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

    unsafe fn add_inner(&mut self, ptr: *mut u8, ty: TypeInfo, ent: Entity) {
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
        self.info.push((ty, offset, ent));
        self.cursor = end;
    }
    /// Record 'reserved_entity' with 'bundle'
    ///
    /// Can also be used with 'EntityBuilder' 
    pub fn record_entity(&mut self, ent: Entity, bundle: impl DynamicBundle) -> &mut Self {
        let len = bundle.type_info().len();
        unsafe {
            bundle.put(|ptr, ty| self.add_inner(ptr, ty, ent));
        }
        self.ent.push((ent, len));
        self
    }

    pub(crate) fn sort_buffered(&mut self) {
        self.info[..].sort_unstable_by_key(|z| (z.2, z.0));
    }

    fn ret_mark(&self) -> (usize, usize) {
        let (beg, end) = (0, self.ent[self.mark].1);
        (beg, end)
    }
    
    pub(crate) fn build(&mut self) -> (Entity, ReadyRecorder<'_>) {
        let (beg, end) = self.ret_mark();
        self.ids.extend(self.info[beg..end].iter().map(|x| x.0.id()));
        let (ent, _) = self.ent[self.mark];
        (ent, ReadyRecorder { recorder: self })
    }
    
    /// Drop previously `recorded` entities and their components
    ///
    /// Recorder is cleared implicitly when entities are spawned, so usually this doesn't need to
    /// be called
    pub fn clear(&mut self) {
        self.ids.clear();
        self.ent.clear();
        self.cursor = 0;
        self.mark = 0;
        unsafe {
            for (ty, offset, _) in self.info.drain(..) {
                ty.drop(self.storage.as_ptr().add(offset));
            }
        }
    }
}

unsafe impl Send for EntityRecorder {}
unsafe impl Sync for EntityRecorder {}

impl Drop for EntityRecorder {
    fn drop(&mut self) {
        self.clear();
        if self.layout.size() != 0 {
            unsafe {
                dealloc(self.storage.as_ptr(), self.layout);
            }
        }
    }
}

impl Default for EntityRecorder {
    /// Create an empty recorder
    fn default() -> Self {
        Self {
            ent: Vec::new(),
            storage: NonNull::dangling(),
            layout: Layout::from_size_align(0, 8).unwrap(),
            cursor: 0,
            info: Vec::new(),
            ids: Vec::new(),
            mark: 0,
        }
    }
}

/// The output of an '[EntityRecorder]` suitable for passing to
/// [`World::spawn_into`](crate::World::spawn_into)
pub struct ReadyRecorder<'a> {
    recorder: &'a mut EntityRecorder,
}
unsafe impl DynamicBundle for ReadyRecorder<'_> {
    fn with_ids<T>(&self, f: impl FnOnce(&[TypeId]) -> T) -> T {
        f(&self.recorder.ids)
    }

    fn type_info(&self) -> Vec<TypeInfo> {
        let (beg, end) = self.recorder.ret_mark();
        self.recorder.info[beg..end].iter().map(|x| x.0).collect()
    }

    unsafe fn put(self, mut f: impl FnMut(*mut u8, TypeInfo)) {
        let (beg, end) = self.recorder.ret_mark();
        for (ty, offset, _) in self.recorder.info.drain(beg..end) {
            let ptr = self.recorder.storage.as_ptr().add(offset);
            f(ptr, ty);
        }
    }
}
