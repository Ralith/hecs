use std::any::{type_name, TypeId};
use std::error::Error;
use std::fmt;

use downcast_rs::{impl_downcast, Downcast};
use fxhash::FxHashMap;

use crate::archetype::Archetype;
use crate::borrow::BorrowState;
use crate::{Bundle, EntityRef, Query, QueryIter, Ref, RefMut};

/// An unordered collection of entities, each having any number of distinctly typed components
///
/// The components of entities who have the same set of component types are stored in contiguous
/// runs, allowing for extremely fast, cache-friendly iteration.
#[derive(Default)]
pub struct World {
    entities: Vec<EntityMeta>,
    free: Vec<u32>,
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
    pub fn spawn(&mut self, components: impl Bundle) -> Entity {
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
        self.entities[entity.id as usize].archetype = archetype;
        let archetype = &mut self.archetypes[archetype as usize];
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
        QueryIter::new(&self.borrows, &self.entities, &self.archetypes)
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
                &self.borrows,
                self.archetypes[meta.archetype as usize]
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
                &self.borrows,
                self.archetypes[meta.archetype as usize]
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
            &self.borrows,
            &self.archetypes[meta.archetype as usize],
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
        Iter::new(&self.borrows, &self.archetypes, &self.entities)
    }

    /// Add `components` to `entity`
    ///
    /// Computational cost is proportional to the number of components `entity` has. If an entity
    /// already has a component of a certain type, it is dropped and replaced.
    pub fn insert(&mut self, entity: Entity, components: impl Bundle) -> Result<(), NoSuchEntity> {
        use std::collections::hash_map::Entry;

        let meta = &mut self.entities[entity.id as usize];
        if meta.generation != entity.generation {
            return Err(NoSuchEntity);
        }
        unsafe {
            let arch = &mut self.archetypes[meta.archetype as usize];
            let mut info = arch.types().to_vec();
            for ty in components.type_info() {
                if let Some(ptr) = arch.get_dynamic(ty.id(), ty.layout().size(), meta.index) {
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
            if target == meta.archetype {
                components.store(&mut self.archetypes[meta.archetype as usize], meta.index);
                return Ok(());
            }

            let (source_arch, target_arch) = index2(
                &mut self.archetypes,
                meta.archetype as usize,
                target as usize,
            );
            let old_components = source_arch.move_component_set(meta.index);
            meta.archetype = target;
            meta.index = target_arch.allocate(entity.id);
            old_components.store(target_arch, meta.index);
            components.store(target_arch, meta.index);
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
