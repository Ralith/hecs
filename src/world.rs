use std::any::TypeId;
use std::error::Error;
use std::fmt;

use downcast_rs::{impl_downcast, Downcast};
use fxhash::FxHashMap;

use crate::archetype::{Archetype, TypeInfo};
use crate::{Query, QueryIter};

/// An unordered collection of entities, each having zero or more distinctly typed components
#[derive(Default)]
pub struct World {
    entities: Vec<EntityMeta>,
    free: Vec<u32>,
    archetypes: Vec<Archetype>,
    archetype_index: FxHashMap<Vec<TypeId>, usize>,
}

impl World {
    /// Create an empty world
    pub fn new() -> Self {
        Self::default()
    }

    /// Create an entity with certain components
    ///
    /// Returns the ID of the newly created entity
    pub fn spawn(&mut self, components: impl ComponentSet) -> Entity {
        use std::collections::hash_map::Entry;

        let entity = match self.free.pop() {
            Some(i) => Entity {
                generation: self.entities[i as usize].generation,
                id: i,
            },
            None => {
                let i = self.entities.len() as u32;
                self.entities.push(EntityMeta {
                    generation: 0,
                    archetype: 0,
                    index: 0,
                });
                Entity {
                    generation: 0,
                    id: i,
                }
            }
        };
        let archetype = match self.archetype_index.entry(components.elements()) {
            Entry::Occupied(x) => *x.get(),
            Entry::Vacant(x) => {
                self.archetypes.push(Archetype::new(components.info()));
                let index = self.archetypes.len() - 1;
                x.insert(index);
                index
            }
        };
        self.entities[entity.id as usize].archetype = archetype as u32;
        let archetype = &mut self.archetypes[archetype];
        unsafe {
            let index = archetype.allocate(entity.id);
            self.entities[entity.id as usize].index = index;
            components.store(archetype, index);
        }
        entity
    }

    /// Destroy an entity and all its components
    ///
    /// Returns false iff the entity was already destroyed.
    pub fn despawn(&mut self, entity: Entity) -> Result<(), NoSuchEntity> {
        let meta = &mut self.entities[entity.id as usize];
        if meta.generation != entity.generation {
            return Err(NoSuchEntity);
        }
        meta.generation += 1;
        if let Some(moved) = unsafe { self.archetypes[meta.archetype as usize].remove(meta.index) }
        {
            self.entities[moved as usize].index = meta.index;
        }
        Ok(())
    }

    /// Get the `T` component of `entity`
    pub fn get<T: Component>(&self, entity: Entity) -> Result<&T, NoSuchEntity> {
        let meta = &self.entities[entity.id as usize];
        if meta.generation != entity.generation {
            return Err(NoSuchEntity);
        }
        unsafe { Ok(self.archetypes[meta.archetype as usize].get(meta.index)) }
    }

    /// Get the `T` component of `entity` mutably
    pub fn get_mut<T: Component>(&mut self, entity: Entity) -> Result<&mut T, NoSuchEntity> {
        let meta = &self.entities[entity.id as usize];
        if meta.generation != entity.generation {
            return Err(NoSuchEntity);
        }
        unsafe { Ok(self.archetypes[meta.archetype as usize].get_mut(meta.index)) }
    }

    /// Access certain components from all entities
    ///
    /// Entities are yielded in arbitrary order.
    pub fn iter<'a, Q: Query<'a>>(&'a mut self) -> QueryIter<'a, Q> {
        QueryIter::new(&self.entities, &mut self.archetypes)
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct NoSuchEntity;

impl fmt::Display for NoSuchEntity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad("no such entity")
    }
}

impl Error for NoSuchEntity {}

/// Types that can be components (implemented automatically)
pub trait Component: Downcast + Send + Sync + 'static {}
impl_downcast!(Component);
impl<T: Send + Sync + 'static> Component for T {}

pub(crate) struct EntityMeta {
    pub(crate) generation: u32,
    archetype: u32,
    index: u32,
}

/// Lightweight unique ID of an entity
#[derive(Clone, Copy, Debug, Hash, Eq, Ord, PartialEq, PartialOrd)]
pub struct Entity {
    pub(crate) generation: u32,
    pub(crate) id: u32,
}

/// A collection of distinctly typed values that can be used to create an entity
pub trait ComponentSet {
    // Future work: Reduce heap allocation, redundant sorting
    fn elements(&self) -> Vec<TypeId>;
    fn info(&self) -> Vec<TypeInfo>;
    unsafe fn store(self, archetype: &mut Archetype, index: u32);
}

/// Helper for incrementally constructing an entity with dynamic component types
#[derive(Default)]
pub struct EntityBuilder {
    components: Vec<Box<dyn Component>>,
    types: Vec<TypeInfo>,
}

impl EntityBuilder {
    /// Create a builder representing an entity with no components
    pub fn new() -> Self {
        Self::default()
    }

    /// Add `component` to the entity
    pub fn with<T: Component>(&mut self, component: T) -> &mut Self {
        self.components.push(Box::new(component));
        self.types.push(TypeInfo::of::<T>());
        self
    }

    /// Prepare for spawning
    pub fn build(mut self) -> BuiltEntity {
        self.types.sort_unstable();
        BuiltEntity { inner: self }
    }
}

/// The output of an `EntityBuilder`, suitable for passing to `World::spawn`
pub struct BuiltEntity {
    inner: EntityBuilder,
}

impl ComponentSet for BuiltEntity {
    fn elements(&self) -> Vec<TypeId> {
        self.inner.types.iter().map(|x| x.id()).collect()
    }
    fn info(&self) -> Vec<TypeInfo> {
        self.inner.types.clone()
    }
    unsafe fn store(self, archetype: &mut Archetype, index: u32) {
        for (component, info) in self
            .inner
            .components
            .into_iter()
            .zip(self.inner.types.into_iter())
        {
            let component = Box::into_raw(component) as *mut u8;
            archetype.put_dynamic(component, info.id(), info.layout(), index);
            std::alloc::dealloc(component, info.layout());
        }
    }
}

macro_rules! tuple_impl {
    ($($name: ident),*) => {
        impl<$($name: Component),*> ComponentSet for ($($name,)*) {
            fn elements(&self) -> Vec<TypeId> {
                self.info().into_iter().map(|x| x.id()).collect()
            }
            fn info(&self) -> Vec<TypeInfo> {
                let mut xs = vec![$(TypeInfo::of::<$name>()),*];
                xs.sort_unstable();
                xs
            }
            unsafe fn store(self, archetype: &mut Archetype, index: u32) {
                #[allow(non_snake_case)]
                let ($($name,)*) = self;
                $(
                    archetype.put($name, index);
                )*
            }
        }
    }
}

tuple_impl!(A);
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
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z, AA);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z, AA, AB);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z, AA, AB, AC);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z, AA, AB, AC, AD);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z, AA, AB, AC, AD, AE);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z, AA, AB, AC, AD, AE, AF);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z, AA, AB, AC, AD, AE, AF, AG);
