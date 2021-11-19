use core::marker::PhantomData;
use core::ptr::NonNull;

use alloc::vec::Vec;

use crate::entities::EntityMeta;
use crate::{Archetype, Component, ComponentError, Entity, MissingComponent};

/// Column of a single component
pub struct Column<'a, T> {
    entities: &'a [EntityMeta],
    archetypes: &'a [Archetype],
    archetype_column_offsets: Vec<Option<usize>>,
    _marker: PhantomData<T>,
}

impl<'a, T: Component> Column<'a, T> {
    pub(crate) fn new(
        entities: &'a [EntityMeta],
        archetypes: &'a [Archetype],
        archetype_column_offsets: Vec<Option<usize>>,
        _marker: PhantomData<T>,
    ) -> Self {
        Self {
            entities,
            archetypes,
            archetype_column_offsets,
            _marker,
        }
    }

    /// Get this column's component from entity if it exist
    pub fn get(&self, entity: Entity) -> Result<&T, ComponentError> {
        unsafe {
            let meta = self
                .entities
                .get(entity.id as usize)
                .expect("no such entity");
            let archetype = self
                .archetypes
                .get(meta.location.archetype as usize)
                .unwrap();
            let state = self.archetype_column_offsets[meta.location.archetype as usize]
                .ok_or_else(MissingComponent::new::<T>)?;
            let target = NonNull::new_unchecked(
                archetype
                    .get_base::<T>(state)
                    .as_ptr()
                    .add(meta.location.index as usize),
            );
            archetype.borrow::<T>(state);
            let comp = target.as_ptr().cast::<T>();
            Ok(&*comp)
        }
    }
}

/// Mutable column of a single component
pub struct ColumnMut<'a, T> {
    entities: &'a [EntityMeta],
    archetypes: &'a [Archetype],
    archetype_column_offsets: Vec<Option<usize>>,
    _marker: PhantomData<T>,
}

impl<'a, T: Component> ColumnMut<'a, T> {
    pub(crate) fn new(
        entities: &'a [EntityMeta],
        archetypes: &'a [Archetype],
        archetype_column_offsets: Vec<Option<usize>>,
        _marker: PhantomData<T>,
    ) -> Self {
        Self {
            entities,
            archetypes,
            archetype_column_offsets,
            _marker,
        }
    }

    /// Mutably get this column's component from entity if it exist
    pub fn get(&self, entity: Entity) -> Result<&mut T, ComponentError> {
        unsafe {
            let meta = self
                .entities
                .get(entity.id as usize)
                .expect("no such entity");
            let archetype = self
                .archetypes
                .get(meta.location.archetype as usize)
                .unwrap();
            let state = self.archetype_column_offsets[meta.location.archetype as usize]
                .ok_or_else(MissingComponent::new::<T>)?;
            let target = NonNull::new_unchecked(
                archetype
                    .get_base::<T>(state)
                    .as_ptr()
                    .add(meta.location.index as usize),
            );
            archetype.borrow_mut::<T>(state);
            let comp = target.as_ptr().cast::<T>();
            Ok(&mut *comp)
        }
    }
}
