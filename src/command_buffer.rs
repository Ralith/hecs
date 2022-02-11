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
use crate::World;
use crate::{align, DynamicBundle};
use crate::{Bundle, Entity};

/// Records operations for future application to a [`World`]
///
/// Useful when operations cannot be applied directly due to ordering concerns or borrow checking.
///
/// ```
/// # use hecs::*;
/// let mut world = World::new();
/// let entity = world.reserve_entity();
/// let mut cmd = CommandBuffer::new();
/// cmd.insert(entity, (true, 42));
/// cmd.run_on(&mut world); // cmd can now be reused
/// assert_eq!(*world.get::<i32>(entity).unwrap(), 42);
/// ```
pub struct CommandBuffer {
    entities: Vec<EntityIndex>,
    remove_comps: Vec<RemovedComps>,
    despawn_ent: Vec<Entity>,
    storage: NonNull<u8>,
    layout: Layout,
    cursor: usize,
    components: Vec<ComponentInfo>,
    ids: Vec<TypeId>,
}

impl CommandBuffer {
    /// Create an empty command buffer
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
        self.components.push(ComponentInfo { ty, offset });
        self.cursor = end;
    }

    /// Add components from `bundle` to `entity`, if it exists
    ///
    /// Pairs well with [`World::reserve_entity`] to spawn entities with a known handle.
    pub fn insert(&mut self, entity: Entity, components: impl DynamicBundle) {
        let first_component = self.components.len();
        unsafe {
            components.put(|ptr, ty| self.add_inner(ptr, ty));
        }
        self.entities.push(EntityIndex {
            entity: Some(entity),
            first_component,
        });
    }

    /// Remove components from `entity` if they exist
    pub fn remove<T: Bundle + 'static>(&mut self, ent: Entity) {
        fn remove_bundle_and_ignore_result<T: Bundle + 'static>(world: &mut World, ents: Entity) {
            let _ = world.remove::<T>(ents);
        }
        self.remove_comps.push(RemovedComps {
            remove: remove_bundle_and_ignore_result::<T>,
            entity: ent,
        });
    }

    /// Despawn `entity` from World
    pub fn despawn(&mut self, entity: Entity) {
        self.despawn_ent.push(entity);
    }

    /// Spawn a new entity with `components`
    ///
    /// If the [`Entity`] is needed immediately, consider combining [`World::reserve_entity`] with
    /// [`insert`](CommandBuffer::insert) instead.
    pub fn spawn(&mut self, components: impl DynamicBundle) {
        let first_component = self.components.len();
        unsafe {
            components.put(|ptr, ty| self.add_inner(ptr, ty));
        }
        self.entities.push(EntityIndex {
            entity: None,
            first_component,
        });
    }

    /// Run recorded commands on `world`, clearing the command buffer
    pub fn run_on(&mut self, world: &mut World) {
        let mut end = self.components.len();
        for entity in self.entities.iter().rev() {
            self.components[entity.first_component..end].sort_unstable_by_key(|z| z.ty);
            end = entity.first_component;
        }

        for index in (0..self.entities.len()).rev() {
            let (entity, components) = self.build(index);
            match entity {
                Some(entity) => {
                    // If `entity` no longer exists, quietly drop the components.
                    let _ = world.insert(entity, components);
                }
                None => {
                    world.spawn(components);
                }
            }
        }

        for comp in self.remove_comps.iter() {
            (comp.remove)(world, comp.entity);
        }

        for entity in self.despawn_ent.iter() {
            world.despawn(*entity).unwrap();
        }

        self.clear();
    }

    fn build(&mut self, index: usize) -> (Option<Entity>, RecordedEntity<'_>) {
        self.ids.clear();
        self.ids.extend(
            self.components[self.entities[index].first_component..]
                .iter()
                .map(|x| x.ty.id()),
        );
        let entity = self.entities[index].entity;
        (entity, RecordedEntity { cmd: self, index })
    }

    /// Drop all recorded commands
    pub fn clear(&mut self) {
        self.ids.clear();
        self.entities.clear();
        self.cursor = 0;
        unsafe {
            for info in self.components.drain(..) {
                info.ty.drop(self.storage.as_ptr().add(info.offset));
            }
        }
        self.remove_comps.clear();
        self.despawn_ent.clear();
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
            components: Vec::new(),
            ids: Vec::new(),
            despawn_ent: Vec::new(),
            remove_comps: Vec::new(),
        }
    }
}

/// The output of an '[CommandBuffer]` suitable for passing to
/// [`World::spawn_into`](crate::World::spawn_into)
struct RecordedEntity<'a> {
    cmd: &'a mut CommandBuffer,
    index: usize,
}

unsafe impl DynamicBundle for RecordedEntity<'_> {
    fn with_ids<T>(&self, f: impl FnOnce(&[TypeId]) -> T) -> T {
        f(&self.cmd.ids)
    }

    fn type_info(&self) -> Vec<TypeInfo> {
        self.cmd.components[self.cmd.entities[self.index].first_component..]
            .iter()
            .map(|x| x.ty)
            .collect()
    }

    unsafe fn put(self, mut f: impl FnMut(*mut u8, TypeInfo)) {
        for info in self
            .cmd
            .components
            .drain(self.cmd.entities[self.index].first_component..)
        {
            let ptr = self.cmd.storage.as_ptr().add(info.offset);
            f(ptr, info.ty);
        }
    }
}

/// Data required to store components and their offset  
struct ComponentInfo {
    ty: TypeInfo,
    // Position in 'storage'
    offset: usize,
}

/// Data of buffered 'entity' and its relative position in component data
struct EntityIndex {
    entity: Option<Entity>,
    // Position of this entity's first component in `CommandBuffer::info`
    first_component: usize,
}

/// Data required to remove components from 'entity'
struct RemovedComps {
    remove: fn(&mut World, Entity),
    entity: Entity,
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
        buffer.insert(ent, (true, "a"));
        buffer.insert(entc, (true, "a"));
        buffer.insert(enta, (1, 1.0));
        buffer.insert(entb, (1.0, "a"));
        buffer.run_on(&mut world);
        assert_eq!(world.archetypes().len(), 4);
    }
}
