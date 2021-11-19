use core::{any::TypeId, cell::RefCell, ptr::NonNull};

use alloc::vec::Vec;
use hashbrown::HashMap;

use crate::{
    archetype::OrderedTypeIdMap, entities::Location, Component, ComponentError, DynamicBundle,
    Entity, MissingComponent, NoSuchEntity, TypeInfo, World,
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

    pub fn insert_one(
        &self,
        entity: Entity,
        component: impl Component,
    ) -> Result<(), NoSuchEntity> {
        self.insert(entity, (component,))
    }
    pub fn insert(
        &self,
        entity: Entity,
        components: impl DynamicBundle,
    ) -> Result<(), NoSuchEntity> {
        if self.access.is_entity_overridden(entity) {
            let mut override_map = self.override_data.borrow_mut();
            let data = override_map
                .get_mut(&entity)
                .expect("override data not present despite entity being marked as overriden");
            match data {
                EntityOverride::Deleted => return Err(NoSuchEntity),
                EntityOverride::Changed(data) => {
                    let types = components.type_info();
                    todo!();
                }
            }
        } else {
            todo!()
        }
    }
}

impl<'a> Drop for ErgoScope<'a> {
    fn drop(&mut self) {
        self.access.expect_zero_refs();
        core::mem::swap(self.original_world_ref, &mut self.world);
    }
}

enum ComponentData {
    WorldOwned(Location, NonNull<u8>),
    ScopeOwned(NonNull<u8>),
}

enum EntityOverride {
    Deleted,
    Changed(EntityOverrideData),
}
struct EntityOverrideData {
    components: Vec<ComponentData>,
    // sorted by descending alignment then id
    types: Vec<TypeInfo>,
}

impl EntityOverrideData {
    fn get_data_ptr(&self, type_id: TypeId) -> Option<NonNull<u8>> {
        todo!()
    }
}

unsafe impl DynamicBundle for EntityOverrideData {
    fn with_ids<T>(&self, f: impl FnOnce(&[TypeId]) -> T) -> T {
        let ids: Vec<TypeId> = self.types.iter().map(|t| t.id()).collect();
        f(&ids)
    }

    fn type_info(&self) -> Vec<TypeInfo> {
        self.types.clone()
    }

    unsafe fn put(self, f: impl FnMut(*mut u8, TypeInfo)) {
        // TODO think about how to handle WorldOwned data here
        todo!()
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
}
