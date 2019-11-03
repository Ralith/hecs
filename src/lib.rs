mod storage;

pub use storage::*;

use std::any::{type_name, TypeId};
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
    pub fn get<'a, T: Fetch<'a>>(&'a self) -> T::Ref {
        T::fetch(self)
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
            let mut storage = storage.try_lock().expect("storage already borrowed");
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

pub trait Fetch<'a> {
    type Ref;
    fn fetch(world: &'a World) -> Self::Ref;
}

impl<'a, T: Storage> Fetch<'a> for T {
    type Ref = StorageRefMut<'a, T>;
    fn fetch(world: &'a World) -> StorageRefMut<'a, T> {
        let guard = world
            .storages
            .get(&TypeId::of::<T>())
            .unwrap_or_else(|| panic!("storage {} not registered", type_name::<T>()))
            .try_lock()
            .unwrap_or_else(|_| panic!("storage {} already borrowed", type_name::<T>()));
        StorageRefMut::new(guard)
    }
}

macro_rules! tuple_impl {
    ($($name: ident),*) => {
        impl<'a, $($name: Fetch<'a>),*> Fetch<'a> for ($($name),*) {
            type Ref = ($(<$name as Fetch<'a>>::Ref),*);
            fn fetch(world: &'a World) -> Self::Ref {
                ($($name::fetch(world)),*)
            }
        }
    }
}

tuple_impl!(A, B);
tuple_impl!(A, B, C);
tuple_impl!(A, B, C, D);
tuple_impl!(A, B, C, D, E);
tuple_impl!(A, B, C, D, E, F);
tuple_impl!(A, B, C, D, E, F, G);
tuple_impl!(A, B, C, D, E, F, G, H);
tuple_impl!(A, B, C, D, E, F, G, H, I);
tuple_impl!(A, B, C, D, E, F, G, H, I, J);
tuple_impl!(A, B, C, D, E, F, G, H, I, J, K);
tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L);
tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M);
tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N);
tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke() {
        let mut world = World::new();
        world.register::<VecStorage<u32>>();
        world.register::<VecStorage<u16>>();
        let entity = world.spawn();
        world.insert::<VecStorage<u32>>(entity, 32);
        world.insert::<VecStorage<u16>>(entity, 16);

        {
            let s = world.get::<VecStorage<u32>>();
            assert_eq!(s.iter().cloned().collect::<Vec<_>>(), [32]);

            assert_eq!((&s, &s, &s).join().collect::<Vec<_>>(), [(&32, &32, &32)]);
        }

        {
            let (s, mut t) = world.get::<(VecStorage<u32>, VecStorage<u16>)>();
            assert_eq!((&s, &mut t).join().collect::<Vec<_>>(), [(&32, &mut 16)]);
        }

        assert!(world.contains(entity));
        assert_eq!(world.remove::<VecStorage<u32>>(entity), Some(32));
        assert_eq!(world.remove::<VecStorage<u32>>(entity), None);
        assert!(world.despawn(entity));
        assert!(!world.contains(entity));
    }

    #[test]
    #[should_panic(expected = "already borrowed")]
    fn double_borrow() {
        let mut world = World::new();
        world.register::<VecStorage<u32>>();
        world.get::<(VecStorage<u32>, VecStorage<u32>)>();
    }
}
