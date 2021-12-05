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

use crate::alloc::boxed::Box;
use crate::archetype::Archetype;
use crate::entities::EntityMeta;
use crate::{Component, Entity, World};

/// A collection of component types to fetch from a [`World`](crate::World)
pub trait Query {
    #[doc(hidden)]
    type Fetch: for<'a> Fetch<'a>;
}

/// Type of values yielded by a query
///
/// Once rust offers generic associated types, this will be moved into [`Query`].
pub type QueryItem<'a, Q> = <<Q as Query>::Fetch as Fetch<'a>>::Item;

/// Streaming iterators over contiguous homogeneous ranges of components
#[allow(clippy::missing_safety_doc)]
pub unsafe trait Fetch<'a>: Sized {
    /// Type of value to be fetched
    type Item;

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
    fn execute(archetype: &'a Archetype, state: Self::State) -> Self;
    /// Release dynamic borrows acquired by `borrow`
    fn release(archetype: &Archetype, state: Self::State);

    /// Invoke `f` for every component type that may be borrowed and whether the borrow is unique
    fn for_each_borrow(f: impl FnMut(TypeId, bool));

    /// Access the `n`th item in this archetype without bounds checking
    ///
    /// # Safety
    /// - Must only be called after `borrow`
    /// - `release` must not be called while `'a` is still live
    /// - Bounds-checking must be performed externally
    /// - Any resulting borrows must be legal (e.g. no &mut to something another iterator might access)
    unsafe fn get(&self, n: usize) -> Self::Item;
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
    type Fetch = FetchRead<T>;
}

#[doc(hidden)]
pub struct FetchRead<T>(NonNull<T>);

unsafe impl<'a, T: Component> Fetch<'a> for FetchRead<T> {
    type Item = &'a T;

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
    fn execute(archetype: &'a Archetype, state: Self::State) -> Self {
        Self(archetype.get_base(state))
    }
    fn release(archetype: &Archetype, state: Self::State) {
        archetype.release::<T>(state);
    }

    fn for_each_borrow(mut f: impl FnMut(TypeId, bool)) {
        f(TypeId::of::<T>(), false);
    }

    unsafe fn get(&self, n: usize) -> Self::Item {
        &*self.0.as_ptr().add(n)
    }
}

impl<'a, T: Component> Query for &'a mut T {
    type Fetch = FetchWrite<T>;
}

#[doc(hidden)]
pub struct FetchWrite<T>(NonNull<T>);

unsafe impl<'a, T: Component> Fetch<'a> for FetchWrite<T> {
    type Item = &'a mut T;

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
    fn execute(archetype: &'a Archetype, state: Self::State) -> Self {
        Self(archetype.get_base::<T>(state))
    }
    fn release(archetype: &Archetype, state: Self::State) {
        archetype.release_mut::<T>(state);
    }

    fn for_each_borrow(mut f: impl FnMut(TypeId, bool)) {
        f(TypeId::of::<T>(), true);
    }

    unsafe fn get(&self, n: usize) -> Self::Item {
        &mut *self.0.as_ptr().add(n)
    }
}

impl<T: Query> Query for Option<T> {
    type Fetch = TryFetch<T::Fetch>;
}

#[doc(hidden)]
pub struct TryFetch<T>(Option<T>);

unsafe impl<'a, T: Fetch<'a>> Fetch<'a> for TryFetch<T> {
    type Item = Option<T::Item>;

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
    fn execute(archetype: &'a Archetype, state: Self::State) -> Self {
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

    unsafe fn get(&self, n: usize) -> Option<T::Item> {
        Some(self.0.as_ref()?.get(n))
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
    type Fetch = FetchOr<L::Fetch, R::Fetch>;
}

#[doc(hidden)]
pub struct FetchOr<L, R>(Or<L, R>);

unsafe impl<'a, L: Fetch<'a>, R: Fetch<'a>> Fetch<'a> for FetchOr<L, R> {
    type Item = Or<L::Item, R::Item>;

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

    fn execute(archetype: &'a Archetype, state: Self::State) -> Self {
        Self(state.map(|l| L::execute(archetype, l), |r| R::execute(archetype, r)))
    }

    fn release(archetype: &Archetype, state: Self::State) {
        state.map(|l| L::release(archetype, l), |r| R::release(archetype, r));
    }

    fn for_each_borrow(mut f: impl FnMut(TypeId, bool)) {
        L::for_each_borrow(&mut f);
        R::for_each_borrow(&mut f);
    }

    unsafe fn get(&self, n: usize) -> Self::Item {
        self.0.as_ref().map(|l| l.get(n), |r| r.get(n))
    }
}

/// Query transformer skipping entities that have a `T` component
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
/// let entities = world.query::<Without<bool, &i32>>()
///     .iter()
///     .map(|(e, &i)| (e, i))
///     .collect::<Vec<_>>();
/// assert_eq!(entities, &[(c, 42)]);
/// ```
pub struct Without<T, Q>(PhantomData<(Q, fn(T))>);

impl<T: Component, Q: Query> Query for Without<T, Q> {
    type Fetch = FetchWithout<T, Q::Fetch>;
}

#[doc(hidden)]
pub struct FetchWithout<T, F>(F, PhantomData<fn(T)>);

unsafe impl<'a, T: Component, F: Fetch<'a>> Fetch<'a> for FetchWithout<T, F> {
    type Item = F::Item;

    type State = F::State;

    fn dangling() -> Self {
        Self(F::dangling(), PhantomData)
    }

    fn access(archetype: &Archetype) -> Option<Access> {
        if archetype.has::<T>() {
            None
        } else {
            F::access(archetype)
        }
    }

    fn borrow(archetype: &Archetype, state: Self::State) {
        F::borrow(archetype, state)
    }
    fn prepare(archetype: &Archetype) -> Option<Self::State> {
        if archetype.has::<T>() {
            return None;
        }
        F::prepare(archetype)
    }
    fn execute(archetype: &'a Archetype, state: Self::State) -> Self {
        Self(F::execute(archetype, state), PhantomData)
    }
    fn release(archetype: &Archetype, state: Self::State) {
        F::release(archetype, state)
    }

    fn for_each_borrow(f: impl FnMut(TypeId, bool)) {
        F::for_each_borrow(f);
    }

    unsafe fn get(&self, n: usize) -> F::Item {
        self.0.get(n)
    }
}

/// Query transformer skipping entities that do not have a `T` component
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
/// let entities = world.query::<With<bool, &i32>>()
///     .iter()
///     .map(|(e, &i)| (e, i))
///     .collect::<Vec<_>>();
/// assert_eq!(entities.len(), 2);
/// assert!(entities.contains(&(a, 123)));
/// assert!(entities.contains(&(b, 456)));
/// ```
pub struct With<T, Q>(PhantomData<(Q, fn(T))>);

impl<T: Component, Q: Query> Query for With<T, Q> {
    type Fetch = FetchWith<T, Q::Fetch>;
}

#[doc(hidden)]
pub struct FetchWith<T, F>(F, PhantomData<fn(T)>);

unsafe impl<'a, T: Component, F: Fetch<'a>> Fetch<'a> for FetchWith<T, F> {
    type Item = F::Item;

    type State = F::State;

    fn dangling() -> Self {
        Self(F::dangling(), PhantomData)
    }

    fn access(archetype: &Archetype) -> Option<Access> {
        if archetype.has::<T>() {
            F::access(archetype)
        } else {
            None
        }
    }

    fn borrow(archetype: &Archetype, state: Self::State) {
        F::borrow(archetype, state)
    }
    fn prepare(archetype: &Archetype) -> Option<Self::State> {
        if !archetype.has::<T>() {
            return None;
        }
        F::prepare(archetype)
    }
    fn execute(archetype: &'a Archetype, state: Self::State) -> Self {
        Self(F::execute(archetype, state), PhantomData)
    }
    fn release(archetype: &Archetype, state: Self::State) {
        F::release(archetype, state)
    }

    fn for_each_borrow(f: impl FnMut(TypeId, bool)) {
        F::for_each_borrow(f);
    }

    unsafe fn get(&self, n: usize) -> F::Item {
        self.0.get(n)
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
    type Fetch = FetchSatisfies<Q::Fetch>;
}

#[doc(hidden)]
pub struct FetchSatisfies<F>(bool, PhantomData<F>);

unsafe impl<'a, F: Fetch<'a>> Fetch<'a> for FetchSatisfies<F> {
    type Item = bool;

    type State = bool;

    fn dangling() -> Self {
        Self(false, PhantomData)
    }

    fn access(archetype: &Archetype) -> Option<Access> {
        F::access(archetype).map(|_| Access::Iterate)
    }

    fn borrow(_archetype: &Archetype, _state: Self::State) {}
    fn prepare(archetype: &Archetype) -> Option<Self::State> {
        Some(F::prepare(archetype).is_some())
    }
    fn execute(_archetype: &'a Archetype, state: Self::State) -> Self {
        Self(state, PhantomData)
    }
    fn release(_archetype: &Archetype, _state: Self::State) {}

    fn for_each_borrow(_: impl FnMut(TypeId, bool)) {}

    unsafe fn get(&self, _: usize) -> bool {
        self.0
    }
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
            // TODO: Release prior borrows on failure?
            if let Some(state) = Q::Fetch::prepare(x) {
                Q::Fetch::borrow(x, state);
            }
        }
        self.borrowed = true;
    }

    /// Transform the query into one that requires a certain component without borrowing it
    ///
    /// This can be useful when the component needs to be borrowed elsewhere and it isn't necessary
    /// for the iterator to expose its data directly.
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
    ///     .with::<bool>()
    ///     .iter()
    ///     .map(|(e, &i)| (e, i)) // Copy out of the world
    ///     .collect::<Vec<_>>();
    /// assert!(entities.contains(&(a, 123)));
    /// assert!(entities.contains(&(b, 456)));
    /// ```
    pub fn with<T: Component>(self) -> QueryBorrow<'w, With<T, Q>> {
        self.transform()
    }

    /// Transform the query into one that skips entities having a certain component
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
    ///     .without::<bool>()
    ///     .iter()
    ///     .map(|(e, &i)| (e, i)) // Copy out of the world
    ///     .collect::<Vec<_>>();
    /// assert_eq!(entities, &[(c, 42)]);
    /// ```
    pub fn without<T: Component>(self) -> QueryBorrow<'w, Without<T, Q>> {
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

unsafe impl<'w, Q: Query> Send for QueryBorrow<'w, Q> {}
unsafe impl<'w, Q: Query> Sync for QueryBorrow<'w, Q> {}

impl<'w, Q: Query> Drop for QueryBorrow<'w, Q> {
    fn drop(&mut self) {
        if self.borrowed {
            for x in self.archetypes {
                if let Some(state) = Q::Fetch::prepare(x) {
                    Q::Fetch::release(x, state);
                }
            }
        }
    }
}

impl<'q, 'w, Q: Query> IntoIterator for &'q mut QueryBorrow<'w, Q> {
    type Item = (Entity, QueryItem<'q, Q>);
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

unsafe impl<'q, Q: Query> Send for QueryIter<'q, Q> {}
unsafe impl<'q, Q: Query> Sync for QueryIter<'q, Q> {}

impl<'q, Q: Query> Iterator for QueryIter<'q, Q> {
    type Item = (Entity, QueryItem<'q, Q>);

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

    /// Transform the query into one that requires a certain component without borrowing it
    ///
    /// See `QueryBorrow::with`
    pub fn with<T: Component>(self) -> QueryMut<'q, With<T, Q>> {
        self.transform()
    }

    /// Transform the query into one that skips entities having a certain component
    ///
    /// See `QueryBorrow::without`
    pub fn without<T: Component>(self) -> QueryMut<'q, Without<T, Q>> {
        self.transform()
    }

    /// Helper to change the type of the query
    fn transform<R: Query>(self) -> QueryMut<'q, R> {
        QueryMut {
            iter: unsafe { QueryIter::new(self.iter.meta, self.iter.archetypes) },
        }
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

fn assert_borrow<Q: Query>() {
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
    unsafe fn next<'a>(&mut self) -> Option<(u32, <Q::Fetch as Fetch<'a>>::Item)> {
        if self.position == self.len {
            return None;
        }
        let entity = self.entities.as_ptr().add(self.position);
        let item = self.fetch.get(self.position);
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

unsafe impl<'q, Q: Query> Send for BatchedIter<'q, Q> {}
unsafe impl<'q, Q: Query> Sync for BatchedIter<'q, Q> {}

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
    type Item = (Entity, QueryItem<'q, Q>);

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

unsafe impl<'q, Q: Query> Send for Batch<'q, Q> {}
unsafe impl<'q, Q: Query> Sync for Batch<'q, Q> {}

macro_rules! tuple_impl {
    ($($name: ident),*) => {
        unsafe impl<'a, $($name: Fetch<'a>),*> Fetch<'a> for ($($name,)*) {
            type Item = ($($name::Item,)*);

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
            fn execute(archetype: &'a Archetype, state: Self::State) -> Self {
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

            #[allow(unused_variables, clippy::unused_unit)]
            unsafe fn get(&self, n: usize) -> Self::Item {
                #[allow(non_snake_case)]
                let ($($name,)*) = self;
                ($($name.get(n),)*)
            }
        }

        impl<$($name: Query),*> Query for ($($name,)*) {
            type Fetch = ($($name::Fetch,)*);
        }
    };
}

//smaller_tuples_too!(tuple_impl, B, A);
smaller_tuples_too!(tuple_impl, O, N, M, L, K, J, I, H, G, F, E, D, C, B, A);

/// A prepared query can be stored independently of the [`World`] to amortize query set-up costs.
pub struct PreparedQuery<Q: Query> {
    memo: (u64, u64),
    state: Box<[(usize, <Q::Fetch as Fetch<'static>>::State)]>,
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
            // This memo will not match any world as the first ID will 1.
            memo: (0, 0),
            state: Default::default(),
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

        Self { memo, state }
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

        PreparedQueryBorrow::new(meta, archetypes, &*self.state)
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

        let state: &'q [(usize, <Q::Fetch as Fetch<'q>>::State)] =
            unsafe { mem::transmute(&*self.state) };

        unsafe { PreparedQueryIter::new(meta, archetypes, state.iter()) }
    }
}

/// Combined borrow of a [`PreparedQuery`] and a [`World`]
pub struct PreparedQueryBorrow<'q, Q: Query> {
    meta: &'q [EntityMeta],
    archetypes: &'q [Archetype],
    state: &'q [(usize, <Q::Fetch as Fetch<'static>>::State)],
}

impl<'q, Q: Query> PreparedQueryBorrow<'q, Q> {
    fn new(
        meta: &'q [EntityMeta],
        archetypes: &'q [Archetype],
        state: &'q [(usize, <Q::Fetch as Fetch<'static>>::State)],
    ) -> Self {
        for (idx, state) in state {
            Q::Fetch::borrow(&archetypes[*idx], *state);
        }

        Self {
            meta,
            archetypes,
            state,
        }
    }

    /// Execute the prepared query
    // The lifetime narrowing here is required for soundness.
    pub fn iter<'i>(&'i mut self) -> PreparedQueryIter<'i, Q> {
        let state: &'i [(usize, <Q::Fetch as Fetch<'i>>::State)] =
            unsafe { mem::transmute(self.state) };

        unsafe { PreparedQueryIter::new(self.meta, self.archetypes, state.iter()) }
    }
}

impl<Q: Query> Drop for PreparedQueryBorrow<'_, Q> {
    fn drop(&mut self) {
        for (idx, state) in self.state {
            Q::Fetch::release(&self.archetypes[*idx], *state);
        }
    }
}

/// Iterates over all entities matching a [`PreparedQuery`]
pub struct PreparedQueryIter<'q, Q: Query> {
    meta: &'q [EntityMeta],
    archetypes: &'q [Archetype],
    state: SliceIter<'q, (usize, <Q::Fetch as Fetch<'q>>::State)>,
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
        state: SliceIter<'q, (usize, <Q::Fetch as Fetch<'q>>::State)>,
    ) -> Self {
        Self {
            meta,
            archetypes,
            state,
            iter: ChunkIter::empty(),
        }
    }
}

unsafe impl<Q: Query> Send for PreparedQueryIter<'_, Q> {}
unsafe impl<Q: Query> Sync for PreparedQueryIter<'_, Q> {}

impl<'q, Q: Query> Iterator for PreparedQueryIter<'q, Q> {
    type Item = (Entity, QueryItem<'q, Q>);

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
