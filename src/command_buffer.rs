use core::any::TypeId;
use core::mem;
use core::ops::Range;
use core::ptr::{self, NonNull};

use crate::alloc::alloc::{alloc, dealloc, Layout};
use crate::alloc::vec::Vec;
use crate::archetype::TypeInfo;
use crate::{align, DynamicBundle};
use crate::{Bundle, Entity};
use crate::{Component, World};

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
/// assert_eq!(*world.get::<&i32>(entity).unwrap(), 42);
/// ```
pub struct CommandBuffer {
    cmds: Vec<Cmd>,
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
    ///
    /// When inserting a single component, see [`insert_one`](Self::insert_one) for convenience.
    pub fn insert(&mut self, entity: Entity, components: impl DynamicBundle) {
        let first_component = self.components.len();
        unsafe {
            components.put(|ptr, ty| self.add_inner(ptr, ty));
        }
        self.components[first_component..].sort_unstable_by_key(|c| c.ty);
        self.cmds.push(Cmd::SpawnOrInsert(EntityIndex {
            entity: Some(entity),
            components: first_component..self.components.len(),
        }));
    }

    /// Add `component` to `entity`, if the entity exists
    ///
    /// See [`insert`](Self::insert).
    pub fn insert_one(&mut self, entity: Entity, component: impl Component) {
        self.insert(entity, (component,));
    }

    /// Remove components from `entity` if they exist
    ///
    /// When removing a single component, see [`remove_one`](Self::remove_one) for convenience.
    pub fn remove<T: Bundle + 'static>(&mut self, ent: Entity) {
        fn remove_bundle_and_ignore_result<T: Bundle + 'static>(world: &mut World, ents: Entity) {
            let _ = world.remove::<T>(ents);
        }
        self.cmds.push(Cmd::Remove(RemovedComps {
            remove: remove_bundle_and_ignore_result::<T>,
            entity: ent,
        }));
    }

    /// Remove a component from `entity` if it exists
    ///
    /// See [`remove`](Self::remove).
    pub fn remove_one<T: Component>(&mut self, ent: Entity) {
        self.remove::<(T,)>(ent);
    }

    /// Despawn `entity` from World
    pub fn despawn(&mut self, entity: Entity) {
        self.cmds.push(Cmd::Despawn(entity));
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
        self.components[first_component..].sort_unstable_by_key(|c| c.ty);
        self.cmds.push(Cmd::SpawnOrInsert(EntityIndex {
            entity: None,
            components: first_component..self.components.len(),
        }));
    }

    /// Run recorded commands on `world`, clearing the command buffer
    pub fn run_on(&mut self, world: &mut World) {
        for i in 0..self.cmds.len() {
            match mem::replace(&mut self.cmds[i], Cmd::Despawn(Entity::DANGLING)) {
                Cmd::SpawnOrInsert(entity) => {
                    let components = self.build(entity.components);
                    match entity.entity {
                        Some(entity) => {
                            // If `entity` no longer exists, quietly drop the components.
                            let _ = world.insert(entity, components);
                        }
                        None => {
                            world.spawn(components);
                        }
                    }
                }
                Cmd::Remove(remove) => {
                    (remove.remove)(world, remove.entity);
                }
                Cmd::Despawn(entity) => {
                    let _ = world.despawn(entity);
                }
            }
        }
        // Wipe out component references so `clear` doesn't try to double-free
        self.components.clear();

        self.clear();
    }

    fn build(&mut self, components: Range<usize>) -> RecordedEntity<'_> {
        self.ids.clear();
        self.ids.extend(
            self.components[components.clone()]
                .iter()
                .map(|x| x.ty.id()),
        );
        RecordedEntity {
            cmd: self,
            components,
        }
    }

    /// Drop all recorded commands
    pub fn clear(&mut self) {
        self.ids.clear();
        self.cursor = 0;
        for info in self.components.drain(..) {
            unsafe {
                info.ty.drop(self.storage.as_ptr().add(info.offset));
            }
        }
        self.cmds.clear();
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
            cmds: Vec::new(),
            storage: NonNull::dangling(),
            layout: Layout::from_size_align(0, 8).unwrap(),
            cursor: 0,
            components: Vec::new(),
            ids: Vec::new(),
        }
    }
}

/// The output of an '[CommandBuffer]` suitable for passing to
/// [`World::spawn_into`](crate::World::spawn_into)
struct RecordedEntity<'a> {
    cmd: &'a mut CommandBuffer,
    components: Range<usize>,
}

unsafe impl DynamicBundle for RecordedEntity<'_> {
    fn with_ids<T>(&self, f: impl FnOnce(&[TypeId]) -> T) -> T {
        f(&self.cmd.ids)
    }

    fn type_info(&self) -> Vec<TypeInfo> {
        self.cmd.components[self.components.clone()]
            .iter()
            .map(|x| x.ty)
            .collect()
    }

    unsafe fn put(mut self, mut f: impl FnMut(*mut u8, TypeInfo)) {
        // Zero out the components slice so `drop` won't double-free
        let components = mem::replace(&mut self.components, 0..0);
        for info in &self.cmd.components[components] {
            let ptr = self.cmd.storage.as_ptr().add(info.offset);
            f(ptr, info.ty);
        }
    }
}

impl Drop for RecordedEntity<'_> {
    fn drop(&mut self) {
        // If `put` was never called, we still need to drop this entity's components and discard
        // their info.
        unsafe {
            for info in &self.cmd.components[self.components.clone()] {
                info.ty.drop(self.cmd.storage.as_ptr().add(info.offset));
            }
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
    // Position of this entity's components in `CommandBuffer::info`
    //
    // We could store a single start point for the first initialized entity, rather than one for
    // each, but this would be more error prone for marginal space savings.
    components: Range<usize>,
}

/// Data required to remove components from 'entity'
struct RemovedComps {
    remove: fn(&mut World, Entity),
    entity: Entity,
}

/// A buffered command
enum Cmd {
    SpawnOrInsert(EntityIndex),
    Remove(RemovedComps),
    Despawn(Entity),
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

    #[test]
    fn failed_insert_regression() {
        // Verify that failing to insert components doesn't lead to concatenating components
        // together
        #[derive(Clone)]
        struct A;

        let mut world = World::new();

        // Get two IDs
        let a = world.spawn((A,));
        let b = world.spawn((A,));

        // Invalidate them both
        world.clear();

        let mut cmd = CommandBuffer::new();
        cmd.insert_one(a, A);
        cmd.insert_one(b, A);

        // Make `a` valid again
        world.spawn_at(a, ());

        // The insert to `a` should succeed
        cmd.run_on(&mut world);

        assert!(world.satisfies::<&A>(a).unwrap());
    }

    #[test]
    fn insert_then_remove() {
        let mut world = World::new();
        let a = world.spawn(());
        let mut cmd = CommandBuffer::new();
        cmd.insert_one(a, 42i32);
        cmd.remove_one::<i32>(a);
        cmd.run_on(&mut world);
        assert!(!world.satisfies::<&i32>(a).unwrap());
    }

    #[test]
    fn remove_then_insert() {
        let mut world = World::new();
        let a = world.spawn((17i32,));
        let mut cmd = CommandBuffer::new();
        cmd.remove_one::<i32>(a);
        cmd.insert_one(a, 42i32);
        cmd.run_on(&mut world);
        assert_eq!(*world.get::<&i32>(a).unwrap(), 42);
    }
}
