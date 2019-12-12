// Copyright 2019 Google LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::any::TypeId;
use std::error::Error;
use std::{fmt, ptr};

use fxhash::{FxHashMap, FxHashSet};

use crate::archetype::Archetype;
use crate::borrow::BorrowState;
use crate::{Bundle, DynamicBundle, EntityRef, MissingComponent, Query, QueryIter, Ref, RefMut};

/// An unordered collection of entities, each having any number of distinctly typed components
///
/// Similar to `HashMap<Entity, Vec<Box<dyn Any>>>` where each `Vec` never contains two of the same
/// type, but far more efficient to traverse.
///
/// The components of entities who have the same set of component types are stored in contiguous
/// runs, allowing for extremely fast, cache-friendly iteration.
#[derive(Default)]
pub struct World {
    entities: Entities,
    index: FxHashMap<Vec<TypeId>, u32>,
    archetypes: Vec<Archetype>,
    borrows: BorrowState,
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
    /// Arguments can be tuples, structs annotated with `#[derive(Bundle)]`, or `EntityBuilder`,
    /// preferred if the set of components isn't statically known. To spawn an entity with only one
    /// component, use a one-element tuple like `(x,)`.
    ///
    /// # Example
    /// ```
    /// # use hecs::*;
    /// let mut world = World::new();
    /// let a = world.spawn((123, "abc"));
    /// let b = world.spawn((456, true));
    /// ```
    pub fn spawn(&mut self, components: impl DynamicBundle) -> Entity {
        let entity = self.entities.alloc();
        let archetype = components.with_ids(|ids| {
            self.index.get(ids).copied().unwrap_or_else(|| {
                for &id in ids {
                    self.borrows.ensure(id);
                }
                let x = self.archetypes.len() as u32;
                self.archetypes.push(Archetype::new(components.type_info()));
                self.index.insert(ids.to_vec(), x);
                x
            })
        });
        self.entities.meta[entity.id as usize].location.archetype = archetype;
        let archetype = &mut self.archetypes[archetype as usize];
        unsafe {
            let index = archetype.allocate(entity.id);
            self.entities.meta[entity.id as usize].location.index = index;
            components.put(|ptr, ty, size| {
                archetype.put_dynamic(ptr, ty, size, index);
                true
            });
        }
        entity
    }

    /// Destroy an entity and all its components
    pub fn despawn(&mut self, entity: Entity) -> Result<(), NoSuchEntity> {
        let loc = self.entities.free(entity)?;
        if let Some(moved) = unsafe { self.archetypes[loc.archetype as usize].remove(loc.index) } {
            self.entities.meta[moved as usize].location.index = loc.index;
        }
        Ok(())
    }

    /// Despawn all entities
    ///
    /// Preserves allocated storage for reuse.
    pub fn clear(&mut self) {
        for x in &mut self.archetypes {
            x.clear();
        }
        self.entities.clear();
    }

    /// Whether `entity` still exists
    pub fn contains(&self, entity: Entity) -> bool {
        self.entities.meta[entity.id as usize].generation == entity.generation
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
    /// Query types can also be constructed with `#[derive(Query)]` on a struct whose fields all
    /// have query types.
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
        QueryIter::new(&self.borrows, &self.entities.meta, &self.archetypes)
    }

    /// Borrow the `T` component of `entity`
    ///
    /// Panics if the component is already uniquely borrowed.
    pub fn get<T: Component>(&self, entity: Entity) -> Result<Ref<'_, T>, ComponentError> {
        let meta = &self.entities.meta[entity.id as usize];
        if meta.generation != entity.generation {
            return Err(ComponentError::NoSuchEntity);
        }
        unsafe {
            Ok(Ref::new(
                &self.borrows,
                self.archetypes[meta.location.archetype as usize]
                    .get(meta.location.index)
                    .ok_or_else(MissingComponent::new::<T>)?,
            ))
        }
    }

    /// Uniquely borrow the `T` component of `entity`
    ///
    /// Panics if the component is already borrowed.
    pub fn get_mut<T: Component>(&self, entity: Entity) -> Result<RefMut<'_, T>, ComponentError> {
        let meta = &self.entities.meta[entity.id as usize];
        if meta.generation != entity.generation {
            return Err(ComponentError::NoSuchEntity);
        }
        unsafe {
            Ok(RefMut::new(
                &self.borrows,
                self.archetypes[meta.location.archetype as usize]
                    .get(meta.location.index)
                    .ok_or_else(MissingComponent::new::<T>)?,
            ))
        }
    }

    /// Access an entity regardless of its component types
    ///
    /// Does not immediately borrow any component.
    pub fn entity(&self, entity: Entity) -> Result<EntityRef<'_>, NoSuchEntity> {
        let meta = &self.entities.meta[entity.id as usize];
        if meta.generation != entity.generation {
            return Err(NoSuchEntity);
        }
        Ok(EntityRef::new(
            &self.borrows,
            &self.archetypes[meta.location.archetype as usize],
            meta.location.index,
        ))
    }

    /// Iterate over all entities in the world
    ///
    /// Entities are yielded in arbitrary order. Prefer `World::query` for better performance when
    /// components will be accessed in predictable patterns.
    ///
    /// # Example
    /// ```
    /// # use hecs::*;
    /// let mut world = World::new();
    /// let a = world.spawn(());
    /// let b = world.spawn(());
    /// assert_eq!(world.iter().map(|(id, _)| id).collect::<Vec<_>>(), &[a, b]);
    /// ```
    pub fn iter(&self) -> Iter<'_> {
        Iter::new(&self.borrows, &self.archetypes, &self.entities.meta)
    }

    /// Add `components` to `entity`
    ///
    /// Computational cost is proportional to the number of components `entity` has. If an entity
    /// already has a component of a certain type, it is dropped and replaced.
    ///
    /// # Example
    /// ```
    /// # use hecs::*;
    /// let mut world = World::new();
    /// let e = world.spawn((123, "abc"));
    /// world.insert(e, (456, true));
    /// assert_eq!(*world.get::<i32>(e).unwrap(), 456);
    /// assert_eq!(*world.get::<bool>(e).unwrap(), true);
    /// ```
    pub fn insert(
        &mut self,
        entity: Entity,
        components: impl DynamicBundle,
    ) -> Result<(), NoSuchEntity> {
        use std::collections::hash_map::Entry;

        let loc = self.entities.get_mut(entity)?;
        unsafe {
            let arch = &mut self.archetypes[loc.archetype as usize];
            let mut info = arch.types().to_vec();
            for ty in components.type_info() {
                if let Some(ptr) = arch.get_dynamic(ty.id(), ty.layout().size(), loc.index) {
                    ty.drop(ptr.as_ptr());
                } else {
                    self.borrows.ensure(ty.id());
                    info.push(ty);
                }
            }
            info.sort();

            let elements = info.iter().map(|x| x.id()).collect::<Vec<_>>();
            let target = match self.index.entry(elements) {
                Entry::Occupied(x) => *x.get(),
                Entry::Vacant(x) => {
                    let index = self.archetypes.len() as u32;
                    self.archetypes.push(Archetype::new(info));
                    x.insert(index);
                    index
                }
            };
            if target == loc.archetype {
                let arch = &mut self.archetypes[loc.archetype as usize];
                components.put(|ptr, ty, size| {
                    arch.put_dynamic(ptr, ty, size, loc.index);
                    true
                });
                return Ok(());
            }

            let (source_arch, target_arch) = index2(
                &mut self.archetypes,
                loc.archetype as usize,
                target as usize,
            );
            let target_index = target_arch.allocate(entity.id);
            source_arch.move_to(loc.index, |ptr, ty, size| {
                target_arch.put_dynamic(ptr, ty, size, target_index);
            });
            components.put(|ptr, ty, size| {
                target_arch.put_dynamic(ptr, ty, size, target_index);
                true
            });
            loc.archetype = target;
            loc.index = target_index;
        }
        Ok(())
    }

    /// Add `component` to `entity`
    ///
    /// See `insert`.
    pub fn insert_one(
        &mut self,
        entity: Entity,
        component: impl Component,
    ) -> Result<(), NoSuchEntity> {
        self.insert(entity, (component,))
    }

    /// Remove components from `entity`
    ///
    /// Computational cost is proportional to the number of components `entity` has. The entity
    /// itself is not removed, even if no components remain; use `despawn` for that. If any
    /// component in `T` is not present in `entity`, no components are removed and an error is
    /// returned.
    ///
    /// # Example
    /// ```
    /// # use hecs::*;
    /// let mut world = World::new();
    /// let e = world.spawn((123, "abc", true));
    /// assert_eq!(world.remove::<(i32, &str)>(e), Ok((123, "abc")));
    /// assert!(world.get::<i32>(e).is_err());
    /// assert!(world.get::<&str>(e).is_err());
    /// assert_eq!(*world.get::<bool>(e).unwrap(), true);
    /// ```
    pub fn remove<T: Bundle>(&mut self, entity: Entity) -> Result<T, ComponentError> {
        use std::collections::hash_map::Entry;

        let loc = self.entities.get_mut(entity)?;
        unsafe {
            let removed = T::with_static_ids(|ids| ids.iter().copied().collect::<FxHashSet<_>>());
            let info = self.archetypes[loc.archetype as usize]
                .types()
                .iter()
                .cloned()
                .filter(|x| !removed.contains(&x.id()))
                .collect::<Vec<_>>();
            let elements = info.iter().map(|x| x.id()).collect::<Vec<_>>();
            let target = match self.index.entry(elements) {
                Entry::Occupied(x) => *x.get(),
                Entry::Vacant(x) => {
                    self.archetypes.push(Archetype::new(info));
                    let index = (self.archetypes.len() - 1) as u32;
                    x.insert(index);
                    index
                }
            };
            let (source_arch, target_arch) = index2(
                &mut self.archetypes,
                loc.archetype as usize,
                target as usize,
            );
            let target_index = target_arch.allocate(entity.id);
            let x = T::get(|ty, size| source_arch.get_dynamic(ty, size, loc.index))?;
            source_arch.move_to(loc.index, |src, ty, size| {
                // Only move the components present in the target archetype, i.e. the non-removed ones.
                if let Some(dst) = target_arch.get_dynamic(ty, size, target_index) {
                    ptr::copy_nonoverlapping(src, dst.as_ptr(), size);
                }
            });
            loc.archetype = target;
            loc.index = target_index;
            Ok(x)
        }
    }

    /// Remove the `T` component from `entity`
    ///
    /// See `remove`.
    pub fn remove_one<T: Component>(&mut self, entity: Entity) -> Result<T, ComponentError> {
        self.remove::<(T,)>(entity).map(|(x,)| x)
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

fn index2<T>(x: &mut [T], i: usize, j: usize) -> (&mut T, &mut T) {
    assert!(i != j);
    assert!(i < x.len());
    assert!(j < x.len());
    let ptr = x.as_mut_ptr();
    unsafe { (&mut *ptr.add(i), &mut *ptr.add(j)) }
}

/// Errors that arise when accessing components
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub enum ComponentError {
    /// The entity was already despawned
    NoSuchEntity,
    /// The entity did not have a requested component
    MissingComponent(MissingComponent),
}

impl std::error::Error for ComponentError {}

impl fmt::Display for ComponentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use ComponentError::*;
        match *self {
            NoSuchEntity => f.write_str("no such entity"),
            MissingComponent(ref x) => x.fmt(f),
        }
    }
}

impl From<NoSuchEntity> for ComponentError {
    fn from(NoSuchEntity: NoSuchEntity) -> Self {
        ComponentError::NoSuchEntity
    }
}

impl From<MissingComponent> for ComponentError {
    fn from(x: MissingComponent) -> Self {
        ComponentError::MissingComponent(x)
    }
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
pub trait Component: Send + Sync + 'static {}
impl<T: Send + Sync + 'static> Component for T {}

/// Lightweight unique ID of an entity
///
/// Obtained from `World::spawn`. Can be stored to refer to an entity in the future.
#[derive(Clone, Copy, Hash, Eq, Ord, PartialEq, PartialOrd)]
pub struct Entity {
    pub(crate) generation: u32,
    pub(crate) id: u32,
}

impl fmt::Debug for Entity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}v{}", self.id, self.generation)
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

impl<A: DynamicBundle> Extend<A> for World {
    fn extend<T>(&mut self, iter: T)
    where
        T: IntoIterator<Item = A>,
    {
        for x in iter {
            self.spawn(x);
        }
    }
}

impl<A: DynamicBundle> std::iter::FromIterator<A> for World {
    fn from_iter<I: IntoIterator<Item = A>>(iter: I) -> Self {
        let mut world = World::new();
        world.extend(iter);
        world
    }
}

#[derive(Default)]
struct Entities {
    meta: Vec<EntityMeta>,
    free: Vec<u32>,
}

impl Entities {
    fn alloc(&mut self) -> Entity {
        match self.free.pop() {
            Some(i) => Entity {
                generation: self.meta[i as usize].generation,
                id: i,
            },
            None => {
                let i = self.meta.len() as u32;
                self.meta.push(EntityMeta {
                    generation: 0,
                    location: Location {
                        archetype: 0,
                        index: 0,
                    },
                });
                Entity {
                    generation: 0,
                    id: i,
                }
            }
        }
    }

    fn free(&mut self, entity: Entity) -> Result<Location, NoSuchEntity> {
        let meta = &mut self.meta[entity.id as usize];
        if meta.generation != entity.generation {
            return Err(NoSuchEntity);
        }
        meta.generation += 1;
        self.free.push(entity.id);
        Ok(meta.location)
    }

    fn clear(&mut self) {
        self.meta.clear();
        self.free.clear();
    }

    fn get_mut(&mut self, entity: Entity) -> Result<&mut Location, NoSuchEntity> {
        let meta = &mut self.meta[entity.id as usize];
        if meta.generation != entity.generation {
            return Err(NoSuchEntity);
        }
        Ok(&mut meta.location)
    }
}

#[derive(Copy, Clone)]
pub(crate) struct EntityMeta {
    pub(crate) generation: u32,
    location: Location,
}

#[derive(Copy, Clone)]
struct Location {
    archetype: u32,
    index: u32,
}

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
