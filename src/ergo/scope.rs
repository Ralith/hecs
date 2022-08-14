use core::{
    any::TypeId,
    cell::RefCell,
    ptr::{null_mut, NonNull},
};

use alloc::{rc::Rc, vec::Vec};
use hashbrown::HashMap;

use crate::{
    entities::Location, Bundle, Component, ComponentError, DynamicBundle, Entity, MissingComponent,
    NoSuchEntity, TypeInfo, World,
};

use super::{
    access::*,
    query::{Query, QueryBorrow},
};

#[derive(Default)]
pub(super) struct ActiveQueryState {
    pub archetype_idx: u32,
    pub archetype_iter_pos: u32,
    pub new_entity_iter_pos: u32,
    pub entity_match_fn: Option<fn(&ErgoScope, Entity) -> bool>,
    pub new_entities: Vec<(Entity, bool)>,
}

pub struct ErgoScope<'a> {
    original_world_ref: &'a mut World,
    // We have to take ownership by core::mem::swap-ing the World into the scope,
    // since it'd otherwise be possible to core::mem::forget the ErgoScope to avoid
    // invoking the `Drop` impl, which is required for soundness.
    pub(super) world: World,
    pub(super) access: AccessControl,
    override_data: RefCell<HashMap<Entity, EntityOverride>>,
    query_states: RefCell<Vec<Rc<RefCell<ActiveQueryState>>>>,
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
            query_states: Default::default(),
        }
    }

    pub(super) fn alloc_query_state(&self) -> Rc<RefCell<ActiveQueryState>> {
        for query_state in self.query_states.borrow().iter() {
            if Rc::strong_count(query_state) == 1 {
                *query_state.borrow_mut() = Default::default();
                return query_state.clone();
            }
        }
        let new = Rc::new(RefCell::default());
        self.query_states.borrow_mut().push(new.clone());
        new
    }

    /// Returns a `ComponentRef` to the `T` component of `entity`
    pub fn get<T: Component>(&self, entity: Entity) -> Result<ComponentRef<T>, ComponentError> {
        if !self.access.is_entity_overridden(entity) {
            let location = self.world.entities().get(entity)?;
            let archetype = &self.world.archetypes_inner()[location.archetype as usize];
            let type_info = TypeInfo::of::<T>();
            let layout = alloc::alloc::Layout::new::<T>().pad_to_align();
            unsafe {
                let addr = archetype
                    .get_dynamic(type_info.id(), layout.size(), location.index)
                    .ok_or_else(
                        || ComponentError::MissingComponent(MissingComponent::new::<T>()),
                    )?;
                Ok(self
                    .access
                    .get_typed_component_ref(entity, &type_info, addr))
            }
        } else {
            self.get_overriden(entity)
        }
    }

    pub(super) fn get_overriden<T: Component>(
        &self,
        entity: Entity,
    ) -> Result<ComponentRef<T>, ComponentError> {
        let override_map = self.override_data.borrow();
        let data = override_map
            .get(&entity)
            .expect("override data not present despite entity being marked as overriden");
        match data {
            EntityOverride::Deleted => Err(ComponentError::NoSuchEntity),
            EntityOverride::Changed(data) => {
                let type_info = TypeInfo::of::<T>();
                let addr = data.get_data_ptr(type_info.id()).ok_or_else(|| {
                    ComponentError::MissingComponent(MissingComponent::new::<T>())
                })?;
                unsafe {
                    Ok(self
                        .access
                        .get_typed_component_ref(entity, &type_info, addr))
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
        let mut was_overridden = true;
        let result = if self.access.is_entity_overridden(entity) {
            let mut override_map = self.override_data.borrow_mut();
            let data = override_map
                .get_mut(&entity)
                .expect("override data not present despite entity being marked as overriden");
            match data {
                EntityOverride::Deleted => Err(NoSuchEntity),
                EntityOverride::Changed(data) => {
                    unsafe {
                        components.put(|src_ptr, type_info| {
                            if self.access.has_active_borrows(entity, type_info.id()) {
                                panic!("Component {} on entity {:?} has an active borrow when inserting component", type_info.name().unwrap_or(""), entity);
                            }
                            if let Some(new_ptr) = data.put_component(type_info, src_ptr) {
                                self.access.update_data_ptr(entity, &type_info, new_ptr);
                            }
                        });
                    }
                    Ok(())
                }
            }
        } else {
            // first create a EntityOverrideData from the entity's existing data
            let mut override_data = EntityOverrideData::from_world(&self.world, entity)?;
            // then put the new components in the data
            unsafe {
                components.put(|ptr, ty| {
                    if self.access.has_active_borrows(entity, ty.id()) {
                        panic!("Component {} on entity {:?} has an active borrow when inserting component", ty.name().unwrap_or(""), entity);
                    }
                    if let Some(new_ptr) = override_data.put_component(ty, ptr) {
                        self.access.update_data_ptr(entity, &ty, new_ptr);
                    }
                });
            };
            self.override_data
                .borrow_mut()
                .insert(entity, EntityOverride::Changed(override_data));
            self.access.set_entity_overridden(entity);
            was_overridden = false;
            Ok(())
        };
        if result.is_ok() {
            self.on_entity_archetype_changed(entity, !was_overridden);
        }
        result
    }

    /// Create an entity with certain components
    ///
    /// Returns the ID of the newly created entity.
    ///
    /// Arguments can be tuples, structs annotated with [`#[derive(Bundle)]`](macro@Bundle), or the
    /// result of calling [`build`](crate::EntityBuilder::build) on an
    /// [`EntityBuilder`](crate::EntityBuilder), which is useful if the set of components isn't
    /// statically known. To spawn an entity with only one component, use a one-element tuple like
    /// `(x,)`.
    ///
    /// Any type that satisfies `Send + Sync + 'static` can be used as a component.
    ///
    /// # Example
    /// ```
    /// # use hecs::ergo::*;
    /// let mut world = World::new();
    /// let ergo = ErgoScope::new(&mut world);
    /// let a = ergo.spawn((123, "abc"));
    /// let b = ergo.spawn((456, true));
    /// ```
    pub fn spawn(&self, components: impl DynamicBundle) -> Entity {
        let entity = self.world.reserve_entity();
        self.spawn_at(entity, components);
        entity
    }

    /// Create an entity with certain components and a specific [`Entity`] handle.
    ///
    /// See [`spawn`](Self::spawn).
    ///
    /// Despawns any existing entity with the same [`Entity::id`].
    ///
    /// # Example
    /// ```
    /// # use hecs::ergo::*;
    /// let mut world = World::new();
    /// let ergo = ErgoScope::new(&mut world);
    /// let a = ergo.spawn((123, "abc"));
    /// let b = ergo.spawn((456, true));
    /// ergo.despawn(a);
    /// assert!(!ergo.contains(a));
    /// // all previous Entity values pointing to 'a' will be live again, instead pointing to the new entity.
    /// ergo.spawn_at(a, (789, "ABC"));
    /// assert!(ergo.contains(a));
    /// ```
    pub fn spawn_at(&self, entity: Entity, components: impl DynamicBundle) {
        if self.access.is_entity_overridden(entity) {
            let _ = self.despawn(entity);
        } else {
            self.access.set_entity_overridden(entity);
        }
        // first create a EntityOverrideData from the entity's existing data
        let mut override_data = EntityOverrideData::new();
        // then put the new components in the data
        unsafe {
            components.put(|ptr, ty| {
                if let Some(new_ptr) = override_data.put_component(ty, ptr) {
                    self.access.update_data_ptr(entity, &ty, new_ptr);
                }
            });
        };
        self.override_data
            .borrow_mut()
            .insert(entity, EntityOverride::Changed(override_data));
        self.on_entity_archetype_changed(entity, true);
    }

    /// Allocate an entity ID
    pub fn reserve_entity(&self) -> Entity {
        self.world.entities().reserve_entity()
    }

    /// Remove the `T` component from `entity`
    ///
    /// See [`remove`](Self::remove).
    pub fn remove_one<T: Component>(&self, entity: Entity) -> Result<T, ComponentError> {
        self.remove::<(T,)>(entity).map(|(x,)| x)
    }

    /// Remove components from `entity`
    ///
    /// When removing a single component, see [`remove_one`](Self::remove_one) for convenience.
    pub fn remove<T: Bundle + 'static>(&self, entity: Entity) -> Result<T, ComponentError> {
        let mut was_overridden = true;
        let result = {
            if !self.access.is_entity_overridden(entity) {
                // if we don't have override data, create it before proceeding
                let override_data = EntityOverrideData::from_world(&self.world, entity)?;

                self.override_data
                    .borrow_mut()
                    .insert(entity, EntityOverride::Changed(override_data));
                self.access.set_entity_overridden(entity);
                was_overridden = false;
            }

            let mut override_map = self.override_data.borrow_mut();
            let data = override_map
                .get_mut(&entity)
                .expect("override data not present despite entity being marked as overriden");
            match data {
                EntityOverride::Deleted => Err(ComponentError::NoSuchEntity),
                EntityOverride::Changed(existing_data) => unsafe {
                    let removed_data = T::get(|ty| existing_data.get_data_ptr(ty.id()))?;
                    T::with_static_type_info(|types| {
                        for ty in types {
                            if self.access.has_active_borrows(entity, ty.id()) {
                                panic!("Component {} on entity {:?} has an active borrow when removing component", ty.name().unwrap_or(""), entity);
                            }
                            if existing_data.remove_assume_moved(ty.id()) {
                                self.access.update_data_ptr(entity, &ty, null_mut());
                            }
                        }
                    });
                    Ok(removed_data)
                },
            }
        };
        if result.is_ok() {
            self.on_entity_archetype_changed(entity, !was_overridden);
        }
        result
    }

    /// Destroy an entity and all its components
    pub fn despawn(&self, entity: Entity) -> Result<(), NoSuchEntity> {
        let mut change_made_overridden = false;
        let result = {
            if !self.access.is_entity_overridden(entity) {
                // if we don't have override data, create it before proceeding
                let override_data = EntityOverrideData::from_world(&self.world, entity)?;

                self.override_data
                    .borrow_mut()
                    .insert(entity, EntityOverride::Changed(override_data));
                self.access.set_entity_overridden(entity);
                change_made_overridden = true;
            }
            let mut override_map = self.override_data.borrow_mut();
            let entity_override = override_map
                .get_mut(&entity)
                .expect("override data not present despite entity being marked as overriden");
            match entity_override {
                EntityOverride::Deleted => return Err(NoSuchEntity),
                EntityOverride::Changed(data) => unsafe {
                    for ty in &data.types {
                        if self.access.has_active_borrows(entity, ty.id()) {
                            panic!("Component {} on entity {:?} has an active borrow when despawning entity", ty.name().unwrap_or(""), entity);
                        }
                        self.access.update_data_ptr(entity, &ty, null_mut());
                    }
                },
            }
            *entity_override = EntityOverride::Deleted;
            Ok(())
        };
        self.on_entity_archetype_changed(entity, change_made_overridden);
        result
    }

    // TODO implement len()

    // TODO implement
    // pub fn satisfies<Q: Query>(&self, entity: Entity) -> Result<bool, NoSuchEntity> {
    //     Ok(self.entity(entity)?.satisfies::<Q>())
    // }

    /// Whether `entity` exists
    pub fn contains(&self, entity: Entity) -> bool {
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

    // TODO implement query_one

    /// Iterate over all entities that have certain components.
    ///
    /// Calling `iter` on the returned value yields `(Entity, Q)` tuples, where `Q` is some query
    /// type. A query type is any type for which an implementation of [`Query`] exists, e.g. `&T`,
    /// `&mut T`, a tuple of query types, or an `Option` wrapping a query type, where `T` is any
    /// component type. Components queried with `&mut` must only appear once. Entities which do not
    /// have a component type referenced outside of an `Option` will be skipped.
    ///
    /// Entities are yielded in arbitrary order.
    ///
    /// The returned [`QueryBorrow`] can be further transformed with combinator methods; see its
    /// documentation for details.
    ///
    /// Iterating a query yields references with lifetimes bound to the [`QueryBorrow`] returned
    /// here. To ensure those are invalidated, the return value of this method must be dropped for
    /// its dynamic borrows from the world to be released. Similarly, lifetime rules ensure that
    /// references obtained from a query cannot outlive the [`QueryBorrow`].
    ///
    /// # Example
    /// ```
    /// # use hecs::ergo::*;
    /// let mut world = World::new();
    /// let a = world.spawn((123, true, "abc"));
    /// let b = world.spawn((456, false));
    /// let c = world.spawn((42, "def"));
    /// let ergo = ErgoScope::new(&mut world);
    /// let entities = ergo.query::<(&i32, &bool)>()
    ///     .iter()
    ///     .map(|(e, (i, b))| (e, *i.read(), *b.read())) // Copy out of the world
    ///     .collect::<Vec<_>>();
    /// assert_eq!(entities.len(), 2);
    /// assert!(entities.contains(&(a, 123, true)));
    /// assert!(entities.contains(&(b, 456, false)));
    /// ```
    pub fn query<Q: Query>(&self) -> QueryBorrow<'_, Q> {
        QueryBorrow::new(
            self,
            &self.world.entities().meta,
            self.world.archetypes_inner(),
        )
    }

    fn on_entity_archetype_changed(&self, entity: Entity, change_made_overridden: bool) {
        // Insert into override-results of active queries,
        // if the entity is not already in the list.
        // If the entity was not previously overridden, check if each query
        // had already processed the entity in the archetype. If so, the entity was already
        // processed and should be marked as such.
        let query_states = self.query_states.borrow();
        for state in query_states.iter() {
            let mut state = state.borrow_mut();
            let mut mark_as_processed = false;
            if change_made_overridden {
                if let Ok(original_location) = self.world.entities().get(entity) {
                    mark_as_processed = original_location.archetype < state.archetype_idx
                        || (original_location.archetype == state.archetype_idx
                            && original_location.index <= state.archetype_iter_pos);
                }
            }
            let mut found = false;
            for (new_entity, processed) in &mut state.new_entities {
                if *new_entity == entity {
                    found = true;
                    *processed |= mark_as_processed;
                    break;
                }
            }
            if !found {
                state.new_entities.push((entity, mark_as_processed));
            }
        }
    }
}

impl<'a> Drop for ErgoScope<'a> {
    fn drop(&mut self) {
        if self.access.has_active_refs() {
            // Since there may be active ComponentRefs, we must leak AccessControl's heap
            // allocated memory so that the pointers remain valid
            core::mem::forget(core::mem::take(&mut self.access));
            panic!("active references when dropping ErgoScope");
        }
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
    // Moves data at src_ptr into the ComponentData, replacing the existing data
    // Returns the new pointer if the operation caused the underlying storage to change
    unsafe fn replace(&mut self, type_info: &TypeInfo, src_ptr: NonNull<u8>) -> Option<*mut u8> {
        match self {
            ComponentData::ScopeOwned(data) => {
                if src_ptr != *data {
                    type_info.drop(data.as_ptr());
                    core::ptr::copy_nonoverlapping(
                        src_ptr.as_ptr(),
                        data.as_ptr(),
                        type_info.layout().size(),
                    );
                }
                None
            }
            ComponentData::WorldOwned(_, data) => {
                if src_ptr != *data {
                    type_info.drop(data.as_ptr());
                    core::ptr::copy_nonoverlapping(
                        src_ptr.as_ptr(),
                        data.as_ptr(),
                        type_info.layout().size(),
                    );
                }
                None
            }
            ComponentData::Removed => {
                // TODO maybe we can use a more efficient storage similar to EntityBuilder?
                let dst_ptr = alloc::alloc::alloc(type_info.layout());
                assert!(!dst_ptr.is_null(), "allocation failed");
                core::ptr::copy_nonoverlapping(
                    src_ptr.as_ptr(),
                    dst_ptr,
                    type_info.layout().size(),
                );
                *self = ComponentData::ScopeOwned(NonNull::new_unchecked(dst_ptr));
                Some(dst_ptr)
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
                if !world.contains(entity) {
                    world.spawn_at(entity, data);
                } else {
                    if !removed_components.is_empty() {
                        let _removed_data = world
                            .remove_dynamic(entity, &removed_components)
                            .expect("error removing components in move_to_world");
                    }
                    world
                        .insert(entity, data)
                        .expect("failed to insert components when moving changed data to world");
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
            if let ComponentData::ScopeOwned(ptr) = &self.components[i] {
                unsafe {
                    self.types[i].drop(ptr.as_ptr());
                    alloc::alloc::dealloc(ptr.as_ptr(), self.types[i].layout())
                }
            }
        }
    }
}

impl EntityOverrideData {
    fn new() -> Self {
        Self {
            components: Vec::new(),
            types: Vec::new(),
        }
    }
    // Moves a component into self, adding or replacing existing data
    unsafe fn put_component(&mut self, type_info: TypeInfo, src_ptr: *mut u8) -> Option<*mut u8> {
        match self.get_component_data_mut(type_info.id()) {
            Some(data) => data.replace(&type_info, NonNull::new(src_ptr).unwrap()),
            None => self.add_new_component(type_info, src_ptr),
        }
    }
    unsafe fn add_new_component(
        &mut self,
        type_info: TypeInfo,
        src_ptr: *mut u8,
    ) -> Option<*mut u8> {
        self.types.push(type_info);
        let dst_ptr = alloc::alloc::alloc(type_info.layout());
        assert!(!dst_ptr.is_null(), "allocation failed");
        core::ptr::copy_nonoverlapping(src_ptr, dst_ptr, type_info.layout().size());
        self.components
            .push(ComponentData::ScopeOwned(NonNull::new_unchecked(dst_ptr)));
        self.ensure_sorted();
        Some(dst_ptr)
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
    /// effectively assuming it has been moved.
    unsafe fn remove_assume_moved(&mut self, id: TypeId) -> bool {
        if let Some(idx) = self.types.iter().position(|t| t.id() == id) {
            let ty = self.types[idx];
            let data = &mut self.components[idx];
            match data {
                ComponentData::WorldOwned(..) => {
                    *data = ComponentData::Removed;
                    true
                }
                ComponentData::ScopeOwned(ptr) => {
                    alloc::alloc::dealloc(ptr.as_ptr(), ty.layout());
                    *data = ComponentData::Removed;
                    true
                }
                _ => false,
            }
        } else {
            false
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
            if let ComponentData::ScopeOwned(data_ptr) = &self.components[idx] {
                f(data_ptr.as_ptr(), self.types[idx]);
                alloc::alloc::dealloc(data_ptr.as_ptr(), self.types[idx].layout());
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
    #[should_panic]
    fn ergo_read_panic_write_active() {
        let mut world = World::new();
        let e = world.spawn((5i32,));
        assert!(world.len() == 1);
        let ergo_scope = ErgoScope::new(&mut world);
        let mut write = ergo_scope.get::<i32>(e).expect("failed to get component");
        let write = write.write();
        let read = ergo_scope.get::<i32>(e).expect("failed to get component");
        read.read();
        drop(write);
        drop(read);
    }

    #[test]
    #[should_panic]
    fn ergo_write_panic_read_active() {
        let mut world = World::new();
        let e = world.spawn((5i32,));
        assert!(world.len() == 1);
        let ergo_scope = ErgoScope::new(&mut world);
        let read = ergo_scope.get::<i32>(e).expect("failed to get component");
        let read = read.read();
        let mut write = ergo_scope.get::<i32>(e).expect("failed to get component");
        write.write();
        drop(read);
        drop(write);
    }

    #[test]
    fn ergo_insert() {
        let mut world = World::new();
        let e = world.spawn((5i32, 1.5f32));
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
            .get::<&usize>(e)
            .expect("failed to get inserted component");
        assert_eq!(*component, 8usize);

        // check that modified components have their values in the world
        let component = world
            .get::<&i32>(e)
            .expect("failed to get inserted component");
        assert_eq!(*component, 7i32);
    }

    #[test]
    #[should_panic]
    fn ergo_insert_panic_active_borrows() {
        let mut world = World::new();
        let e = world.spawn((5i32, 1.5f32));
        {
            let ergo_scope = ErgoScope::new(&mut world);
            let component = ergo_scope.get::<i32>(e).expect("failed to get component");

            let component_borrow = component.read();
            ergo_scope
                .insert(e, (8i32,))
                .expect("failed to insert component");
            drop(component_borrow);
        }
    }

    #[test]
    fn ergo_remove() {
        let mut world = World::new();
        let e = world.spawn((5i32, 1.5f32));
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
        assert!(world.get::<&i32>(e).is_err());
        // check that other components still work
        let component = world
            .get::<&f32>(e)
            .expect("failed to get inserted component");
        assert_eq!(*component, 1.5f32);
    }

    #[test]
    #[should_panic]
    fn ergo_remove_panic_active_borrow() {
        let mut world = World::new();
        let e = world.spawn((5i32, 1.5f32));
        {
            let ergo_scope = ErgoScope::new(&mut world);
            let component = ergo_scope.get::<i32>(e).expect("failed to get component");

            let component_borrow = component.read();
            ergo_scope
                .remove::<(i32,)>(e)
                .expect("failed to remove component");
            drop(component_borrow);
        }
    }

    #[test]
    fn ergo_despawn() {
        let mut world = World::new();
        let e = world.spawn((5i32, 1.5f32));
        {
            let ergo_scope = ErgoScope::new(&mut world);
            assert!(ergo_scope.despawn(e).is_ok());
            assert!(!ergo_scope.contains(e));

            assert!(ergo_scope.get::<i32>(e).is_err());
        }
        assert!(!world.contains(e));

        assert!(world.get::<&i32>(e).is_err());
    }

    #[should_panic]
    #[test]
    fn ergo_despawn_panic_active_borrow() {
        let mut world = World::new();
        let e = world.spawn((5i32, 1.5f32));
        {
            let ergo_scope = ErgoScope::new(&mut world);
            let component = ergo_scope.get::<i32>(e).expect("failed to get component");

            let component_borrow = component.read();
            ergo_scope.despawn(e).expect("failed to despawn entity");
            drop(component_borrow);
        }
    }

    #[test]
    fn ergo_reuse_refs() {
        let mut world = World::new();
        let e = world.spawn((5i32, 1.5f32));
        {
            let ergo_scope = ErgoScope::new(&mut world);
            let component = ergo_scope.get::<i32>(e).expect("failed to get component");

            ergo_scope
                .remove_one::<i32>(e)
                .expect("failed to remove component");
            ergo_scope
                .insert_one(e, 8i32)
                .expect("failed to insert component");
            assert_eq!(*component.read(), 8i32);
        }
    }

    #[should_panic]
    #[test]
    fn ergo_read_panic_removed_component() {
        let mut world = World::new();
        let e = world.spawn((5i32, 1.5f32));
        {
            let ergo_scope = ErgoScope::new(&mut world);
            let component = ergo_scope.get::<i32>(e).expect("failed to get component");

            ergo_scope
                .remove_one::<i32>(e)
                .expect("failed to remove component");
            assert_eq!(*component.read(), 5i32);
        }
    }

    #[test]
    fn ergo_spawn() {
        let mut world = World::new();
        let e = {
            let ergo_scope = ErgoScope::new(&mut world);
            let e = ergo_scope.spawn((5i32, 1.5f32));
            let component = ergo_scope.get::<i32>(e).expect("failed to get component");
            assert_eq!(*component.read(), 5i32);
            e
        };

        let c1 = world.get::<&i32>(e).expect("failed to get component");
        assert_eq!(*c1, 5i32);
        let c2 = world.get::<&f32>(e).expect("failed to get component");
        assert_eq!(*c2, 1.5f32);
    }
}
