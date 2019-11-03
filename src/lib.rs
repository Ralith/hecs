mod storage;

pub use storage::*;

use std::any::TypeId;
use std::sync::Mutex;

use fxhash::FxHashMap;
use hibitset::{BitSet, BitSetLike, BitSetNot};

#[derive(Clone, Copy, Debug, Hash, Eq, Ord, PartialEq, PartialOrd)]
pub struct Entity {
    generation: u32,
    index: u32,
}

pub struct World {
    entities: BitSet,
    generations: Vec<u32>,
    storages: FxHashMap<TypeId, Mutex<Box<dyn AbstractStorage>>>,
}

impl World {
    pub fn new() -> Self {
        Self {
            entities: BitSet::new(),
            generations: Vec::new(),
            storages: FxHashMap::default(),
        }
    }

    /// Add a new type of storage
    pub fn register<S: Storage>(&mut self) {
        dbg!(std::any::type_name::<S>());
        self.storages.insert(
            TypeId::of::<S>(),
            Mutex::new(Box::new(Masked::new(S::default()))),
        );
    }

    /// Discard a type of storage, destroying its contents
    pub fn unregister<S: Storage>(&mut self) {
        self.storages.remove(&TypeId::of::<S>());
    }

    /// Access a storage
    pub fn get<S: Storage>(&self) -> Option<StorageRefMut<'_, S>> {
        let guard = self.storages.get(&TypeId::of::<S>())?.lock().unwrap();
        Some(StorageRefMut::new(guard))
    }

    /// Create a new entity
    pub fn spawn(&mut self) -> Entity {
        let index = BitSetNot(&self.entities).iter().next().unwrap();
        self.entities.add(index);
        if index as usize >= self.generations.len() {
            self.generations.resize(index as usize + 1, 0);
        }
        let generation = self.generations[index as usize];
        Entity { generation, index }
    }

    /// Whether `entity` currently exists
    pub fn contains(&self, entity: Entity) -> bool {
        self.entities.contains(entity.index)
            && self.generations[entity.index as usize] == entity.generation
    }

    /// Destroy an entity and all associated components
    ///
    /// Returns `false` iff the entity was previously destroyed
    pub fn despawn(&mut self, entity: Entity) -> bool {
        if !self.contains(entity) {
            return false;
        }
        for storage in self.storages.values() {
            let mut storage = storage.lock().unwrap();
            storage.free(entity.index);
        }
        self.generations[entity.index as usize] =
            self.generations[entity.index as usize].wrapping_add(1);
        true
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
        self.get::<S>()
            .expect("no such storage")
            .insert(entity.index, component)
    }

    /// Remove `component` from `entity`
    ///
    /// Returns `none` iff no such component exists.
    pub fn remove<S: Storage>(&self, entity: Entity) -> Option<S::Component> {
        if !self.contains(entity) {
            return None;
        }
        self.get::<S>()
            .expect("no such storage")
            .remove(entity.index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke() {
        let mut world = World::new();
        world.register::<VecStorage<u32>>();
        let entity = world.spawn();
        world.insert::<VecStorage<u32>>(entity, 42);
        assert!(world.contains(entity));
        assert_eq!(world.remove::<VecStorage<u32>>(entity), Some(42));
        assert_eq!(world.remove::<VecStorage<u32>>(entity), None);
        assert!(world.despawn(entity));
        assert!(!world.contains(entity));
    }
}
