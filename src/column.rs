use core::marker::PhantomData;
use core::ptr::NonNull;

use alloc::vec::Vec;

use crate::entities::{EntityMeta, NoSuchEntity};
use crate::{Archetype, Component, ComponentError, Entity, MissingComponent};

/// Borrows every `T` component in a world
pub struct Column<'a, T: Component> {
    entities: &'a [EntityMeta],
    archetypes: &'a [Archetype],
    archetype_state: Vec<Option<(usize, NonNull<T>)>>,
    _marker: PhantomData<T>,
}

impl<'a, T: Component> Column<'a, T> {
    pub(crate) fn new(
        entities: &'a [EntityMeta],
        archetypes: &'a [Archetype],
        _marker: PhantomData<T>,
    ) -> Self {
        let archetype_state = archetypes
            .iter()
            .map(|archetype| {
                let state = archetype.get_state::<T>();
                state.map(|state| {
                    archetype.borrow::<T>(state);
                    let base = archetype.get_base::<T>(state);
                    (state, base)
                })
            })
            .collect();

        Self {
            entities,
            archetypes,
            archetype_state,
            _marker,
        }
    }

    /// Access the `T` component of `entity`
    pub fn get(&self, entity: Entity) -> Result<&T, ComponentError> {
        let meta = self
            .entities
            .get(entity.id as usize)
            .filter(|meta| meta.generation == entity.generation)
            .ok_or(NoSuchEntity)?;
        let (_state, base) = self.archetype_state[meta.location.archetype as usize]
            .ok_or_else(MissingComponent::new::<T>)?;
        unsafe {
            let target = base.as_ptr().add(meta.location.index as usize);

            Ok(&*target)
        }
    }
}

unsafe impl<'a, T: Component> Send for Column<'a, T> {}
unsafe impl<'a, T: Component> Sync for Column<'a, T> {}

impl<'a, T: Component> Drop for Column<'a, T> {
    fn drop(&mut self) {
        for (archetype, state) in self.archetypes.iter().zip(&self.archetype_state) {
            if let Some((state, _base)) = state {
                archetype.release::<T>(*state);
            }
        }
    }
}

/// Uniquely borrows every `T` component in a world
pub struct ColumnMut<'a, T: Component> {
    entities: &'a [EntityMeta],
    archetypes: &'a [Archetype],
    archetype_state: Vec<Option<(usize, NonNull<T>)>>,
    _marker: PhantomData<T>,
}

impl<'a, T: Component> ColumnMut<'a, T> {
    pub(crate) fn new(
        entities: &'a [EntityMeta],
        archetypes: &'a [Archetype],
        _marker: PhantomData<T>,
    ) -> Self {
        let archetype_state = archetypes
            .iter()
            .map(|archetype| {
                let state = archetype.get_state::<T>();
                state.map(|state| {
                    archetype.borrow_mut::<T>(state);
                    let base = archetype.get_base::<T>(state);
                    (state, base)
                })
            })
            .collect();

        Self {
            entities,
            archetypes,
            archetype_state,
            _marker,
        }
    }

    /// Access the `T` component of `entity`
    pub fn get(&mut self, entity: Entity) -> Result<&mut T, ComponentError> {
        unsafe { self.get_unchecked(entity) }
    }

    /// Access the `T` component of `entity` without unique access to `self`. Allows simultaneous
    /// access to the components of multiple entities.
    ///
    /// # Safety
    ///
    /// Must not be invoked while a borrow to any part of the same `T` is live, e.g. due to a prior
    /// call to `get_unchecked` on the same entity.
    pub unsafe fn get_unchecked(&self, entity: Entity) -> Result<&mut T, ComponentError> {
        let meta = self
            .entities
            .get(entity.id as usize)
            .filter(|meta| meta.generation == entity.generation)
            .ok_or(NoSuchEntity)?;
        let (_state, base) = self.archetype_state[meta.location.archetype as usize]
            .ok_or_else(MissingComponent::new::<T>)?;
        let target = base.as_ptr().add(meta.location.index as usize);
        Ok(&mut *target)
    }
}

unsafe impl<'a, T: Component> Send for ColumnMut<'a, T> {}
unsafe impl<'a, T: Component> Sync for ColumnMut<'a, T> {}

impl<'a, T: Component> Drop for ColumnMut<'a, T> {
    fn drop(&mut self) {
        for (archetype, state) in self.archetypes.iter().zip(&self.archetype_state) {
            if let Some((state, _base)) = state {
                archetype.release_mut::<T>(*state);
            }
        }
    }
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use crate::World;

    #[test]
    fn borrow_twice() {
        let mut world = World::new();
        world.spawn((true, "abc"));
        let c = world.column_mut::<bool>();
        drop(c);
        world.column::<bool>();
    }

    #[test]
    #[should_panic(expected = "bool already borrowed uniquely")]
    fn mut_shared_overlap() {
        let mut world = World::new();
        world.spawn((true, "abc"));
        let c = world.column_mut::<bool>();
        world.column::<bool>();
        drop(c);
    }

    #[test]
    #[should_panic(expected = "bool already borrowed")]
    fn shared_mut_overlap() {
        let mut world = World::new();
        world.spawn((true, "abc"));
        let c = world.column::<bool>();
        world.column_mut::<bool>();
        drop(c);
    }
}
