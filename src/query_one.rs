use core::marker::PhantomData;

use crate::query::{Fetch, With, Without};
use crate::{Archetype, Query, QueryItem};

/// A borrow of a [`World`](crate::World) sufficient to execute the query `Q` on a single entity
pub struct QueryOne<'a, Q: Query> {
    archetype: &'a Archetype,
    index: u32,
    borrowed: bool,
    _marker: PhantomData<Q>,
}

impl<'a, Q: Query> QueryOne<'a, Q> {
    /// Construct a query accessing the entity in `archetype` at `index`
    ///
    /// # Safety
    ///
    /// `index` must be in-bounds for `archetype`
    pub(crate) unsafe fn new(archetype: &'a Archetype, index: u32) -> Self {
        Self {
            archetype,
            index,
            borrowed: false,
            _marker: PhantomData,
        }
    }

    /// Get the query result, or `None` if the entity does not satisfy the query
    ///
    /// Must be called at most once.
    ///
    /// Panics if called more than once or if it would construct a borrow that clashes with another
    /// pre-existing borrow.
    // Note that this uses self's lifetime, not 'a, for soundness.
    pub fn get(&mut self) -> Option<QueryItem<'_, Q>> {
        if self.borrowed {
            panic!("called QueryOnce::get twice; construct a new query instead");
        }
        let state = Q::Fetch::prepare(self.archetype)?;
        Q::Fetch::borrow(self.archetype, state);
        let fetch = Q::Fetch::execute(self.archetype, state);
        self.borrowed = true;
        unsafe { Some(fetch.get(self.index as usize)) }
    }

    /// Transform the query into one that requires another query be satisfied
    ///
    /// See `QueryBorrow::with`
    pub fn with<R: Query>(self) -> QueryOne<'a, With<Q, R>> {
        self.transform()
    }

    /// Transform the query into one that skips entities satisfying another
    ///
    /// See `QueryBorrow::without` for details.
    pub fn without<R: Query>(self) -> QueryOne<'a, Without<Q, R>> {
        self.transform()
    }

    /// Helper to change the type of the query
    fn transform<R: Query>(mut self) -> QueryOne<'a, R> {
        let x = QueryOne {
            archetype: self.archetype,
            index: self.index,
            borrowed: self.borrowed,
            _marker: PhantomData,
        };
        // Ensure `Drop` won't fire redundantly
        self.borrowed = false;
        x
    }
}

impl<Q: Query> Drop for QueryOne<'_, Q> {
    fn drop(&mut self) {
        if self.borrowed {
            let state = Q::Fetch::prepare(self.archetype).unwrap();
            Q::Fetch::release(self.archetype, state);
        }
    }
}

unsafe impl<Q: Query> Send for QueryOne<'_, Q> {}
unsafe impl<Q: Query> Sync for QueryOne<'_, Q> {}
