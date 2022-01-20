use core::{any::TypeId, arch, cell::RefCell, ptr::NonNull};
use std::{fs::remove_dir, thread::LocalKey};

use alloc::vec::Vec;
use hashbrown::HashMap;

use crate::{
    entities::Location, Bundle, Component, ComponentError, DynamicBundle, Entity, EntityBuilder,
    MissingComponent, NoSuchEntity, TypeInfo, World,
};

use super::access::*;

pub struct ErgoScope<'a> {
    original_world_ref: &'a mut World,
    // We have to take ownership by core::mem::swap-ing the World into the scope,
    // since it'd otherwise be possible to core::mem::forget the ErgoScope to avoid
    // invoking the `Drop` impl, which is required for soundness.
    world: World,
    access: AccessControl,
    override_data: RefCell<HashMap<Entity, EntityOverride>>,
}

impl<'a> ErgoScope<'a> {
    pub fn new(world: &'a mut World) -> Self {
        let mut access = AccessControl::default();
        access.prepare(world);
        let mut world_temp = World::default();
        core::mem::swap(&mut world_temp, world);
        Self {
            original_world_ref: world,
            world: world_temp,
            access,
            override_data: Default::default(),
        }
    }

    /// Returns a `ComponentRef` to the `T` component of `entity`
    ///
    /// Panics if the component is already uniquely borrowed from another entity with the same
    /// components.
    pub fn get<T: Component>(&self, entity: Entity) -> Result<ComponentRef<T>, ComponentError> {
        if !self.access.is_entity_overridden(entity) {
            let location = self.world.entities().get(entity)?;
            let archetype = &self.world.archetypes_inner()[location.archetype as usize];
            let type_id = TypeId::of::<T>();
            let layout = alloc::alloc::Layout::new::<T>().pad_to_align();
            unsafe {
                let addr = archetype
                    .get_dynamic(type_id, layout.size(), location.index)
                    .ok_or(ComponentError::MissingComponent(
                        MissingComponent::new::<T>(),
                    ))?;
                Ok(self.access.get_typed_component_ref(addr))
            }
        } else {
            let override_map = self.override_data.borrow();
            let data = override_map
                .get(&entity)
                .expect("override data not present despite entity being marked as overriden");
            match data {
                EntityOverride::Deleted => {
                    return Err(ComponentError::NoSuchEntity);
                }
                EntityOverride::Changed(data) => {
                    let type_id = TypeId::of::<T>();
                    let addr =
                        data.get_data_ptr(type_id)
                            .ok_or(ComponentError::MissingComponent(
                                MissingComponent::new::<T>(),
                            ))?;
                    unsafe { Ok(self.access.get_typed_component_ref(addr)) }
                }
            }
        }
    }

    /// Add `component` to `entity`
    ///
    /// See [`insert`](Self::insert).
    pub fn insert_one(
        &self,
        entity: Entity,
        component: impl Component,
    ) -> Result<(), NoSuchEntity> {
        self.insert(entity, (component,))
    }

    /// Add `components` to `entity`
    ///
    /// When inserting a single component, see [`insert_one`](Self::insert_one) for convenience.
    pub fn insert(
        &self,
        entity: Entity,
        components: impl DynamicBundle,
    ) -> Result<(), NoSuchEntity> {
        // TODO ensure there are no active locks on affected component data
        if self.access.is_entity_overridden(entity) {
            let mut override_map = self.override_data.borrow_mut();
            let data = override_map
                .get_mut(&entity)
                .expect("override data not present despite entity being marked as overriden");
            match data {
                EntityOverride::Deleted => return Err(NoSuchEntity),
                EntityOverride::Changed(data) => {
                    unsafe {
                        components.put(|src_ptr, type_info| {
                            data.put_component(type_info, src_ptr);
                        });
                    }
                    return Ok(());
                }
            }
        } else {
            // first create a EntityOverrideData from the entity's existing data
            let mut override_data = EntityOverrideData::from_world(&self.world, entity)?;
            // then put the new components in the data
            unsafe {
                components.put(|ptr, ty| {
                    override_data.put_component(ty, ptr);
                });
            };
            self.override_data
                .borrow_mut()
                .insert(entity, EntityOverride::Changed(override_data));
            self.access.set_entity_overridden(entity);
            Ok(())
        }
    }

    /// Remove components from `entity`
    ///
    /// When removing a single component, see [`remove_one`](Self::remove_one) for convenience.
    pub fn remove<T: Bundle + 'static>(&self, entity: Entity) -> Result<T, ComponentError> {
        // TODO ensure there are no active locks on the component data
        if !self.access.is_entity_overridden(entity) {
            // if we don't have override data, create it before proceeding
            let override_data = EntityOverrideData::from_world(&self.world, entity)?;

            self.override_data
                .borrow_mut()
                .insert(entity, EntityOverride::Changed(override_data));
            self.access.set_entity_overridden(entity);
        }

        let mut override_map = self.override_data.borrow_mut();
        let data = override_map
            .get_mut(&entity)
            .expect("override data not present despite entity being marked as overriden");
        match data {
            EntityOverride::Deleted => Err(ComponentError::NoSuchEntity),
            EntityOverride::Changed(existing_data) => unsafe {
                let removed_data = T::get(|ty| existing_data.get_data_ptr(ty.id()))?;
                T::with_static_ids(|type_ids| {
                    for ty in type_ids {
                        existing_data.remove_assume_moved(*ty);
                    }
                });
                Ok(removed_data)
            },
        }
    }

    /// Destroy an entity and all its components
    pub fn despawn(&self, entity: Entity) -> Result<(), NoSuchEntity> {
        // TODO ensure there are no active locks on the component data
        if !self.access.is_entity_overridden(entity) {
            // if we don't have override data, create it before proceeding
            let override_data = EntityOverrideData::from_world(&self.world, entity)?;

            self.override_data
                .borrow_mut()
                .insert(entity, EntityOverride::Changed(override_data));
            self.access.set_entity_overridden(entity);
        }
        let mut override_map = self.override_data.borrow_mut();
        let data = override_map
            .get_mut(&entity)
            .expect("override data not present despite entity being marked as overriden");
        *data = EntityOverride::Deleted;
        Ok(())
    }

    /// Destroy an entity and all its components
    pub fn contains(&self, entity: Entity) -> bool {
        // TODO ensure there are no active locks on the component data
        if self.access.is_entity_overridden(entity) {
            let mut override_map = self.override_data.borrow_mut();
            let data = override_map
                .get_mut(&entity)
                .expect("override data not present despite entity being marked as overriden");
            !matches!(data, EntityOverride::Deleted)
        } else {
            self.world.contains(entity)
        }
    }
}

impl<'a> Drop for ErgoScope<'a> {
    fn drop(&mut self) {
        self.access.expect_zero_refs();
        for (entity, data) in self.override_data.borrow_mut().drain() {
            data.move_to_world(entity, &mut self.world);
        }
        core::mem::swap(self.original_world_ref, &mut self.world);
    }
}

enum ComponentData {
    WorldOwned(Location, NonNull<u8>),
    ScopeOwned(NonNull<u8>),
    Removed,
}

impl ComponentData {
    // Moves src_ptr into the ComponentData, replacing the existing data
    fn replace(&mut self, type_info: &TypeInfo, src_ptr: NonNull<u8>) {
        match self {
            ComponentData::ScopeOwned(data) => {
                if src_ptr != *data {
                    unsafe { type_info.drop(data.as_ptr()) }
                    *data = src_ptr;
                }
            }
            ComponentData::WorldOwned(_, data) => {
                if src_ptr != *data {
                    unsafe { type_info.drop(data.as_ptr()) }
                    *data = src_ptr;
                }
            }
            ComponentData::Removed => {
                // TODO maybe we can use a more efficient storage similar to EntityBuilder?
                unsafe {
                    let dst_ptr = alloc::alloc::alloc(type_info.layout());
                    assert!(!dst_ptr.is_null(), "allocation failed");
                    core::ptr::copy_nonoverlapping(
                        src_ptr.as_ptr(),
                        dst_ptr,
                        type_info.layout().size(),
                    );
                    *self = ComponentData::ScopeOwned(NonNull::new_unchecked(dst_ptr));
                }
            }
        }
    }
}

enum EntityOverride {
    Deleted,
    Changed(EntityOverrideData),
}

impl EntityOverride {
    fn move_to_world(self, entity: Entity, world: &mut World) {
        match self {
            Self::Deleted => {
                // I suppose we don't really care if the entity didn't exist
                let _result = world.despawn(entity);
            }
            Self::Changed(data) => {
                let mut removed_components = Vec::new();
                for idx in 0..data.types.len() {
                    if let ComponentData::Removed = &data.components[idx] {
                        removed_components.push(data.types[idx]);
                    }
                }
                if !removed_components.is_empty() {
                    let _removed_data = world
                        .remove_dynamic(entity, &removed_components)
                        .expect("error removing components in move_to_world");
                }
                if !world.contains(entity) {
                    world.spawn_at(entity, data);
                } else {
                    let _insert_result = world.insert(entity, data);
                }
            }
        }
    }
}

struct EntityOverrideData {
    // sorted by descending alignment then id
    components: Vec<ComponentData>,
    types: Vec<TypeInfo>,
}

impl Drop for EntityOverrideData {
    fn drop(&mut self) {
        for i in 0..self.types.len() {
            match self.components[i] {
                ComponentData::ScopeOwned(ptr) => unsafe {
                    self.types[i].drop(ptr.as_ptr());
                    alloc::alloc::dealloc(ptr.as_ptr(), self.types[i].layout())
                },
                _ => {}
            }
        }
    }
}

impl EntityOverrideData {
    // Moves a component into self, adding or replacing existing data
    unsafe fn put_component(&mut self, type_info: TypeInfo, src_ptr: *mut u8) {
        match self.get_component_data_mut(type_info.id()) {
            Some(data) => data.replace(&type_info, NonNull::new(src_ptr).unwrap()),
            None => {
                self.add_new_component(type_info, src_ptr);
            }
        }
    }
    unsafe fn add_new_component(&mut self, type_info: TypeInfo, src_ptr: *mut u8) {
        self.types.push(type_info);
        let dst_ptr = alloc::alloc::alloc(type_info.layout());
        assert!(!dst_ptr.is_null(), "allocation failed");
        core::ptr::copy_nonoverlapping(src_ptr, dst_ptr, type_info.layout().size());
        self.components
            .push(ComponentData::ScopeOwned(NonNull::new_unchecked(dst_ptr)));
        self.ensure_sorted();
    }

    fn get_component_data_mut(&mut self, type_id: TypeId) -> Option<&mut ComponentData> {
        if let Some(pos) = self.types.iter().position(|t| t.id() == type_id) {
            Some(&mut self.components[pos])
        } else {
            None
        }
    }

    fn get_data_ptr(&self, type_id: TypeId) -> Option<NonNull<u8>> {
        if let Some(pos) = self.types.iter().position(|t| t.id() == type_id) {
            match self.components[pos] {
                ComponentData::ScopeOwned(ptr) | ComponentData::WorldOwned(_, ptr) => {
                    return Some(ptr)
                }
                ComponentData::Removed => return None,
            };
        }
        None
    }

    fn ensure_sorted(&mut self) {
        let mut sorted = true;
        for idx in 1..self.types.len() {
            if self.types[idx - 1] > self.types[idx] {
                sorted = false;
            }
        }
        if sorted {
            return;
        }
        let mut new_order = (0..self.types.len())
            .map(|i| (false, i))
            .collect::<Vec<_>>();
        new_order.sort_unstable_by(|(_, x), (_, y)| self.types[*x].cmp(&self.types[*y]));

        for idx in 0..new_order.len() {
            let (done, new_idx) = &mut new_order[idx];
            if *done {
                continue;
            }
            *done = true;

            let mut prev_j = idx;
            let mut j = *new_idx;
            while idx != j {
                self.components.swap(prev_j, j);
                self.types.swap(prev_j, j);
                new_order[j].0 = true;
                prev_j = j;
                j = new_order[j].1;
            }
        }
    }

    /// Frees scope-owned memory without dropping the data,
    /// effectively assuming it has been moved
    unsafe fn remove_assume_moved(&mut self, id: TypeId) {
        if let Some(idx) = self.types.iter().position(|t| t.id() == id) {
            let ty = self.types[idx];
            let data = &mut self.components[idx];
            match data {
                ComponentData::WorldOwned(..) => *data = ComponentData::Removed,
                ComponentData::ScopeOwned(ptr) => {
                    alloc::alloc::dealloc(ptr.as_ptr(), ty.layout());
                    *data = ComponentData::Removed
                }
                _ => {}
            }
        }
    }

    fn from_world(world: &World, entity: Entity) -> Result<EntityOverrideData, NoSuchEntity> {
        let location = world.entities().get(entity)?;
        let archetype = &world.archetypes_inner()[location.archetype as usize];
        let mut component_data = Vec::new();
        let types = Vec::from(archetype.types());
        for ty in archetype.types() {
            component_data.push(ComponentData::WorldOwned(
                location,
                unsafe { archetype.get_dynamic(ty.id(), ty.layout().size(), location.index) }
                    .unwrap(),
            ));
        }
        Ok(EntityOverrideData {
            components: component_data,
            types,
        })
    }
}

unsafe impl DynamicBundle for EntityOverrideData {
    fn with_ids<T>(&self, f: impl FnOnce(&[TypeId]) -> T) -> T {
        let ids = (0..self.types.len())
            .filter_map(|idx| match self.components[idx] {
                ComponentData::ScopeOwned(..) => Some(self.types[idx].id()),
                _ => None,
            })
            .collect::<Vec<_>>();
        f(&ids)
    }

    fn type_info(&self) -> Vec<TypeInfo> {
        (0..self.types.len())
            .filter_map(|idx| match self.components[idx] {
                ComponentData::ScopeOwned(..) => Some(self.types[idx]),
                _ => None,
            })
            .collect()
    }

    unsafe fn put(mut self, mut f: impl FnMut(*mut u8, TypeInfo)) {
        for idx in 0..self.types.len() {
            match &self.components[idx] {
                ComponentData::ScopeOwned(data_ptr) => {
                    f(data_ptr.as_ptr(), self.types[idx]);
                    alloc::alloc::dealloc(data_ptr.as_ptr(), self.types[idx].layout());
                }
                _ => {}
            }
        }
        self.components.clear();
        self.types.clear();
    }
}

#[cfg(test)]
mod test {
    use super::*;
    #[test]

    fn ergo_get_read() {
        let mut world = World::new();
        let e = world.spawn((5i32, 1.5f32));
        assert!(world.len() == 1);
        let ergo_scope = ErgoScope::new(&mut world);
        let component = ergo_scope.get::<f32>(e).expect("failed to get component");
        assert_eq!(*component.read(), 1.5f32);
        let component = ergo_scope.get::<i32>(e).expect("failed to get component");
        assert_eq!(*component.read(), 5i32);
    }

    #[test]
    fn ergo_get_write() {
        let mut world = World::new();
        let e = world.spawn((5i32, 1.5f32));
        assert!(world.len() == 1);
        let ergo_scope = ErgoScope::new(&mut world);
        let mut component = ergo_scope.get::<f32>(e).expect("failed to get component");
        *component.write() = 2.5f32;
        assert_eq!(*component.read(), 2.5f32);
        let component = ergo_scope.get::<f32>(e).expect("failed to get component");
        assert_eq!(*component.read(), 2.5f32);
        let mut component = ergo_scope.get::<i32>(e).expect("failed to get component");
        *component.write() = 3i32;
        assert_eq!(*component.read(), 3i32);
        let component = ergo_scope.get::<i32>(e).expect("failed to get component");
        assert_eq!(*component.read(), 3i32);
    }

    #[test]
    fn ergo_insert() {
        let mut world = World::new();
        let e = world.spawn((5i32, 1.5f32));
        assert!(world.len() == 1);
        {
            let ergo_scope = ErgoScope::new(&mut world);

            ergo_scope
                .insert(e, (6usize,))
                .expect("failed to insert component");
            // check that reading the inserted component works
            let component = ergo_scope.get::<usize>(e).expect("failed to get component");
            assert_eq!(*component.read(), 6usize);

            // check that reading a world-owned component works after insert
            let component = ergo_scope.get::<i32>(e).expect("failed to get component");
            assert_eq!(*component.read(), 5i32);

            // check that writing then reading the inserted component works
            let mut component = ergo_scope.get::<usize>(e).expect("failed to get component");
            *component.write() = 8usize;
            assert_eq!(*component.read(), 8usize);
            let component = ergo_scope.get::<usize>(e).expect("failed to get component");
            assert_eq!(*component.read(), 8usize);

            // check that writing then reading a world-owned component works
            let mut component = ergo_scope.get::<i32>(e).expect("failed to get component");
            *component.write() = 7i32;
            assert_eq!(*component.read(), 7i32);
        }

        // check that the inserted component exists in the world
        let component = world
            .get::<usize>(e)
            .expect("failed to get inserted component");
        assert_eq!(*component, 8usize);

        // check that modified components have their values in the world
        let component = world
            .get::<i32>(e)
            .expect("failed to get inserted component");
        assert_eq!(*component, 7i32);
    }

    #[test]
    fn ergo_remove() {
        let mut world = World::new();
        let e = world.spawn((5i32, 1.5f32));
        assert!(world.len() == 1);
        {
            let ergo_scope = ErgoScope::new(&mut world);

            ergo_scope
                .remove::<(i32,)>(e)
                .expect("failed to remove component");
            assert!(ergo_scope.get::<i32>(e).is_err());
            // check that other components still work
            let component = ergo_scope
                .get::<f32>(e)
                .expect("failed to get inserted component");
            assert_eq!(*component.read(), 1.5f32);

            ergo_scope
                .insert(e, (5i32,))
                .expect("failed to insert component");
            // check that component works after inserting again
            let component = ergo_scope.get::<i32>(e).expect("failed to get component");
            assert_eq!(*component.read(), 5i32);

            // remove again
            ergo_scope
                .remove::<(i32,)>(e)
                .expect("failed to remove component");
            assert!(ergo_scope.get::<i32>(e).is_err());
        }

        // check that removed components are removed in world
        assert!(world.get::<i32>(e).is_err());
        // check that other components still work
        let component = world
            .get::<f32>(e)
            .expect("failed to get inserted component");
        assert_eq!(*component, 1.5f32);
    }

    #[test]
    fn ergo_despawn() {
        let mut world = World::new();
        let e = world.spawn((5i32, 1.5f32));
        assert!(world.len() == 1);
        {
            let ergo_scope = ErgoScope::new(&mut world);
            assert!(ergo_scope.despawn(e).is_ok());
            assert!(!ergo_scope.contains(e));

            assert!(ergo_scope.get::<i32>(e).is_err());
        }
        assert!(!world.contains(e));

        assert!(world.get::<i32>(e).is_err());
    }

    // TODO write a test demonstrating behaviour of getting a ptr to world-owned component,
    // then removing the component, then adding a new component of the same type

    // TODO write tests for panic cases in borrowing
}
