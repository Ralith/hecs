use core::marker::PhantomData;

use crate::query::{Fetch, With, Without};
use crate::{Archetype, Component, Entity, Query};

/// A borrow of a `World` sufficient to execute the query `Q` on a single entity
pub struct QueryOne<'w, Q: Query<'w, C>, C: Copy + 'w> {
    archetype: &'w Archetype,
    index: u32,
    borrowed: bool,
    entity: Entity,
    context: C,
    _marker: PhantomData<Q>,
}

impl<'w, Q: Query<'w, C>, C: Copy + 'w> QueryOne<'w, Q, C> {
    /// Construct a query accessing the entity in `archetype` at `index`
    ///
    /// # Safety
    ///
    /// `index` must be in-bounds for `archetype`
    pub(crate) unsafe fn new(
        archetype: &'w Archetype,
        index: u32,
        entity: Entity,
        context: C,
    ) -> Self {
        Self {
            archetype,
            index,
            borrowed: false,
            entity,
            context,
            _marker: PhantomData,
        }
    }

    /// Get the query result, or `None` if the entity does not satisfy the query
    ///
    /// Must be called at most once.
    ///
    /// Panics if called more than once or if it would construct a borrow that clashes with another
    /// pre-existing borrow.
    pub fn get(&mut self) -> Option<<Q::Fetch as Fetch<'_, 'w, C>>::Item> {
        if self.borrowed {
            panic!("called QueryOnce::get twice; construct a new query instead");
        }
        unsafe {
            let mut fetch = Q::Fetch::get(self.archetype, self.index as usize)?;
            self.borrowed = true;
            Q::Fetch::borrow(self.archetype);
            Some(fetch.next(self.entity, self.context))
        }
    }

    /// Transform the query into one that requires a certain component without borrowing it
    ///
    /// See `QueryBorrow::with` for details.
    pub fn with<T: Component>(self) -> QueryOne<'w, With<T, Q>, C> {
        self.transform()
    }

    /// Transform the query into one that skips entities having a certain component
    ///
    /// See `QueryBorrow::without` for details.
    pub fn without<T: Component>(self) -> QueryOne<'w, Without<T, Q>, C> {
        self.transform()
    }

    /// Helper to change the type of the query
    fn transform<R: Query<'w, C>>(mut self) -> QueryOne<'w, R, C> {
        let x = QueryOne {
            archetype: self.archetype,
            index: self.index,
            borrowed: self.borrowed,
            entity: self.entity,
            context: self.context,
            _marker: PhantomData,
        };
        // Ensure `Drop` won't fire redundantly
        self.borrowed = false;
        x
    }
}

impl<'w, Q: Query<'w, C>, C: Copy + 'w> Drop for QueryOne<'w, Q, C> {
    fn drop(&mut self) {
        if self.borrowed {
            Q::Fetch::release(self.archetype);
        }
    }
}

unsafe impl<'w, Q: Query<'w, C>, C: Copy + Sync + 'w> Send for QueryOne<'w, Q, C> {}
unsafe impl<'w, Q: Query<'w, C>, C: Copy + Sync + 'w> Sync for QueryOne<'w, Q, C> {}
