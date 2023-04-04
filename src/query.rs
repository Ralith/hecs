// Copyright 2019 Google LLC
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use core::any::TypeId;
use core::marker::PhantomData;
use core::mem;
use core::ptr::NonNull;
use core::slice::Iter as SliceIter;

use crate::alloc::{boxed::Box, vec::Vec};
use crate::archetype::Archetype;
use crate::entities::EntityMeta;
use crate::{Component, Entity, World};

/// A collection of component types to fetch from a [`World`](crate::World)
///
/// The interface of this trait is a private implementation detail.
pub trait Query {
    /// Type of results yielded by the query
    ///
    /// This is usually the same type as the query itself, except with an appropriate lifetime.
    type Item<'a>;

    #[doc(hidden)]
    type Fetch: Fetch;

    #[doc(hidden)]
    /// Access the `n`th item in this archetype without bounds checking
    ///
    /// # Safety
    /// - Must only be called after [`Fetch::borrow`] or with exclusive access to the archetype
    /// - [`Fetch::release`] must not be called while `'a` is still live
    /// - Bounds-checking must be performed externally
    /// - Any resulting borrows must be legal (e.g. no &mut to something another iterator might access)
    unsafe fn get<'a>(fetch: &Self::Fetch, n: usize) -> Self::Item<'a>;
}

/// Marker trait indicating whether a given [`Query`] will not produce unique references
#[allow(clippy::missing_safety_doc)]
pub unsafe trait QueryShared {}

/// Streaming iterators over contiguous homogeneous ranges of components
#[allow(clippy::missing_safety_doc)]
pub unsafe trait Fetch: Sized {
    /// The type of the data which can be cached to speed up retrieving
    /// the relevant type states from a matching [`Archetype`]
    type State: Copy;

    /// A value on which `get` may never be called
    fn dangling() -> Self;

    /// How this query will access `archetype`, if at all
    fn access(archetype: &Archetype) -> Option<Access>;

    /// Acquire dynamic borrows from `archetype`
    fn borrow(archetype: &Archetype, state: Self::State);
    /// Look up state for `archetype` if it should be traversed
    fn prepare(archetype: &Archetype) -> Option<Self::State>;
    /// Construct a `Fetch` for `archetype` based on the associated state
    fn execute(archetype: &Archetype, state: Self::State) -> Self;
    /// Release dynamic borrows acquired by `borrow`
    fn release(archetype: &Archetype, state: Self::State);

    /// Invoke `f` for every component type that may be borrowed and whether the borrow is unique
    fn for_each_borrow(f: impl FnMut(TypeId, bool));
}

/// Type of access a [`Query`] may have to an [`Archetype`]
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub enum Access {
    /// Read entity IDs only, no components
    Iterate,
    /// Read components
    Read,
    /// Read and write components
    Write,
}

impl<'a, T: Component> Query for &'a T {
    type Item<'q> = &'q T;

    type Fetch = FetchRead<T>;

    unsafe fn get<'q>(fetch: &FetchRead<T>, n: usize) -> &'q T {
        &*fetch.0.as_ptr().add(n)
    }
}

unsafe impl<'a, T> QueryShared for &'a T {}

#[doc(hidden)]
pub struct FetchRead<T>(NonNull<T>);

unsafe impl<T: Component> Fetch for FetchRead<T> {
    type State = usize;

    fn dangling() -> Self {
        Self(NonNull::dangling())
    }

    fn access(archetype: &Archetype) -> Option<Access> {
        if archetype.has::<T>() {
            Some(Access::Read)
        } else {
            None
        }
    }

    fn borrow(archetype: &Archetype, state: Self::State) {
        archetype.borrow::<T>(state);
    }
    fn prepare(archetype: &Archetype) -> Option<Self::State> {
        archetype.get_state::<T>()
    }
    fn execute(archetype: &Archetype, state: Self::State) -> Self {
        Self(archetype.get_base(state))
    }
    fn release(archetype: &Archetype, state: Self::State) {
        archetype.release::<T>(state);
    }

    fn for_each_borrow(mut f: impl FnMut(TypeId, bool)) {
        f(TypeId::of::<T>(), false);
    }
}

impl<'a, T: Component> Query for &'a mut T {
    type Item<'q> = &'q mut T;

    type Fetch = FetchWrite<T>;

    unsafe fn get<'q>(fetch: &FetchWrite<T>, n: usize) -> &'q mut T {
        &mut *fetch.0.as_ptr().add(n)
    }
}

#[doc(hidden)]
pub struct FetchWrite<T>(NonNull<T>);

unsafe impl<T: Component> Fetch for FetchWrite<T> {
    type State = usize;

    fn dangling() -> Self {
        Self(NonNull::dangling())
    }

    fn access(archetype: &Archetype) -> Option<Access> {
        if archetype.has::<T>() {
            Some(Access::Write)
        } else {
            None
        }
    }

    fn borrow(archetype: &Archetype, state: Self::State) {
        archetype.borrow_mut::<T>(state);
    }
    #[allow(clippy::needless_question_mark)]
    fn prepare(archetype: &Archetype) -> Option<Self::State> {
        Some(archetype.get_state::<T>()?)
    }
    fn execute(archetype: &Archetype, state: Self::State) -> Self {
        Self(archetype.get_base::<T>(state))
    }
    fn release(archetype: &Archetype, state: Self::State) {
        archetype.release_mut::<T>(state);
    }

    fn for_each_borrow(mut f: impl FnMut(TypeId, bool)) {
        f(TypeId::of::<T>(), true);
    }
}

impl<T: Query> Query for Option<T> {
    type Item<'q> = Option<T::Item<'q>>;

    type Fetch = TryFetch<T::Fetch>;

    unsafe fn get<'q>(fetch: &TryFetch<T::Fetch>, n: usize) -> Option<T::Item<'q>> {
        Some(T::get(fetch.0.as_ref()?, n))
    }
}

unsafe impl<T: QueryShared> QueryShared for Option<T> {}

#[doc(hidden)]
pub struct TryFetch<T>(Option<T>);

unsafe impl<T: Fetch> Fetch for TryFetch<T> {
    type State = Option<T::State>;

    fn dangling() -> Self {
        Self(None)
    }

    fn access(archetype: &Archetype) -> Option<Access> {
        Some(T::access(archetype).unwrap_or(Access::Iterate))
    }

    fn borrow(archetype: &Archetype, state: Self::State) {
        if let Some(state) = state {
            T::borrow(archetype, state);
        }
    }
    fn prepare(archetype: &Archetype) -> Option<Self::State> {
        Some(T::prepare(archetype))
    }
    fn execute(archetype: &Archetype, state: Self::State) -> Self {
        Self(state.map(|state| T::execute(archetype, state)))
    }
    fn release(archetype: &Archetype, state: Self::State) {
        if let Some(state) = state {
            T::release(archetype, state);
        }
    }

    fn for_each_borrow(f: impl FnMut(TypeId, bool)) {
        T::for_each_borrow(f);
    }
}

/// Holds an `L`, or an `R`, or both
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Or<L, R> {
    /// Just an `L`
    Left(L),
    /// Just an `R`
    Right(R),
    /// Both an `L` and an `R`
    Both(L, R),
}

impl<L, R> Or<L, R> {
    /// Construct an `Or<L, R>` if at least one argument is `Some`
    pub fn new(l: Option<L>, r: Option<R>) -> Option<Self> {
        match (l, r) {
            (None, None) => None,
            (Some(l), None) => Some(Or::Left(l)),
            (None, Some(r)) => Some(Or::Right(r)),
            (Some(l), Some(r)) => Some(Or::Both(l, r)),
        }
    }

    /// Destructure into two `Option`s, where either or both are `Some`
    pub fn split(self) -> (Option<L>, Option<R>) {
        match self {
            Or::Left(l) => (Some(l), None),
            Or::Right(r) => (None, Some(r)),
            Or::Both(l, r) => (Some(l), Some(r)),
        }
    }

    /// Extract `L` regardless of whether `R` is present
    pub fn left(self) -> Option<L> {
        match self {
            Or::Left(l) => Some(l),
            Or::Both(l, _) => Some(l),
            _ => None,
        }
    }

    /// Extract `R` regardless of whether `L` is present
    pub fn right(self) -> Option<R> {
        match self {
            Or::Right(r) => Some(r),
            Or::Both(_, r) => Some(r),
            _ => None,
        }
    }

    /// Transform `L` with `f` and `R` with `g`
    pub fn map<L1, R1, F, G>(self, f: F, g: G) -> Or<L1, R1>
    where
        F: FnOnce(L) -> L1,
        G: FnOnce(R) -> R1,
    {
        match self {
            Or::Left(l) => Or::Left(f(l)),
            Or::Right(r) => Or::Right(g(r)),
            Or::Both(l, r) => Or::Both(f(l), g(r)),
        }
    }

    /// Convert from `&Or<L, R>` to `Or<&L, &R>`
    pub fn as_ref(&self) -> Or<&L, &R> {
        match *self {
            Or::Left(ref l) => Or::Left(l),
            Or::Right(ref r) => Or::Right(r),
            Or::Both(ref l, ref r) => Or::Both(l, r),
        }
    }

    /// Convert from `&mut Or<L, R>` to `Or<&mut L, &mut R>`
    pub fn as_mut(&mut self) -> Or<&mut L, &mut R> {
        match *self {
            Or::Left(ref mut l) => Or::Left(l),
            Or::Right(ref mut r) => Or::Right(r),
            Or::Both(ref mut l, ref mut r) => Or::Both(l, r),
        }
    }
}

impl<L, R> Or<&'_ L, &'_ R>
where
    L: Clone,
    R: Clone,
{
    /// Maps an `Or<&L, &R>` to an `Or<L, R>` by cloning its contents
    pub fn cloned(self) -> Or<L, R> {
        self.map(Clone::clone, Clone::clone)
    }
}

impl<L: Query, R: Query> Query for Or<L, R> {
    type Item<'q> = Or<L::Item<'q>, R::Item<'q>>;

    type Fetch = FetchOr<L::Fetch, R::Fetch>;

    unsafe fn get<'q>(fetch: &Self::Fetch, n: usize) -> Self::Item<'q> {
        fetch.0.as_ref().map(|l| L::get(l, n), |r| R::get(r, n))
    }
}

unsafe impl<L: QueryShared, R: QueryShared> QueryShared for Or<L, R> {}

#[doc(hidden)]
pub struct FetchOr<L, R>(Or<L, R>);

unsafe impl<L: Fetch, R: Fetch> Fetch for FetchOr<L, R> {
    type State = Or<L::State, R::State>;

    fn dangling() -> Self {
        Self(Or::Left(L::dangling()))
    }

    fn access(archetype: &Archetype) -> Option<Access> {
        L::access(archetype).max(R::access(archetype))
    }

    fn borrow(archetype: &Archetype, state: Self::State) {
        state.map(|l| L::borrow(archetype, l), |r| R::borrow(archetype, r));
    }

    fn prepare(archetype: &Archetype) -> Option<Self::State> {
        Or::new(L::prepare(archetype), R::prepare(archetype))
    }

    fn execute(archetype: &Archetype, state: Self::State) -> Self {
        Self(state.map(|l| L::execute(archetype, l), |r| R::execute(archetype, r)))
    }

    fn release(archetype: &Archetype, state: Self::State) {
        state.map(|l| L::release(archetype, l), |r| R::release(archetype, r));
    }

    fn for_each_borrow(mut f: impl FnMut(TypeId, bool)) {
        L::for_each_borrow(&mut f);
        R::for_each_borrow(&mut f);
    }
}

/// Query transformer skipping entities that satisfy another query
///
/// See also `QueryBorrow::without`.
///
/// # Example
/// ```
/// # use hecs::*;
/// let mut world = World::new();
/// let a = world.spawn((123, true, "abc"));
/// let b = world.spawn((456, false));
/// let c = world.spawn((42, "def"));
/// let entities = world.query::<Without<&i32, &bool>>()
///     .iter()
///     .map(|(e, &i)| (e, i))
///     .collect::<Vec<_>>();
/// assert_eq!(entities, &[(c, 42)]);
/// ```
pub struct Without<Q, R>(PhantomData<(Q, fn(R))>);

impl<Q: Query, R: Query> Query for Without<Q, R> {
    type Item<'q> = Q::Item<'q>;

    type Fetch = FetchWithout<Q::Fetch, R::Fetch>;

    unsafe fn get<'q>(fetch: &Self::Fetch, n: usize) -> Self::Item<'q> {
        Q::get(&fetch.0, n)
    }
}

unsafe impl<Q: QueryShared, R> QueryShared for Without<Q, R> {}

#[doc(hidden)]
pub struct FetchWithout<F, G>(F, PhantomData<fn(G)>);

unsafe impl<F: Fetch, G: Fetch> Fetch for FetchWithout<F, G> {
    type State = F::State;

    fn dangling() -> Self {
        Self(F::dangling(), PhantomData)
    }

    fn access(archetype: &Archetype) -> Option<Access> {
        if G::access(archetype).is_some() {
            None
        } else {
            F::access(archetype)
        }
    }

    fn borrow(archetype: &Archetype, state: Self::State) {
        F::borrow(archetype, state)
    }
    fn prepare(archetype: &Archetype) -> Option<Self::State> {
        if G::access(archetype).is_some() {
            return None;
        }
        F::prepare(archetype)
    }
    fn execute(archetype: &Archetype, state: Self::State) -> Self {
        Self(F::execute(archetype, state), PhantomData)
    }
    fn release(archetype: &Archetype, state: Self::State) {
        F::release(archetype, state)
    }

    fn for_each_borrow(f: impl FnMut(TypeId, bool)) {
        F::for_each_borrow(f);
    }
}

/// Query transformer skipping entities that do not satisfy another query
///
/// See also `QueryBorrow::with`.
///
/// # Example
/// ```
/// # use hecs::*;
/// let mut world = World::new();
/// let a = world.spawn((123, true, "abc"));
/// let b = world.spawn((456, false));
/// let c = world.spawn((42, "def"));
/// let entities = world.query::<With<&i32, &bool>>()
///     .iter()
///     .map(|(e, &i)| (e, i))
///     .collect::<Vec<_>>();
/// assert_eq!(entities.len(), 2);
/// assert!(entities.contains(&(a, 123)));
/// assert!(entities.contains(&(b, 456)));
/// ```
pub struct With<Q, R>(PhantomData<(Q, fn(R))>);

impl<Q: Query, R: Query> Query for With<Q, R> {
    type Item<'q> = Q::Item<'q>;

    type Fetch = FetchWith<Q::Fetch, R::Fetch>;

    unsafe fn get<'q>(fetch: &Self::Fetch, n: usize) -> Self::Item<'q> {
        Q::get(&fetch.0, n)
    }
}

unsafe impl<Q: QueryShared, R> QueryShared for With<Q, R> {}

#[doc(hidden)]
pub struct FetchWith<F, G>(F, PhantomData<fn(G)>);

unsafe impl<F: Fetch, G: Fetch> Fetch for FetchWith<F, G> {
    type State = F::State;

    fn dangling() -> Self {
        Self(F::dangling(), PhantomData)
    }

    fn access(archetype: &Archetype) -> Option<Access> {
        if G::access(archetype).is_some() {
            F::access(archetype)
        } else {
            None
        }
    }

    fn borrow(archetype: &Archetype, state: Self::State) {
        F::borrow(archetype, state)
    }
    fn prepare(archetype: &Archetype) -> Option<Self::State> {
        G::access(archetype)?;
        F::prepare(archetype)
    }
    fn execute(archetype: &Archetype, state: Self::State) -> Self {
        Self(F::execute(archetype, state), PhantomData)
    }
    fn release(archetype: &Archetype, state: Self::State) {
        F::release(archetype, state)
    }

    fn for_each_borrow(f: impl FnMut(TypeId, bool)) {
        F::for_each_borrow(f);
    }
}

/// A query that yields `true` iff an entity would satisfy the query `Q`
///
/// Does not borrow any components, making it faster and more concurrency-friendly than `Option<Q>`.
///
/// # Example
/// ```
/// # use hecs::*;
/// let mut world = World::new();
/// let a = world.spawn((123, true, "abc"));
/// let b = world.spawn((456, false));
/// let c = world.spawn((42, "def"));
/// let entities = world.query::<Satisfies<&bool>>()
///     .iter()
///     .map(|(e, x)| (e, x))
///     .collect::<Vec<_>>();
/// assert_eq!(entities.len(), 3);
/// assert!(entities.contains(&(a, true)));
/// assert!(entities.contains(&(b, true)));
/// assert!(entities.contains(&(c, false)));
/// ```
pub struct Satisfies<Q>(PhantomData<Q>);

impl<Q: Query> Query for Satisfies<Q> {
    type Item<'q> = bool;

    type Fetch = FetchSatisfies<Q::Fetch>;

    unsafe fn get<'q>(fetch: &Self::Fetch, _: usize) -> Self::Item<'q> {
        fetch.0
    }
}

unsafe impl<Q> QueryShared for Satisfies<Q> {}

#[doc(hidden)]
pub struct FetchSatisfies<F>(bool, PhantomData<F>);

unsafe impl<F: Fetch> Fetch for FetchSatisfies<F> {
    type State = bool;

    fn dangling() -> Self {
        Self(false, PhantomData)
    }

    fn access(_archetype: &Archetype) -> Option<Access> {
        Some(Access::Iterate)
    }

    fn borrow(_archetype: &Archetype, _state: Self::State) {}
    fn prepare(archetype: &Archetype) -> Option<Self::State> {
        Some(F::prepare(archetype).is_some())
    }
    fn execute(_archetype: &Archetype, state: Self::State) -> Self {
        Self(state, PhantomData)
    }
    fn release(_archetype: &Archetype, _state: Self::State) {}

    fn for_each_borrow(_: impl FnMut(TypeId, bool)) {}
}

/// A borrow of a [`World`](crate::World) sufficient to execute the query `Q`
///
/// Note that borrows are not released until this object is dropped.
pub struct QueryBorrow<'w, Q: Query> {
    meta: &'w [EntityMeta],
    archetypes: &'w [Archetype],
    borrowed: bool,
    _marker: PhantomData<Q>,
}

impl<'w, Q: Query> QueryBorrow<'w, Q> {
    pub(crate) fn new(meta: &'w [EntityMeta], archetypes: &'w [Archetype]) -> Self {
        Self {
            meta,
            archetypes,
            borrowed: false,
            _marker: PhantomData,
        }
    }

    /// Execute the query
    // The lifetime narrowing here is required for soundness.
    pub fn iter(&mut self) -> QueryIter<'_, Q> {
        self.borrow();
        unsafe { QueryIter::new(self.meta, self.archetypes.iter()) }
    }

    /// Provide random access to the query results
    pub fn view(&mut self) -> View<'_, Q> {
        self.borrow();
        unsafe { View::new(self.meta, self.archetypes) }
    }

    /// Like `iter`, but returns child iterators of at most `batch_size` elements
    ///
    /// Useful for distributing work over a threadpool.
    // The lifetime narrowing here is required for soundness.
    pub fn iter_batched(&mut self, batch_size: u32) -> BatchedIter<'_, Q> {
        self.borrow();
        unsafe { BatchedIter::new(self.meta, self.archetypes.iter(), batch_size) }
    }

    fn borrow(&mut self) {
        if self.borrowed {
            return;
        }
        for x in self.archetypes {
            if x.is_empty() {
                continue;
            }
            // TODO: Release prior borrows on failure?
            if let Some(state) = Q::Fetch::prepare(x) {
                Q::Fetch::borrow(x, state);
            }
        }
        self.borrowed = true;
    }

    /// Transform the query into one that requires another query be satisfied
    ///
    /// Convenient when the values of the components in the other query are not of interest.
    ///
    /// Equivalent to using a query type wrapped in `With`.
    ///
    /// # Example
    /// ```
    /// # use hecs::*;
    /// let mut world = World::new();
    /// let a = world.spawn((123, true, "abc"));
    /// let b = world.spawn((456, false));
    /// let c = world.spawn((42, "def"));
    /// let entities = world.query::<&i32>()
    ///     .with::<&bool>()
    ///     .iter()
    ///     .map(|(e, &i)| (e, i)) // Copy out of the world
    ///     .collect::<Vec<_>>();
    /// assert_eq!(entities.len(), 2);
    /// assert!(entities.contains(&(a, 123)));
    /// assert!(entities.contains(&(b, 456)));
    /// ```
    pub fn with<R: Query>(self) -> QueryBorrow<'w, With<Q, R>> {
        self.transform()
    }

    /// Transform the query into one that skips entities satisfying another
    ///
    /// Equivalent to using a query type wrapped in `Without`.
    ///
    /// # Example
    /// ```
    /// # use hecs::*;
    /// let mut world = World::new();
    /// let a = world.spawn((123, true, "abc"));
    /// let b = world.spawn((456, false));
    /// let c = world.spawn((42, "def"));
    /// let entities = world.query::<&i32>()
    ///     .without::<&bool>()
    ///     .iter()
    ///     .map(|(e, &i)| (e, i)) // Copy out of the world
    ///     .collect::<Vec<_>>();
    /// assert_eq!(entities, &[(c, 42)]);
    /// ```
    pub fn without<R: Query>(self) -> QueryBorrow<'w, Without<Q, R>> {
        self.transform()
    }

    /// Helper to change the type of the query
    fn transform<R: Query>(mut self) -> QueryBorrow<'w, R> {
        let x = QueryBorrow {
            meta: self.meta,
            archetypes: self.archetypes,
            borrowed: self.borrowed,
            _marker: PhantomData,
        };
        // Ensure `Drop` won't fire redundantly
        self.borrowed = false;
        x
    }
}

unsafe impl<'w, Q: Query> Send for QueryBorrow<'w, Q> where for<'a> Q::Item<'a>: Send {}
unsafe impl<'w, Q: Query> Sync for QueryBorrow<'w, Q> where for<'a> Q::Item<'a>: Send {}

impl<'w, Q: Query> Drop for QueryBorrow<'w, Q> {
    fn drop(&mut self) {
        if self.borrowed {
            for x in self.archetypes {
                if x.is_empty() {
                    continue;
                }
                if let Some(state) = Q::Fetch::prepare(x) {
                    Q::Fetch::release(x, state);
                }
            }
        }
    }
}

impl<'q, 'w, Q: Query> IntoIterator for &'q mut QueryBorrow<'w, Q> {
    type Item = (Entity, Q::Item<'q>);
    type IntoIter = QueryIter<'q, Q>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Iterator over the set of entities with the components in `Q`
pub struct QueryIter<'q, Q: Query> {
    meta: &'q [EntityMeta],
    archetypes: SliceIter<'q, Archetype>,
    iter: ChunkIter<Q>,
}

impl<'q, Q: Query> QueryIter<'q, Q> {
    /// # Safety
    ///
    /// `'q` must be sufficient to guarantee that `Q` cannot violate borrow safety, either with
    /// dynamic borrow checks or by representing exclusive access to the `World`.
    unsafe fn new(meta: &'q [EntityMeta], archetypes: SliceIter<'q, Archetype>) -> Self {
        Self {
            meta,
            archetypes,
            iter: ChunkIter::empty(),
        }
    }
}

unsafe impl<'q, Q: Query> Send for QueryIter<'q, Q> where for<'a> Q::Item<'a>: Send {}
unsafe impl<'q, Q: Query> Sync for QueryIter<'q, Q> where for<'a> Q::Item<'a>: Send {}

impl<'q, Q: Query> Iterator for QueryIter<'q, Q> {
    type Item = (Entity, Q::Item<'q>);

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match unsafe { self.iter.next() } {
                None => {
                    let archetype = self.archetypes.next()?;
                    let state = Q::Fetch::prepare(archetype);
                    let fetch = state.map(|state| Q::Fetch::execute(archetype, state));
                    self.iter = fetch.map_or(ChunkIter::empty(), |fetch| ChunkIter {
                        entities: archetype.entities(),
                        fetch,
                        position: 0,
                        len: archetype.len() as usize,
                    });
                    continue;
                }
                Some((id, components)) => {
                    return Some((
                        Entity {
                            id,
                            generation: unsafe { self.meta.get_unchecked(id as usize).generation },
                        },
                        components,
                    ));
                }
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.len();
        (n, Some(n))
    }
}

impl<'q, Q: Query> ExactSizeIterator for QueryIter<'q, Q> {
    fn len(&self) -> usize {
        self.archetypes
            .clone()
            .filter(|&x| Q::Fetch::access(x).is_some())
            .map(|x| x.len() as usize)
            .sum::<usize>()
            + self.iter.remaining()
    }
}

/// A query builder that's convertible directly into an iterator
pub struct QueryMut<'q, Q: Query> {
    iter: QueryIter<'q, Q>,
}

impl<'q, Q: Query> QueryMut<'q, Q> {
    pub(crate) fn new(meta: &'q [EntityMeta], archetypes: &'q mut [Archetype]) -> Self {
        assert_borrow::<Q>();

        Self {
            iter: unsafe { QueryIter::new(meta, archetypes.iter()) },
        }
    }

    /// Provide random access to the query results
    pub fn view(&mut self) -> View<'_, Q> {
        unsafe { View::new(self.iter.meta, self.iter.archetypes.as_slice()) }
    }

    /// Transform the query into one that requires another query be satisfied
    ///
    /// See `QueryBorrow::with`
    pub fn with<R: Query>(self) -> QueryMut<'q, With<Q, R>> {
        self.transform()
    }

    /// Transform the query into one that skips entities satisfying another
    ///
    /// See `QueryBorrow::without`
    pub fn without<R: Query>(self) -> QueryMut<'q, Without<Q, R>> {
        self.transform()
    }

    /// Helper to change the type of the query
    fn transform<R: Query>(self) -> QueryMut<'q, R> {
        QueryMut {
            iter: unsafe { QueryIter::new(self.iter.meta, self.iter.archetypes) },
        }
    }

    /// Like `into_iter`, but returns child iterators of at most `batch_size` elements
    ///
    /// Useful for distributing work over a threadpool.
    pub fn into_iter_batched(self, batch_size: u32) -> BatchedIter<'q, Q> {
        unsafe { BatchedIter::new(self.iter.meta, self.iter.archetypes, batch_size) }
    }
}

impl<'q, Q: Query> IntoIterator for QueryMut<'q, Q> {
    type Item = <QueryIter<'q, Q> as Iterator>::Item;
    type IntoIter = QueryIter<'q, Q>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter
    }
}

pub(crate) fn assert_borrow<Q: Query>() {
    // This looks like an ugly O(n^2) loop, but everything's constant after inlining, so in
    // practice LLVM optimizes it out entirely.
    let mut i = 0;
    Q::Fetch::for_each_borrow(|a, unique| {
        if unique {
            let mut j = 0;
            Q::Fetch::for_each_borrow(|b, _| {
                if i != j {
                    core::assert!(a != b, "query violates a unique borrow");
                }
                j += 1;
            })
        }
        i += 1;
    });
}

struct ChunkIter<Q: Query> {
    entities: NonNull<u32>,
    fetch: Q::Fetch,
    position: usize,
    len: usize,
}

impl<Q: Query> ChunkIter<Q> {
    fn empty() -> Self {
        Self {
            entities: NonNull::dangling(),
            fetch: Q::Fetch::dangling(),
            position: 0,
            len: 0,
        }
    }

    #[inline]
    unsafe fn next<'a>(&mut self) -> Option<(u32, Q::Item<'a>)> {
        if self.position == self.len {
            return None;
        }
        let entity = self.entities.as_ptr().add(self.position);
        let item = Q::get(&self.fetch, self.position);
        self.position += 1;
        Some((*entity, item))
    }

    fn remaining(&self) -> usize {
        self.len - self.position
    }
}

/// Batched version of [`QueryIter`]
pub struct BatchedIter<'q, Q: Query> {
    _marker: PhantomData<&'q Q>,
    meta: &'q [EntityMeta],
    archetypes: SliceIter<'q, Archetype>,
    batch_size: u32,
    batch: u32,
}

impl<'q, Q: Query> BatchedIter<'q, Q> {
    /// # Safety
    ///
    /// `'q` must be sufficient to guarantee that `Q` cannot violate borrow safety, either with
    /// dynamic borrow checks or by representing exclusive access to the `World`.
    unsafe fn new(
        meta: &'q [EntityMeta],
        archetypes: SliceIter<'q, Archetype>,
        batch_size: u32,
    ) -> Self {
        Self {
            _marker: PhantomData,
            meta,
            archetypes,
            batch_size,
            batch: 0,
        }
    }
}

unsafe impl<'q, Q: Query> Send for BatchedIter<'q, Q> where for<'a> Q::Item<'a>: Send {}
unsafe impl<'q, Q: Query> Sync for BatchedIter<'q, Q> where for<'a> Q::Item<'a>: Send {}

impl<'q, Q: Query> Iterator for BatchedIter<'q, Q> {
    type Item = Batch<'q, Q>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let mut archetypes = self.archetypes.clone();
            let archetype = archetypes.next()?;
            let offset = self.batch_size * self.batch;
            if offset >= archetype.len() {
                self.archetypes = archetypes;
                self.batch = 0;
                continue;
            }
            let state = Q::Fetch::prepare(archetype);
            let fetch = state.map(|state| Q::Fetch::execute(archetype, state));
            if let Some(fetch) = fetch {
                self.batch += 1;
                return Some(Batch {
                    meta: self.meta,
                    state: ChunkIter {
                        entities: archetype.entities(),
                        fetch,
                        len: (offset + self.batch_size.min(archetype.len() - offset)) as usize,
                        position: offset as usize,
                    },
                });
            } else {
                self.archetypes = archetypes;
                debug_assert_eq!(
                    self.batch, 0,
                    "query fetch should always reject at the first batch or not at all"
                );
                continue;
            }
        }
    }
}

/// A sequence of entities yielded by [`BatchedIter`]
pub struct Batch<'q, Q: Query> {
    meta: &'q [EntityMeta],
    state: ChunkIter<Q>,
}

impl<'q, Q: Query> Iterator for Batch<'q, Q> {
    type Item = (Entity, Q::Item<'q>);

    fn next(&mut self) -> Option<Self::Item> {
        let (id, components) = unsafe { self.state.next()? };
        Some((
            Entity {
                id,
                generation: self.meta[id as usize].generation,
            },
            components,
        ))
    }
}

unsafe impl<'q, Q: Query> Send for Batch<'q, Q> where for<'a> Q::Item<'a>: Send {}
unsafe impl<'q, Q: Query> Sync for Batch<'q, Q> where for<'a> Q::Item<'a>: Send {}

macro_rules! tuple_impl {
    ($($name: ident),*) => {
        unsafe impl<$($name: Fetch),*> Fetch for ($($name,)*) {
            type State = ($($name::State,)*);

            #[allow(clippy::unused_unit)]
            fn dangling() -> Self {
                ($($name::dangling(),)*)
            }

            #[allow(unused_variables, unused_mut)]
            fn access(archetype: &Archetype) -> Option<Access> {
                let mut access = Access::Iterate;
                $(
                    access = access.max($name::access(archetype)?);
                )*
                Some(access)
            }

            #[allow(unused_variables, non_snake_case, clippy::unused_unit)]
            fn borrow(archetype: &Archetype, state: Self::State) {
                let ($($name,)*) = state;
                $($name::borrow(archetype, $name);)*
            }
            #[allow(unused_variables)]
            fn prepare(archetype: &Archetype) -> Option<Self::State> {
                Some(($($name::prepare(archetype)?,)*))
            }
            #[allow(unused_variables, non_snake_case, clippy::unused_unit)]
            fn execute(archetype: &Archetype, state: Self::State) -> Self {
                let ($($name,)*) = state;
                ($($name::execute(archetype, $name),)*)
            }
            #[allow(unused_variables, non_snake_case, clippy::unused_unit)]
            fn release(archetype: &Archetype, state: Self::State) {
                let ($($name,)*) = state;
                $($name::release(archetype, $name);)*
            }

            #[allow(unused_variables, unused_mut, clippy::unused_unit)]
            fn for_each_borrow(mut f: impl FnMut(TypeId, bool)) {
                $($name::for_each_borrow(&mut f);)*
            }
        }

        impl<$($name: Query),*> Query for ($($name,)*) {
            type Item<'q> = ($($name::Item<'q>,)*);

            type Fetch = ($($name::Fetch,)*);

            #[allow(unused_variables, clippy::unused_unit)]
            unsafe fn get<'q>(fetch: &Self::Fetch, n: usize) -> Self::Item<'q> {
                #[allow(non_snake_case)]
                let ($(ref $name,)*) = *fetch;
                ($($name::get($name, n),)*)
            }
        }

        unsafe impl<$($name: QueryShared),*> QueryShared for ($($name,)*) {}
    };
}

//smaller_tuples_too!(tuple_impl, B, A);
smaller_tuples_too!(tuple_impl, O, N, M, L, K, J, I, H, G, F, E, D, C, B, A);

/// A prepared query can be stored independently of the [`World`] to amortize query set-up costs.
pub struct PreparedQuery<Q: Query> {
    memo: (u64, u32),
    state: Box<[(usize, <Q::Fetch as Fetch>::State)]>,
    fetch: Box<[Option<Q::Fetch>]>,
}

impl<Q: Query> Default for PreparedQuery<Q> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Q: Query> PreparedQuery<Q> {
    /// Create a prepared query which is not yet attached to any world
    pub fn new() -> Self {
        Self {
            // This memo will not match any world as the first ID will be 1.
            memo: (0, 0),
            state: Default::default(),
            fetch: Default::default(),
        }
    }

    #[cold]
    fn prepare(world: &World) -> Self {
        let memo = world.memo();

        let state = world
            .archetypes()
            .enumerate()
            .filter_map(|(idx, x)| Q::Fetch::prepare(x).map(|state| (idx, state)))
            .collect();

        let fetch = world.archetypes().map(|_| None).collect();

        Self { memo, state, fetch }
    }

    /// Query `world`, using dynamic borrow checking
    ///
    /// This will panic if it would violate an existing unique reference
    /// or construct an invalid unique reference.
    pub fn query<'q>(&'q mut self, world: &'q World) -> PreparedQueryBorrow<'q, Q> {
        if self.memo != world.memo() {
            *self = Self::prepare(world);
        }

        let meta = world.entities_meta();
        let archetypes = world.archetypes_inner();

        PreparedQueryBorrow::new(meta, archetypes, &self.state, &mut self.fetch)
    }

    /// Query a uniquely borrowed world
    ///
    /// Avoids the cost of the dynamic borrow checking performed by [`query`][Self::query].
    pub fn query_mut<'q>(&'q mut self, world: &'q mut World) -> PreparedQueryIter<'q, Q> {
        assert_borrow::<Q>();

        if self.memo != world.memo() {
            *self = Self::prepare(world);
        }

        let meta = world.entities_meta();
        let archetypes = world.archetypes_inner();

        let state: &'q [(usize, <Q::Fetch as Fetch>::State)] =
            unsafe { mem::transmute(&*self.state) };

        unsafe { PreparedQueryIter::new(meta, archetypes, state.iter()) }
    }

    /// Provide random access to query results for a uniquely borrow world
    pub fn view_mut<'q>(&'q mut self, world: &'q mut World) -> PreparedView<'q, Q> {
        assert_borrow::<Q>();

        if self.memo != world.memo() {
            *self = Self::prepare(world);
        }

        let meta = world.entities_meta();
        let archetypes = world.archetypes_inner();

        let state: &'q [(usize, <Q::Fetch as Fetch>::State)] =
            unsafe { mem::transmute(&*self.state) };

        unsafe { PreparedView::new(meta, archetypes, state.iter(), &mut self.fetch) }
    }
}

/// Combined borrow of a [`PreparedQuery`] and a [`World`]
pub struct PreparedQueryBorrow<'q, Q: Query> {
    meta: &'q [EntityMeta],
    archetypes: &'q [Archetype],
    state: &'q [(usize, <Q::Fetch as Fetch>::State)],
    fetch: &'q mut [Option<Q::Fetch>],
}

impl<'q, Q: Query> PreparedQueryBorrow<'q, Q> {
    fn new(
        meta: &'q [EntityMeta],
        archetypes: &'q [Archetype],
        state: &'q [(usize, <Q::Fetch as Fetch>::State)],
        fetch: &'q mut [Option<Q::Fetch>],
    ) -> Self {
        for (idx, state) in state {
            if archetypes[*idx].is_empty() {
                continue;
            }
            Q::Fetch::borrow(&archetypes[*idx], *state);
        }

        Self {
            meta,
            archetypes,
            state,
            fetch,
        }
    }

    /// Execute the prepared query
    // The lifetime narrowing here is required for soundness.
    pub fn iter<'i>(&'i mut self) -> PreparedQueryIter<'i, Q> {
        let state: &'i [(usize, <Q::Fetch as Fetch>::State)] =
            unsafe { mem::transmute(self.state) };

        unsafe { PreparedQueryIter::new(self.meta, self.archetypes, state.iter()) }
    }

    /// Provides random access to the results of the prepared query
    pub fn view<'i>(&'i mut self) -> PreparedView<'i, Q> {
        let state: &'i [(usize, <Q::Fetch as Fetch>::State)] =
            unsafe { mem::transmute(self.state) };

        unsafe { PreparedView::new(self.meta, self.archetypes, state.iter(), self.fetch) }
    }
}

impl<Q: Query> Drop for PreparedQueryBorrow<'_, Q> {
    fn drop(&mut self) {
        for (idx, state) in self.state {
            if self.archetypes[*idx].is_empty() {
                continue;
            }
            Q::Fetch::release(&self.archetypes[*idx], *state);
        }
    }
}

/// Iterates over all entities matching a [`PreparedQuery`]
pub struct PreparedQueryIter<'q, Q: Query> {
    meta: &'q [EntityMeta],
    archetypes: &'q [Archetype],
    state: SliceIter<'q, (usize, <Q::Fetch as Fetch>::State)>,
    iter: ChunkIter<Q>,
}

impl<'q, Q: Query> PreparedQueryIter<'q, Q> {
    /// # Safety
    ///
    /// `'q` must be sufficient to guarantee that `Q` cannot violate borrow safety, either with
    /// dynamic borrow checks or by representing exclusive access to the `World`.
    unsafe fn new(
        meta: &'q [EntityMeta],
        archetypes: &'q [Archetype],
        state: SliceIter<'q, (usize, <Q::Fetch as Fetch>::State)>,
    ) -> Self {
        Self {
            meta,
            archetypes,
            state,
            iter: ChunkIter::empty(),
        }
    }
}

unsafe impl<'q, Q: Query> Send for PreparedQueryIter<'q, Q> where for<'a> Q::Item<'a>: Send {}
unsafe impl<'q, Q: Query> Sync for PreparedQueryIter<'q, Q> where for<'a> Q::Item<'a>: Send {}

impl<'q, Q: Query> Iterator for PreparedQueryIter<'q, Q> {
    type Item = (Entity, Q::Item<'q>);

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match unsafe { self.iter.next() } {
                None => {
                    let (idx, state) = self.state.next()?;
                    let archetype = &self.archetypes[*idx];
                    self.iter = ChunkIter {
                        entities: archetype.entities(),
                        fetch: Q::Fetch::execute(archetype, *state),
                        position: 0,
                        len: archetype.len() as usize,
                    };
                    continue;
                }
                Some((id, components)) => {
                    return Some((
                        Entity {
                            id,
                            generation: unsafe { self.meta.get_unchecked(id as usize).generation },
                        },
                        components,
                    ));
                }
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.len();
        (n, Some(n))
    }
}

impl<Q: Query> ExactSizeIterator for PreparedQueryIter<'_, Q> {
    fn len(&self) -> usize {
        self.state
            .clone()
            .map(|(idx, _)| self.archetypes[*idx].len() as usize)
            .sum::<usize>()
            + self.iter.remaining()
    }
}

/// Provides random access to the results of a query
pub struct View<'q, Q: Query> {
    meta: &'q [EntityMeta],
    fetch: Vec<Option<Q::Fetch>>,
}

unsafe impl<'q, Q: Query> Send for View<'q, Q> where for<'a> Q::Item<'a>: Send {}
unsafe impl<'q, Q: Query> Sync for View<'q, Q> where for<'a> Q::Item<'a>: Send {}

impl<'q, Q: Query> View<'q, Q> {
    /// # Safety
    ///
    /// `'q` must be sufficient to guarantee that `Q` cannot violate borrow safety, either with
    /// dynamic borrow checks or by representing exclusive access to the `World`.
    unsafe fn new(meta: &'q [EntityMeta], archetypes: &'q [Archetype]) -> Self {
        let fetch = archetypes
            .iter()
            .map(|archetype| {
                Q::Fetch::prepare(archetype).map(|state| Q::Fetch::execute(archetype, state))
            })
            .collect();

        Self { meta, fetch }
    }

    /// Retrieve the query results corresponding to `entity`
    ///
    /// Will yield `None` if the entity does not exist or does not match the query.
    ///
    /// Does not require exclusive access to the map, but is defined only for queries yielding only shared references.
    pub fn get(&self, entity: Entity) -> Option<Q::Item<'_>>
    where
        Q: QueryShared,
    {
        let meta = self.meta.get(entity.id as usize)?;
        if meta.generation != entity.generation {
            return None;
        }

        self.fetch[meta.location.archetype as usize]
            .as_ref()
            .map(|fetch| unsafe { Q::get(fetch, meta.location.index as usize) })
    }

    /// Retrieve the query results corresponding to `entity`
    ///
    /// Will yield `None` if the entity does not exist or does not match the query.
    pub fn get_mut(&mut self, entity: Entity) -> Option<Q::Item<'_>> {
        unsafe { self.get_unchecked(entity) }
    }

    /// Like `get_mut`, but allows simultaneous access to multiple entities
    ///
    /// # Safety
    ///
    /// Must not be invoked while any unique borrow of the fetched components of `entity` is live.
    pub unsafe fn get_unchecked(&self, entity: Entity) -> Option<Q::Item<'_>> {
        let meta = self.meta.get(entity.id as usize)?;
        if meta.generation != entity.generation {
            return None;
        }

        self.fetch[meta.location.archetype as usize]
            .as_ref()
            .map(|fetch| Q::get(fetch, meta.location.index as usize))
    }

    /// Like `get_mut`, but allows checked simultaneous access to multiple entities
    ///
    /// For N > 3, the check for distinct entities will clone the array and take O(N log N) time.
    ///
    /// # Examples
    ///
    /// ```
    /// # use hecs::World;
    /// let mut world = World::new();
    ///
    /// let a = world.spawn((1, 1.0));
    /// let b = world.spawn((2, 4.0));
    /// let c = world.spawn((3, 9.0));
    ///
    /// let mut query = world.query_mut::<&mut i32>();
    /// let mut view = query.view();
    /// let [a,b,c] = view.get_mut_n([a, b, c]);
    ///
    /// assert_eq!(*a.unwrap(), 1);
    /// assert_eq!(*b.unwrap(), 2);
    /// assert_eq!(*c.unwrap(), 3);
    /// ```
    pub fn get_mut_n<const N: usize>(&mut self, entities: [Entity; N]) -> [Option<Q::Item<'_>>; N] {
        assert_distinct(&entities);

        let mut items = [(); N].map(|()| None);

        for (item, entity) in items.iter_mut().zip(entities) {
            unsafe {
                *item = self.get_unchecked(entity);
            }
        }

        items
    }
}

/// Provides random access to the results of a prepared query
pub struct PreparedView<'q, Q: Query> {
    meta: &'q [EntityMeta],
    fetch: &'q mut [Option<Q::Fetch>],
}

unsafe impl<'q, Q: Query> Send for PreparedView<'q, Q> where for<'a> Q::Item<'a>: Send {}
unsafe impl<'q, Q: Query> Sync for PreparedView<'q, Q> where for<'a> Q::Item<'a>: Send {}

impl<'q, Q: Query> PreparedView<'q, Q> {
    /// # Safety
    ///
    /// `'q` must be sufficient to guarantee that `Q` cannot violate borrow safety, either with
    /// dynamic borrow checks or by representing exclusive access to the `World`.
    unsafe fn new(
        meta: &'q [EntityMeta],
        archetypes: &'q [Archetype],
        state: SliceIter<'q, (usize, <Q::Fetch as Fetch>::State)>,
        fetch: &'q mut [Option<Q::Fetch>],
    ) -> Self {
        fetch.iter_mut().for_each(|fetch| *fetch = None);

        for (idx, state) in state {
            let archetype = &archetypes[*idx];
            fetch[*idx] = Some(Q::Fetch::execute(archetype, *state));
        }

        Self { meta, fetch }
    }

    /// Retrieve the query results corresponding to `entity`
    ///
    /// Will yield `None` if the entity does not exist or does not match the query.
    ///
    /// Does not require exclusive access to the map, but is defined only for queries yielding only shared references.
    pub fn get(&self, entity: Entity) -> Option<Q::Item<'_>>
    where
        Q: QueryShared,
    {
        let meta = self.meta.get(entity.id as usize)?;
        if meta.generation != entity.generation {
            return None;
        }

        self.fetch[meta.location.archetype as usize]
            .as_ref()
            .map(|fetch| unsafe { Q::get(fetch, meta.location.index as usize) })
    }

    /// Retrieve the query results corresponding to `entity`
    ///
    /// Will yield `None` if the entity does not exist or does not match the query.
    pub fn get_mut(&mut self, entity: Entity) -> Option<Q::Item<'_>> {
        unsafe { self.get_unchecked(entity) }
    }

    /// Like `get_mut`, but allows simultaneous access to multiple entities
    ///
    /// # Safety
    ///
    /// Must not be invoked while any unique borrow of the fetched components of `entity` is live.
    pub unsafe fn get_unchecked(&self, entity: Entity) -> Option<Q::Item<'_>> {
        let meta = self.meta.get(entity.id as usize)?;
        if meta.generation != entity.generation {
            return None;
        }

        self.fetch[meta.location.archetype as usize]
            .as_ref()
            .map(|fetch| Q::get(fetch, meta.location.index as usize))
    }

    /// Like `get_mut`, but allows checked simultaneous access to multiple entities
    ///
    /// See [`View::get_mut_n`] for details.
    pub fn get_mut_n<const N: usize>(&mut self, entities: [Entity; N]) -> [Option<Q::Item<'_>>; N] {
        assert_distinct(&entities);

        let mut items = [(); N].map(|()| None);

        for (item, entity) in items.iter_mut().zip(entities) {
            unsafe {
                *item = self.get_unchecked(entity);
            }
        }

        items
    }
}

fn assert_distinct<const N: usize>(entities: &[Entity; N]) {
    match N {
        1 => (),
        2 => assert_ne!(entities[0], entities[1]),
        3 => {
            assert_ne!(entities[0], entities[1]);
            assert_ne!(entities[1], entities[2]);
            assert_ne!(entities[2], entities[0]);
        }
        _ => {
            let mut entities = *entities;
            entities.sort_unstable();
            for index in 0..N - 1 {
                assert_ne!(entities[index], entities[index + 1]);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_order() {
        assert!(Access::Write > Access::Read);
        assert!(Access::Read > Access::Iterate);
        assert!(Some(Access::Iterate) > None);
    }
}
