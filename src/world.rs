use std::any::{type_name, TypeId};
use std::error::Error;
use std::fmt;

use downcast_rs::{impl_downcast, Downcast};
use fxhash::FxHashMap;

use crate::archetype::{Archetype, TypeInfo};
use crate::borrow::{BorrowState, Ref, RefMut};
use crate::{EntityRef, Query, QueryIter};

/// An unordered collection of entities, each having zero or more distinctly typed components
///
/// The components of entities who have the same set of component types are stored in contiguous
/// runs, allowing for extremely fast, cache-friendly iteration.
#[derive(Default)]
pub struct World {
    entities: Vec<EntityMeta>,
    free: Vec<u32>,
    archetypes: Vec<Archetype>,
    archetype_index: FxHashMap<Vec<TypeId>, usize>,
    borrows: BorrowState,
}

impl World {
    /// Create an empty world
    pub fn new() -> Self {
        Self::default()
    }

    /// Create an entity with certain components
    ///
    /// Returns the ID of the newly created entity
    ///
    /// # Example
    /// ```
    /// # use hecs::*;
    /// let mut world = World::new();
    /// let a = world.spawn((123, "abc"));
    /// let b = world.spawn((456, true));
    /// ```
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
                let info = components.info();
                for ty in &info {
                    self.borrows.ensure(ty.id());
                }
                self.archetypes.push(Archetype::new(info));
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

    /// Access certain components from all entities
    ///
    /// Yields `(Entity, Q)` tuples. `Q` can be a shared or unique reference to a component type, an
    /// `Option` wrapping such a reference, or a tuple of other query types. Components queried with
    /// `&mut` must only appear once. Entities which do not have a component type referenced outside
    /// of an `Option` will be skipped.
    ///
    /// Entities are yielded in arbitrary order.
    ///
    /// # Example
    /// ```
    /// # use hecs::*;
    /// let mut world = World::new();
    /// let a = world.spawn((123, true, "abc"));
    /// let b = world.spawn((456, false));
    /// let entities = world.query::<(&i32, &bool)>().collect::<Vec<_>>();
    /// assert_eq!(entities.len(), 2);
    /// assert!(entities.contains(&(a, (&123, &true))));
    /// assert!(entities.contains(&(b, (&456, &false))));
    /// ```
    pub fn query<'a, Q: Query<'a>>(&'a self) -> QueryIter<'a, Q> {
        QueryIter::new(&self.borrows, &self.entities, &self.archetypes)
    }

    /// Get the `T` component of `entity`
    ///
    /// Panics if the entity has no such component
    pub fn get<T: Component>(&self, entity: Entity) -> Result<Ref<'_, T>, NoSuchEntity> {
        let meta = &self.entities[entity.id as usize];
        if meta.generation != entity.generation {
            return Err(NoSuchEntity);
        }
        unsafe {
            Ok(Ref::new(
                &self.borrows,
                self.archetypes[meta.archetype as usize]
                    .get(meta.index)
                    .unwrap_or_else(|| panic!("entity has no {} component", type_name::<T>())),
            ))
        }
    }

    /// Get the `T` component of `entity` mutably
    pub fn get_mut<T: Component>(&self, entity: Entity) -> Result<RefMut<'_, T>, NoSuchEntity> {
        let meta = &self.entities[entity.id as usize];
        if meta.generation != entity.generation {
            return Err(NoSuchEntity);
        }
        unsafe {
            Ok(RefMut::new(
                &self.borrows,
                self.archetypes[meta.archetype as usize]
                    .get(meta.index)
                    .unwrap_or_else(|| panic!("entity has no {} component", type_name::<T>())),
            ))
        }
    }

    /// Access an entity regardless of its component types
    pub fn entity(&self, entity: Entity) -> Result<EntityRef<'_>, NoSuchEntity> {
        let meta = &self.entities[entity.id as usize];
        if meta.generation != entity.generation {
            return Err(NoSuchEntity);
        }
        Ok(EntityRef::new(
            &self.borrows,
            &self.archetypes[meta.archetype as usize],
            meta.index,
        ))
    }

    /// Add `component` to `entity`
    ///
    /// Computational cost is proportional to the number of components `entity` has.
    pub fn insert<T: Component>(
        &mut self,
        entity: Entity,
        component: T,
    ) -> Result<(), NoSuchEntity> {
        use std::collections::hash_map::Entry;

        let meta = &mut self.entities[entity.id as usize];
        if meta.generation != entity.generation {
            return Err(NoSuchEntity);
        }
        unsafe {
            let mut info = self.archetypes[meta.archetype as usize].types().to_vec();
            info.push(TypeInfo::of::<T>());
            let elements = info.iter().map(|x| x.id()).collect::<Vec<_>>();
            let target = match self.archetype_index.entry(elements) {
                Entry::Occupied(x) => *x.get(),
                Entry::Vacant(x) => {
                    self.borrows.ensure(TypeId::of::<T>());
                    self.archetypes.push(Archetype::new(info));
                    let index = self.archetypes.len() - 1;
                    x.insert(index);
                    index
                }
            };
            if target == meta.archetype as usize {
                *self.archetypes[meta.archetype as usize]
                    .get(meta.index)
                    .expect("corrupt archetype index")
                    .as_mut() = component;
            } else {
                let (source_arch, target_arch) =
                    index2(&mut self.archetypes, meta.archetype as usize, target);
                let components = source_arch.move_component_set(meta.index);
                meta.archetype = target as u32;
                meta.index = target_arch.allocate(entity.id);
                components.store(target_arch, meta.index);
                target_arch.put(component, meta.index);
            }
        }
        Ok(())
    }

    /// Remove the `T` component from `entity`
    ///
    /// Computational cost is proportional to the number of components `entity` has. Returns the
    /// removed component in `Some` if the entity is live and had a `T` component.
    pub fn remove<T: Component>(&mut self, entity: Entity) -> Result<T, NoSuchEntity> {
        use std::collections::hash_map::Entry;

        let meta = &mut self.entities[entity.id as usize];
        if meta.generation != entity.generation {
            return Err(NoSuchEntity);
        }
        unsafe {
            let info = self.archetypes[meta.archetype as usize]
                .types()
                .iter()
                .cloned()
                .filter(|x| x.id() != TypeId::of::<T>())
                .collect::<Vec<_>>();
            let elements = info.iter().map(|x| x.id()).collect::<Vec<_>>();
            let target = match self.archetype_index.entry(elements) {
                Entry::Occupied(x) => *x.get(),
                Entry::Vacant(x) => {
                    self.archetypes.push(Archetype::new(info));
                    let index = self.archetypes.len() - 1;
                    x.insert(index);
                    index
                }
            };
            let (source_arch, target_arch) =
                index2(&mut self.archetypes, meta.archetype as usize, target);
            let x = source_arch.read::<T>(meta.index);
            let components = source_arch.move_component_set(meta.index);
            meta.archetype = target as u32;
            meta.index = target_arch.allocate(entity.id);
            components.store(target_arch, meta.index);
            Ok(x)
        }
    }
}

unsafe impl Sync for World {}

fn index2<T>(x: &mut [T], i: usize, j: usize) -> (&mut T, &mut T) {
    assert!(i != j);
    assert!(i < x.len());
    assert!(j < x.len());
    let ptr = x.as_mut_ptr();
    unsafe { (&mut *ptr.add(i), &mut *ptr.add(j)) }
}

/// Error indicating that no entity with a particular ID exists
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
    #[doc(hidden)]
    fn elements(&self) -> Vec<TypeId>;
    #[doc(hidden)]
    fn info(&self) -> Vec<TypeInfo>;
    #[doc(hidden)]
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
            #[allow(unused_variables)]
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

tuple_impl!();
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
