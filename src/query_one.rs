use core::marker::PhantomData;

use crate::entities::EntityMeta;
use crate::query::{assert_borrow, Fetch, With, Without};
use crate::QueryOneError;
use crate::{Archetype, Query};

/// A borrow of a [`World`](crate::World) sufficient to execute the query `Q` on a single entity
pub struct QueryOne<'a, Q: Query> {
    meta: &'a [EntityMeta],
    archetype: Option<&'a Archetype>,
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
    pub(crate) unsafe fn new(meta: &'a [EntityMeta], archetype: &'a Archetype, index: u32) -> Self {
        Self {
            meta,
            archetype: Some(archetype),
            index,
            borrowed: false,
            _marker: PhantomData,
        }
    }

    /// Get the query result, or an error if the entity does not exist or satisfy the query
    ///
    /// Must be called at most once.
    ///
    /// Panics if called more than once or if it would construct a borrow that clashes with another
    /// pre-existing borrow.
    // Note that this uses self's lifetime, not 'a, for soundness.
    pub fn get(&mut self) -> Result<Q::Item<'_>, QueryOneError> {
        assert_borrow::<Q>();
        if self.borrowed {
            panic!("called QueryOnce::get twice; construct a new query instead");
        }
        let archetype = self.archetype.as_ref().ok_or(QueryOneError::NoSuchEntity)?;
        let state = Q::Fetch::prepare(archetype).ok_or(QueryOneError::Unsatisfied)?;
        Q::Fetch::borrow(archetype, state);
        let fetch = Q::Fetch::execute(archetype, state);
        self.borrowed = true;
        unsafe { Ok(Q::get(self.meta, &fetch, self.index as usize)) }
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
            meta: self.meta,
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

impl<Q: Query> Default for QueryOne<'_, Q> {
    /// Construct a `QueryOne` for which `get` will return `NoSuchEntity`
    fn default() -> Self {
        Self {
            meta: &[],
            archetype: None,
            index: 0,
            borrowed: false,
            _marker: PhantomData,
        }
    }
}

impl<Q: Query> Drop for QueryOne<'_, Q> {
    fn drop(&mut self) {
        if self.borrowed {
            let state = Q::Fetch::prepare(self.archetype.unwrap()).unwrap();
            Q::Fetch::release(self.archetype.unwrap(), state);
        }
    }
}

unsafe impl<Q: Query> Send for QueryOne<'_, Q> {}
unsafe impl<Q: Query> Sync for QueryOne<'_, Q> {}
