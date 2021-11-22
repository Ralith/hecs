use core::marker::PhantomData;

use alloc::vec::Vec;

use crate::entities::{EntityMeta, NoSuchEntity};
use crate::{Archetype, Component, ComponentError, Entity, MissingComponent};

/// Column of a single component
pub struct Column<'a, T: Component> {
    entities: &'a [EntityMeta],
    archetypes: &'a [Archetype],
    archetype_column_indices: Vec<Option<usize>>,
    _marker: PhantomData<T>,
}

impl<'a, T: Component> Column<'a, T> {
    pub(crate) fn new(
        entities: &'a [EntityMeta],
        archetypes: &'a [Archetype],
        _marker: PhantomData<T>,
    ) -> Self {
        let mut archetype_column_indices: Vec<Option<usize>> = Vec::new();
        for i in archetypes.iter() {
            archetype_column_indices.push(i.get_state::<T>());
        }
        for val in archetypes.iter() {
            if val.has::<T>() {
                let state = val.get_state::<T>().unwrap();
                val.borrow::<T>(state);
            }
        }

        Self {
            entities,
            archetypes,
            archetype_column_indices,
            _marker,
        }
    }

    /// Get this column's component from entity if it exist
    pub fn get(&self, entity: Entity) -> Result<&T, ComponentError> {
        let meta = self
            .entities
            .get(entity.id as usize)
            .map_or(true, |meta| meta.generation == entity.generation)
            .then(|| self.entities.get(entity.id as usize).ok_or(NoSuchEntity))
            .ok_or(NoSuchEntity)??;
        let archetype = self
            .archetypes
            .get(meta.location.archetype as usize)
            .unwrap();
        let state = self.archetype_column_indices[meta.location.archetype as usize]
            .ok_or_else(MissingComponent::new::<T>)?;
        unsafe {
            let target = archetype
                .get_base::<T>(state)
                .as_ptr()
                .add(meta.location.index as usize);

            Ok(&*target)
        }
    }
}

unsafe impl<'a, T: Component> Send for Column<'a, T> {}
unsafe impl<'a, T: Component> Sync for Column<'a, T> {}

impl<'a, T: Component> Drop for Column<'a, T> {
    fn drop(&mut self) {
        self.archetype_column_indices.clear();
        for val in self.archetypes.iter() {
            if val.has::<T>() {
                let state = val.get_state::<T>().unwrap();
                val.release::<T>(state);
            }
        }
    }
}

/// Mutable column of a single component
pub struct ColumnMut<'a, T: Component> {
    entities: &'a [EntityMeta],
    archetypes: &'a [Archetype],
    archetype_column_indices: Vec<Option<usize>>,
    _marker: PhantomData<T>,
}

impl<'a, T: Component> ColumnMut<'a, T> {
    pub(crate) fn new(
        entities: &'a [EntityMeta],
        archetypes: &'a [Archetype],
        _marker: PhantomData<T>,
    ) -> Self {
        let mut archetype_column_indices: Vec<Option<usize>> = Vec::new();
        for i in archetypes.iter() {
            archetype_column_indices.push(i.get_state::<T>());
        }
        for val in archetypes.iter() {
            if val.has::<T>() {
                let state = val.get_state::<T>().unwrap();
                val.borrow_mut::<T>(state);
            }
        }
        Self {
            entities,
            archetypes,
            archetype_column_indices,
            _marker,
        }
    }

    /// Mutably get this column's component from entity if it exist
    pub fn get(&self, entity: Entity) -> Result<&mut T, ComponentError> {
        let meta = self
            .entities
            .get(entity.id as usize)
            .map_or(true, |meta| meta.generation == entity.generation)
            .then(|| self.entities.get(entity.id as usize).ok_or(NoSuchEntity))
            .ok_or(NoSuchEntity)??;
        let archetype = self
            .archetypes
            .get(meta.location.archetype as usize)
            .unwrap();
        let state = self.archetype_column_indices[meta.location.archetype as usize]
            .ok_or_else(MissingComponent::new::<T>)?;
        unsafe {
            let target = archetype
                .get_base::<T>(state)
                .as_ptr()
                .add(meta.location.index as usize);
            Ok(&mut *target)
        }
    }
}

unsafe impl<'a, T: Component> Send for ColumnMut<'a, T> {}
unsafe impl<'a, T: Component> Sync for ColumnMut<'a, T> {}

impl<'a, T: Component> Drop for ColumnMut<'a, T> {
    fn drop(&mut self) {
        self.archetype_column_indices.clear();
        for val in self.archetypes.iter() {
            if val.has::<T>() {
                let state = val.get_state::<T>().unwrap();
                val.release_mut::<T>(state);
            }
        }
    }
}
