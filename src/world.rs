use std::alloc::{alloc, Layout};
use std::any::{type_name, TypeId};
use std::error::Error;
use std::mem::{self, MaybeUninit};
use std::{fmt, ptr};

use downcast_rs::{impl_downcast, Downcast};
use fxhash::FxHashMap;

use crate::archetype::{Archetype, TypeInfo};
use crate::borrow::{BorrowState, Ref, RefMut};
use crate::{EntityRef, Query, QueryIter};

/// An unordered collection of entities, each having any number of distinctly typed components
///
/// The components of entities who have the same set of component types are stored in contiguous
/// runs, allowing for extremely fast, cache-friendly iteration.
#[derive(Default)]
pub struct World {
    entities: Vec<EntityMeta>,
    free: Vec<u32>,
    archetypes: ArchetypeTable,
}

impl World {
    /// Create an empty world
    pub fn new() -> Self {
        Self::default()
    }

    /// Create an entity with certain components
    ///
    /// Returns the ID of the newly created entity.
    ///
    /// Arguments can be tuples or structs annotated with `#[derive(Bundle)]`. To spawn an entity
    /// with only one component, use a one-element tuple like `(x,)`.
    ///
    /// # Example
    /// ```
    /// # use hecs::*;
    /// let mut world = World::new();
    /// let a = world.spawn((123, "abc"));
    /// let b = world.spawn((456, true));
    /// ```
    pub fn spawn(&mut self, components: impl DynamicBundle) -> Entity {
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
        let archetype = components.get_archetype(&mut self.archetypes);
        self.entities[entity.id as usize].archetype = archetype;
        let archetype = &mut self.archetypes.archetypes[archetype as usize];
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
        if let Some(moved) =
            unsafe { self.archetypes.archetypes[meta.archetype as usize].remove(meta.index) }
        {
            self.entities[moved as usize].index = meta.index;
        }
        self.free.push(entity.id);
        Ok(())
    }

    /// Whether `entity` still exists
    pub fn contains(&self, entity: Entity) -> bool {
        self.entities[entity.id as usize].generation == entity.generation
    }

    /// Efficiently iterate over all entities that have certain components
    ///
    /// Yields `(Entity, Q)` tuples, where `Q` is some query type. A query type is `&T`, `&mut T`, a
    /// tuple of query types, or an `Option` wrapping a query type, where `T` is any component
    /// type. Components queried with `&mut` must only appear once. Entities which do not have a
    /// component type referenced outside of an `Option` will be skipped.
    ///
    /// Entities are yielded in arbitrary order.
    ///
    /// # Example
    /// ```
    /// # use hecs::*;
    /// let mut world = World::new();
    /// let a = world.spawn((123, true, "abc"));
    /// let b = world.spawn((456, false));
    /// let c = world.spawn((42, "def"));
    /// let entities = world.query::<(&i32, &bool)>().collect::<Vec<_>>();
    /// assert_eq!(entities.len(), 2);
    /// assert!(entities.contains(&(a, (&123, &true))));
    /// assert!(entities.contains(&(b, (&456, &false))));
    /// ```
    pub fn query<'a, Q: Query<'a>>(&'a self) -> QueryIter<'a, Q> {
        QueryIter::new(
            &self.archetypes.borrows,
            &self.entities,
            &self.archetypes.archetypes,
        )
    }

    /// Borrow the `T` component of `entity`
    ///
    /// Panics if the entity has no such component or the component is already uniquely borrowed.
    pub fn get<T: Component>(&self, entity: Entity) -> Result<Ref<'_, T>, NoSuchEntity> {
        let meta = &self.entities[entity.id as usize];
        if meta.generation != entity.generation {
            return Err(NoSuchEntity);
        }
        unsafe {
            Ok(Ref::new(
                &self.archetypes.borrows,
                self.archetypes.archetypes[meta.archetype as usize]
                    .get(meta.index)
                    .unwrap_or_else(|| panic!("entity has no {} component", type_name::<T>())),
            ))
        }
    }

    /// Uniquely borrow the `T` component of `entity`
    ///
    /// Panics if the entity has no such component or the component is already borrowed.
    pub fn get_mut<T: Component>(&self, entity: Entity) -> Result<RefMut<'_, T>, NoSuchEntity> {
        let meta = &self.entities[entity.id as usize];
        if meta.generation != entity.generation {
            return Err(NoSuchEntity);
        }
        unsafe {
            Ok(RefMut::new(
                &self.archetypes.borrows,
                self.archetypes.archetypes[meta.archetype as usize]
                    .get(meta.index)
                    .unwrap_or_else(|| panic!("entity has no {} component", type_name::<T>())),
            ))
        }
    }

    /// Access an entity regardless of its component types
    ///
    /// Does not immediately borrow any component.
    pub fn entity(&self, entity: Entity) -> Result<EntityRef<'_>, NoSuchEntity> {
        let meta = &self.entities[entity.id as usize];
        if meta.generation != entity.generation {
            return Err(NoSuchEntity);
        }
        Ok(EntityRef::new(
            &self.archetypes.borrows,
            &self.archetypes.archetypes[meta.archetype as usize],
            meta.index,
        ))
    }

    /// Iterate over all entities in the world
    ///
    /// Entities are yielded in arbitrary order. See also `World::query`.
    ///
    /// ```
    /// # use hecs::*;
    /// let mut world = World::new();
    /// let a = world.spawn(());
    /// let b = world.spawn(());
    /// assert_eq!(world.iter().map(|(id, _)| id).collect::<Vec<_>>(), &[a, b]);
    /// ```
    pub fn iter(&self) -> Iter<'_> {
        Iter::new(
            &self.archetypes.borrows,
            &self.archetypes.archetypes,
            &self.entities,
        )
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
            let mut info = self.archetypes.archetypes[meta.archetype as usize]
                .types()
                .to_vec();
            info.push(TypeInfo::of::<T>());
            let elements = info.iter().map(|x| x.id()).collect::<Vec<_>>();
            let target = match self.archetypes.index.entry(elements) {
                Entry::Occupied(x) => *x.get(),
                Entry::Vacant(x) => {
                    self.archetypes.borrows.ensure(TypeId::of::<T>());
                    self.archetypes.archetypes.push(Archetype::new(info));
                    let index = (self.archetypes.archetypes.len() - 1) as u32;
                    x.insert(index);
                    index
                }
            };
            if target == meta.archetype {
                *self.archetypes.archetypes[meta.archetype as usize]
                    .get(meta.index)
                    .expect("corrupt archetype index")
                    .as_mut() = component;
            } else {
                let (source_arch, target_arch) = index2(
                    &mut self.archetypes.archetypes,
                    meta.archetype as usize,
                    target as usize,
                );
                let components = source_arch.move_component_set(meta.index);
                meta.archetype = target;
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
            let info = self.archetypes.archetypes[meta.archetype as usize]
                .types()
                .iter()
                .cloned()
                .filter(|x| x.id() != TypeId::of::<T>())
                .collect::<Vec<_>>();
            let elements = info.iter().map(|x| x.id()).collect::<Vec<_>>();
            let target = match self.archetypes.index.entry(elements) {
                Entry::Occupied(x) => *x.get(),
                Entry::Vacant(x) => {
                    self.archetypes.archetypes.push(Archetype::new(info));
                    let index = (self.archetypes.archetypes.len() - 1) as u32;
                    x.insert(index);
                    index
                }
            };
            let (source_arch, target_arch) = index2(
                &mut self.archetypes.archetypes,
                meta.archetype as usize,
                target as usize,
            );
            let x = source_arch.take::<T>(meta.index);
            let components = source_arch.move_component_set(meta.index);
            meta.archetype = target;
            meta.index = target_arch.allocate(entity.id);
            components.store(target_arch, meta.index);
            Ok(x)
        }
    }
}

unsafe impl Sync for World {}

impl<'a> IntoIterator for &'a World {
    type IntoIter = Iter<'a>;
    type Item = (Entity, EntityRef<'a>);
    fn into_iter(self) -> Iter<'a> {
        self.iter()
    }
}

/// Storage indexed by component types
#[derive(Default)]
pub struct ArchetypeTable {
    index: FxHashMap<Vec<TypeId>, u32>,
    archetypes: Vec<Archetype>,
    borrows: BorrowState,
}

impl ArchetypeTable {
    /// Get the archetype ID for a set of component types
    ///
    /// `tys` must be sorted by alignment descending, then id.
    pub fn get_id(&mut self, tys: &[TypeId]) -> Option<u32> {
        self.index.get(tys).copied()
    }

    /// Create a new archetype
    ///
    /// `get_id` for these types must have returned `None`. `info` must be sorted.
    pub fn alloc(&mut self, info: Vec<TypeInfo>) -> u32 {
        debug_assert!(
            self.index
                .get(&info.iter().map(|x| x.id()).collect::<Vec<_>>())
                .is_none(),
            "archetype already exists"
        );
        for ty in &info {
            self.borrows.ensure(ty.id());
        }
        let x = self.archetypes.len() as u32;
        self.index
            .insert(info.iter().map(|x| x.id()).collect(), x)
            .map(|_| panic!("duplicate archetype"));
        self.archetypes.push(Archetype::new(info));
        x
    }
}

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
///
/// Obtained from `World::spawn`. Can be stored to refer to an entity in the future.
#[derive(Clone, Copy, Debug, Hash, Eq, Ord, PartialEq, PartialOrd)]
pub struct Entity {
    pub(crate) generation: u32,
    pub(crate) id: u32,
}

/// A statically typed collection of components
pub trait Bundle: DynamicBundle {
    /// The components in the collection
    ///
    /// Must be sorted by alignment descending, then id.
    #[doc(hidden)]
    fn elements() -> &'static [TypeId];
}

/// A collection of components
pub trait DynamicBundle {
    #[doc(hidden)]
    fn get_archetype(&self, table: &mut ArchetypeTable) -> u32;
    #[doc(hidden)]
    unsafe fn store(self, archetype: &mut Archetype, index: u32);
}

/// Helper for incrementally constructing an entity with dynamic component types
///
/// Can be reused efficiently.
///
/// ```
/// # use hecs::*;
/// let mut world = World::new();
/// let mut builder = EntityBuilder::new();
/// builder.add(123).add("abc");
/// let e = world.spawn(builder.build());
/// assert_eq!(*world.get::<i32>(e).unwrap(), 123);
/// assert_eq!(*world.get::<&str>(e).unwrap(), "abc");
/// ```
pub struct EntityBuilder {
    storage: Box<[MaybeUninit<u8>]>,
    // Backwards from the end!
    cursor: *mut u8,
    max_align: usize,
    info: Vec<(TypeInfo, *mut u8)>,
    ids: Vec<TypeId>,
}

impl EntityBuilder {
    /// Create a builder representing an entity with no components
    pub fn new() -> Self {
        Self {
            storage: Box::new([]),
            cursor: ptr::null_mut(),
            max_align: 16,
            info: Vec::new(),
            ids: Vec::new(),
        }
    }

    /// Add `component` to the entity
    pub fn add<T: Component>(&mut self, component: T) -> &mut Self {
        self.max_align = self.max_align.max(mem::align_of::<T>());
        if (self.cursor as usize) < mem::size_of::<T>() {
            self.grow(mem::size_of::<T>());
        }
        unsafe {
            self.cursor = (self.cursor.sub(mem::size_of::<T>()) as usize
                & !(mem::align_of::<T>() - 1)) as *mut u8;
            if self.cursor.cast() < self.storage.as_mut_ptr() {
                self.grow(mem::size_of::<T>());
                self.cursor = (self.cursor.sub(mem::size_of::<T>()) as usize
                    & !(mem::align_of::<T>() - 1)) as *mut u8;
            }
            ptr::write(self.cursor.cast::<T>(), component);
        }
        self.info.push((TypeInfo::of::<T>(), self.cursor));
        self
    }

    fn grow(&mut self, min_increment: usize) {
        let new_len = (self.storage.len() + min_increment)
            .next_power_of_two()
            .max(self.storage.len() * 2)
            .max(64);
        unsafe {
            let alloc = alloc(Layout::from_size_align(new_len, self.max_align).unwrap())
                .cast::<MaybeUninit<u8>>();
            let mut new_storage = Box::from_raw(std::slice::from_raw_parts_mut(alloc, new_len));
            new_storage[new_len - self.storage.len()..].copy_from_slice(&self.storage);
            self.cursor = new_storage
                .as_mut_ptr()
                .add(new_len - self.storage.len())
                .cast();
            self.storage = new_storage;
        }
    }

    /// Construct a `DynamicBundle` suitable for spawning
    pub fn build(&mut self) -> BuiltEntity<'_> {
        self.info.sort_unstable_by(|x, y| x.0.cmp(&y.0));
        self.ids.clear();
        self.ids.extend(self.info.iter().map(|x| x.0.id()));
        BuiltEntity { builder: self }
    }
}

unsafe impl Send for EntityBuilder {}
unsafe impl Sync for EntityBuilder {}

/// The output of an `EntityBuilder`, suitable for passing to `World::spawn`
pub struct BuiltEntity<'a> {
    builder: &'a mut EntityBuilder,
}

impl DynamicBundle for BuiltEntity<'_> {
    fn get_archetype(&self, table: &mut ArchetypeTable) -> u32 {
        table
            .get_id(&self.builder.ids)
            .unwrap_or_else(|| table.alloc(self.builder.info.iter().map(|x| x.0).collect()))
    }

    unsafe fn store(self, archetype: &mut Archetype, index: u32) {
        for (ty, component) in self.builder.info.drain(..) {
            archetype.put_dynamic(component, ty.id(), ty.layout(), index);
        }
    }
}

impl Drop for BuiltEntity<'_> {
    fn drop(&mut self) {
        for (ty, component) in self.builder.info.drain(..) {
            unsafe {
                ty.drop(component);
            }
        }
    }
}

/// Iterator over all of a world's entities
pub struct Iter<'a> {
    borrows: &'a BorrowState,
    archetypes: std::slice::Iter<'a, Archetype>,
    entities: &'a [EntityMeta],
    current: Option<&'a Archetype>,
    index: u32,
}

impl<'a> Iter<'a> {
    fn new(
        borrows: &'a BorrowState,
        archetypes: &'a [Archetype],
        entities: &'a [EntityMeta],
    ) -> Self {
        Self {
            borrows,
            archetypes: archetypes.iter(),
            entities,
            current: None,
            index: 0,
        }
    }
}

impl<'a> Iterator for Iter<'a> {
    type Item = (Entity, EntityRef<'a>);
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.current {
                None => {
                    self.current = Some(self.archetypes.next()?);
                    self.index = 0;
                }
                Some(current) => {
                    if self.index == current.len() as u32 {
                        self.current = None;
                        continue;
                    }
                    let index = self.index;
                    self.index += 1;
                    let id = current.entity_id(index);
                    return Some((
                        Entity {
                            id,
                            generation: self.entities[id as usize].generation,
                        },
                        EntityRef::new(self.borrows, current, index),
                    ));
                }
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(self.entities.len()))
    }
}

macro_rules! tuple_impl {
    ($($name: ident),*) => {
        impl<$($name: Component),*> DynamicBundle for ($($name,)*) {
            fn get_archetype(&self, table: &mut ArchetypeTable) -> u32 {
                const N: usize = count!($($name),*);
                let mut xs: [TypeInfo; N] = [$(TypeInfo::of::<$name>()),*];
                xs.sort_unstable();
                let mut ids = [TypeId::of::<()>(); N];
                for (id, info) in ids.iter_mut().zip(xs.iter()) {
                    *id = info.id();
                }
                table.get_id(&ids).unwrap_or_else(|| table.alloc(xs.to_vec()))
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

macro_rules! count {
    () => { 0 };
    ($x: ident $(, $rest: ident)*) => { 1 + count!($($rest),*) };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_reuse() {
        let mut world = World::new();
        let a = world.spawn(());
        world.despawn(a).unwrap();
        let b = world.spawn(());
        assert_eq!(a.id, b.id);
        assert_ne!(a.generation, b.generation);
    }
}
