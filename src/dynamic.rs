use std::any::{type_name, TypeId};
use std::sync::Mutex;

use fxhash::FxHashMap;

use crate::{AbstractStorage, Entities, Entity, Fetch, Masked, Storage, StorageRefMut};

/// A table mapping each `Entity` to one or more `Components` from a dynamic set
pub struct DynWorld {
    entities: Entities,
    storages: FxHashMap<TypeId, Mutex<Box<dyn AbstractStorage>>>,
}

impl DynWorld {
    pub fn new() -> Self {
        Self {
            entities: Entities::default(),
            storages: FxHashMap::default(),
        }
    }

    /// Add a new type of storage
    pub fn register<S: Storage>(&mut self) {
        if self.storages.contains_key(&TypeId::of::<S>()) {
            panic!("storage {} already registered", type_name::<S>());
        }
        self.storages.insert(
            TypeId::of::<S>(),
            Mutex::new(Box::new(Masked::new(S::default()))),
        );
    }

    /// Discard a type of storage, destroying its contents
    pub fn unregister<S: Storage>(&mut self) {
        self.storages.remove(&TypeId::of::<S>());
    }

    /// Access one or more storages
    pub fn get<'a, T>(&'a self) -> <&'a Self as Fetch<T>>::Ref
    where
        &'a Self: Fetch<T>,
    {
        self.fetch()
    }

    /// Create a new `Entity`
    pub fn spawn(&mut self) -> Entity {
        self.entities.spawn()
    }

    /// Whether `entity` currently exists
    pub fn contains(&self, entity: Entity) -> bool {
        self.entities.contains(entity)
    }

    /// Destroy an entity and all associated components
    ///
    /// Returns `false` iff the entity was previously destroyed
    pub fn despawn(&mut self, entity: Entity) -> bool {
        let was_live = self.entities.despawn(entity);
        if was_live {
            for storage in self.storages.values() {
                let mut storage = storage.try_lock().expect("storage already borrowed");
                storage.free(entity.index);
            }
        }
        was_live
    }

    /// Associate `component` with `entity`
    ///
    /// Returns `Some` if there was a pre-existing component for this entity in this storage.
    pub fn insert<S: Storage>(
        &self,
        entity: Entity,
        component: S::Component,
    ) -> Option<S::Component> {
        if !self.contains(entity) {
            return None;
        }
        self.get::<S>().insert(entity.index, component)
    }

    /// Remove `component` from `entity`
    ///
    /// Returns `none` iff no such component exists.
    pub fn remove<S: Storage>(&self, entity: Entity) -> Option<S::Component> {
        if !self.contains(entity) {
            return None;
        }
        self.get::<S>().remove(entity.index)
    }
}

impl<'a, T: Storage> Fetch<T> for &'a DynWorld {
    type Ref = StorageRefMut<'a, T>;
    fn fetch(self) -> StorageRefMut<'a, T> {
        let guard = self
            .storages
            .get(&TypeId::of::<T>())
            .unwrap_or_else(|| panic!("storage {} not registered", type_name::<T>()))
            .try_lock()
            .unwrap_or_else(|_| panic!("storage {} already borrowed", type_name::<T>()));
        StorageRefMut::new(guard)
    }
}
