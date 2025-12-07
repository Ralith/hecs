#[cfg(feature = "std")]
use core::any::Any;
use core::marker::PhantomData;
use core::ptr::NonNull;
use core::slice::Iter as SliceIter;
use core::{any::TypeId, num::NonZeroU32};
#[cfg(feature = "std")]
use std::sync::{Arc, RwLock};

use crate::alloc::boxed::Box;
#[cfg(feature = "std")]
use hashbrown::hash_map;

use crate::archetype::Archetype;
use crate::entities::EntityMeta;
#[cfg(feature = "std")]
use crate::TypeIdMap;
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
    /// Access the `n`th item in this archetype, an entity with generation `generation`, without bounds checking
    ///
    /// # Safety
    /// - Must only be called after [`Fetch::borrow`] or with exclusive access to the archetype
    /// - [`Fetch::release`] must not be called while `'a` is still live
    /// - Bounds-checking must be performed externally
    /// - Any resulting borrows must be legal (e.g. no &mut to something another iterator might access)
    unsafe fn get<'a>(generation: NonZeroU32, fetch: &Self::Fetch, n: usize) -> Self::Item<'a>;
}

/// Marker trait indicating whether a given [`Query`] will not produce unique references
#[allow(clippy::missing_safety_doc)]
pub unsafe trait QueryShared {}

/// Streaming iterators over contiguous homogeneous ranges of components
#[allow(clippy::missing_safety_doc)]
pub unsafe trait Fetch: Clone + Sized + 'static {
    /// The type of the data which can be cached to speed up retrieving
    /// the relevant type states from a matching [`Archetype`]
    type State: Copy + Send + Sync;

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

impl Query for Entity {
    type Item<'q> = Entity;
    type Fetch = FetchEntity;

    unsafe fn get<'q>(generation: NonZeroU32, fetch: &Self::Fetch, n: usize) -> Self::Item<'q> {
        Entity {
            id: fetch.0.as_ptr().add(n).read(),
            generation,
        }
    }
}

#[doc(hidden)]
#[derive(Clone)]
pub struct FetchEntity(NonNull<u32>);

unsafe impl Fetch for FetchEntity {
    type State = ();

    fn dangling() -> Self {
        Self(NonNull::dangling())
    }

    fn access(_: &Archetype) -> Option<Access> {
        Some(Access::Iterate)
    }

    fn borrow(_: &Archetype, (): Self::State) {}

    fn prepare(_: &Archetype) -> Option<Self::State> {
        Some(())
    }

    fn execute(archetype: &Archetype, (): Self::State) -> Self {
        Self(archetype.entities())
    }

    fn release(_: &Archetype, (): Self::State) {}

    fn for_each_borrow(_: impl FnMut(TypeId, bool)) {}
}

impl<T: Component> Query for &'_ T {
    type Item<'q> = &'q T;

    type Fetch = FetchRead<T>;

    unsafe fn get<'q>(_: NonZeroU32, fetch: &FetchRead<T>, n: usize) -> &'q T {
        &*fetch.0.as_ptr().add(n)
    }
}

unsafe impl<T> QueryShared for &'_ T {}

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
        unsafe { Self(archetype.get_base(state)) }
    }
    fn release(archetype: &Archetype, state: Self::State) {
        archetype.release::<T>(state);
    }

    fn for_each_borrow(mut f: impl FnMut(TypeId, bool)) {
        f(TypeId::of::<T>(), false);
    }
}

impl<T> Clone for FetchRead<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0)
    }
}

impl<T: Component> Query for &'_ mut T {
    type Item<'q> = &'q mut T;

    type Fetch = FetchWrite<T>;

    unsafe fn get<'q>(_: NonZeroU32, fetch: &FetchWrite<T>, n: usize) -> &'q mut T {
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
        unsafe { Self(archetype.get_base::<T>(state)) }
    }
    fn release(archetype: &Archetype, state: Self::State) {
        archetype.release_mut::<T>(state);
    }

    fn for_each_borrow(mut f: impl FnMut(TypeId, bool)) {
        f(TypeId::of::<T>(), true);
    }
}

impl<T> Clone for FetchWrite<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0)
    }
}

impl<T: Query> Query for Option<T> {
    type Item<'q> = Option<T::Item<'q>>;

    type Fetch = TryFetch<T::Fetch>;

    unsafe fn get<'q>(
        generation: NonZeroU32,
        fetch: &TryFetch<T::Fetch>,
        n: usize,
    ) -> Option<T::Item<'q>> {
        Some(T::get(generation, fetch.0.as_ref()?, n))
    }
}

unsafe impl<T: QueryShared> QueryShared for Option<T> {}

#[doc(hidden)]
#[derive(Clone)]
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

    unsafe fn get<'q>(generation: NonZeroU32, fetch: &Self::Fetch, n: usize) -> Self::Item<'q> {
        fetch
            .0
            .as_ref()
            .map(|l| L::get(generation, l, n), |r| R::get(generation, r, n))
    }
}

unsafe impl<L: QueryShared, R: QueryShared> QueryShared for Or<L, R> {}

#[doc(hidden)]
#[derive(Clone)]
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

/// Transforms query `Q` by skipping entities satisfying query `R`
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
/// let entities = world.query::<Without<(Entity, &i32), &bool>>()
///     .iter()
///     .map(|(e, &i)| (e, i))
///     .collect::<Vec<_>>();
/// assert_eq!(entities, &[(c, 42)]);
/// ```
pub struct Without<Q, R>(PhantomData<(Q, fn(R))>);

impl<Q: Query, R: Query> Query for Without<Q, R> {
    type Item<'q> = Q::Item<'q>;

    type Fetch = FetchWithout<Q::Fetch, R::Fetch>;

    unsafe fn get<'q>(generation: NonZeroU32, fetch: &Self::Fetch, n: usize) -> Self::Item<'q> {
        Q::get(generation, &fetch.0, n)
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

impl<F: Clone, G> Clone for FetchWithout<F, G> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0.clone(), PhantomData)
    }
}

/// Transforms query `Q` by skipping entities not satisfying query `R`
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
/// let entities = world.query::<With<(Entity, &i32), &bool>>()
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

    unsafe fn get<'q>(generation: NonZeroU32, fetch: &Self::Fetch, n: usize) -> Self::Item<'q> {
        Q::get(generation, &fetch.0, n)
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

impl<F: Clone, G> Clone for FetchWith<F, G> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0.clone(), PhantomData)
    }
}

/// A query that matches all entities, yielding `bool`s indicating whether each satisfies query `Q`
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
/// let entities = world.query::<(Entity, Satisfies<&bool>)>()
///     .iter()
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

    unsafe fn get<'q>(_: NonZeroU32, fetch: &Self::Fetch, _: usize) -> Self::Item<'q> {
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

impl<T> Clone for FetchSatisfies<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0, PhantomData)
    }
}

/// A borrow of a [`World`](crate::World) sufficient to execute the query `Q`
///
/// Note that borrows are not released until this object is dropped.
pub struct QueryBorrow<'w, Q: Query> {
    world: &'w World,
    cache: Option<CachedQuery<Q::Fetch>>,
}

impl<'w, Q: Query> QueryBorrow<'w, Q> {
    pub(crate) fn new(world: &'w World) -> Self {
        Self { world, cache: None }
    }

    /// Execute the query
    // The lifetime narrowing here is required for soundness.
    pub fn iter(&mut self) -> QueryIter<'_, Q> {
        let cache = self.borrow().clone();
        unsafe { QueryIter::new(self.world, cache) }
    }

    /// Provide random access to the query results
    pub fn view(&mut self) -> View<'_, Q> {
        let cache = self.borrow().clone();
        unsafe {
            View::new(
                self.world.entities_meta(),
                self.world.archetypes_inner(),
                cache,
            )
        }
    }

    /// Like `iter`, but returns child iterators of at most `batch_size` elements
    ///
    /// Useful for distributing work over a threadpool.
    // The lifetime narrowing here is required for soundness.
    pub fn iter_batched(&mut self, batch_size: u32) -> BatchedIter<'_, Q> {
        let cache = self.borrow().clone();
        unsafe {
            BatchedIter::new(
                self.world.entities_meta(),
                self.world.archetypes_inner(),
                batch_size,
                cache,
            )
        }
    }

    fn borrow(&mut self) -> &CachedQuery<Q::Fetch> {
        self.cache.get_or_insert_with(|| {
            let cache = CachedQuery::get(self.world);
            cache.borrow(self.world.archetypes_inner());
            cache
        })
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
    /// let entities = world.query::<(Entity, &i32)>()
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
    /// let entities = world.query::<(Entity, &i32)>()
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
    fn transform<R: Query>(self) -> QueryBorrow<'w, R> {
        QueryBorrow {
            world: self.world,
            cache: None,
        }
    }
}

unsafe impl<Q: Query> Send for QueryBorrow<'_, Q> where for<'a> Q::Item<'a>: Send {}
unsafe impl<Q: Query> Sync for QueryBorrow<'_, Q> where for<'a> Q::Item<'a>: Send {}

impl<Q: Query> Drop for QueryBorrow<'_, Q> {
    fn drop(&mut self) {
        if let Some(cache) = &self.cache {
            cache.release_borrow(self.world.archetypes_inner());
        }
    }
}

impl<'q, Q: Query> IntoIterator for &'q mut QueryBorrow<'_, Q> {
    type Item = Q::Item<'q>;
    type IntoIter = QueryIter<'q, Q>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Iterator over the set of entities with the components in `Q`
pub struct QueryIter<'q, Q: Query> {
    world: &'q World,
    archetypes: ArchetypeIter<Q>,
    iter: ChunkIter<Q>,
}

impl<'q, Q: Query> QueryIter<'q, Q> {
    /// # Safety
    ///
    /// `'q` must be sufficient to guarantee that `Q` cannot violate borrow safety, either with
    /// dynamic borrow checks or by representing exclusive access to the `World`. `cache` must be
    /// from `world`.
    unsafe fn new(world: &'q World, cache: CachedQuery<Q::Fetch>) -> Self {
        Self {
            world,
            archetypes: ArchetypeIter::new(world, cache),
            iter: ChunkIter::empty(),
        }
    }
}

unsafe impl<Q: Query> Send for QueryIter<'_, Q> where for<'a> Q::Item<'a>: Send {}
unsafe impl<Q: Query> Sync for QueryIter<'_, Q> where for<'a> Q::Item<'a>: Send {}

impl<'q, Q: Query> Iterator for QueryIter<'q, Q> {
    type Item = Q::Item<'q>;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match unsafe { self.iter.next(self.world.entities_meta()) } {
                None => {
                    // Safety: `self.world` is the same one we passed to `ArchetypeIter::new` just above
                    unsafe {
                        self.iter = self.archetypes.next(self.world)?;
                    }
                    continue;
                }
                Some(components) => {
                    return Some(components);
                }
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.len();
        (n, Some(n))
    }
}

impl<Q: Query> ExactSizeIterator for QueryIter<'_, Q> {
    fn len(&self) -> usize {
        self.archetypes.entity_len(self.world) + self.iter.remaining()
    }
}

/// A query builder that's convertible directly into an iterator
pub struct QueryMut<'q, Q: Query> {
    world: &'q mut World,
    _marker: PhantomData<fn() -> Q>,
}

impl<'q, Q: Query> QueryMut<'q, Q> {
    pub(crate) fn new(world: &'q mut World) -> Self {
        assert_borrow::<Q>();
        Self {
            world,
            _marker: PhantomData,
        }
    }

    /// Provide random access to the query results
    pub fn view(&mut self) -> View<'_, Q> {
        let cache = CachedQuery::get(self.world);
        unsafe {
            View::new(
                self.world.entities_meta(),
                self.world.archetypes_inner(),
                cache,
            )
        }
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
            world: self.world,
            _marker: PhantomData,
        }
    }

    /// Like `into_iter`, but returns child iterators of at most `batch_size` elements
    ///
    /// Useful for distributing work over a threadpool.
    pub fn into_iter_batched(self, batch_size: u32) -> BatchedIter<'q, Q> {
        let cache = CachedQuery::get(self.world);
        unsafe {
            BatchedIter::new(
                self.world.entities_meta(),
                self.world.archetypes_inner(),
                batch_size,
                cache,
            )
        }
    }
}

impl<'q, Q: Query> IntoIterator for QueryMut<'q, Q> {
    type Item = <QueryIter<'q, Q> as Iterator>::Item;
    type IntoIter = QueryIter<'q, Q>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        let cache = CachedQuery::get(self.world);
        unsafe { QueryIter::new(self.world, cache) }
    }
}

/// Check that Q doesn't alias a `&mut T` on its own. Currently over-conservative for `Or` queries.
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
    fetch: Q::Fetch,
    position: usize,
    len: usize,
}

impl<Q: Query> ChunkIter<Q> {
    fn new(archetype: &Archetype, fetch: Q::Fetch) -> Self {
        Self {
            fetch,
            position: 0,
            len: archetype.len() as usize,
        }
    }

    fn empty() -> Self {
        Self {
            fetch: Q::Fetch::dangling(),
            position: 0,
            len: 0,
        }
    }

    #[inline]
    unsafe fn next<'a>(&mut self, meta: &[EntityMeta]) -> Option<Q::Item<'a>> {
        if self.position == self.len {
            return None;
        }
        let item = Q::get(
            meta.get_unchecked(self.position).generation,
            &self.fetch,
            self.position,
        );
        self.position += 1;
        Some(item)
    }

    fn remaining(&self) -> usize {
        self.len - self.position
    }
}

/// Batched version of [`QueryIter`]
pub struct BatchedIter<'q, Q: Query> {
    _marker: PhantomData<&'q Q>,
    meta: &'q [EntityMeta],
    archetypes: &'q [Archetype],
    index_iter: core::ops::Range<usize>,
    batch_size: u32,
    cache: CachedQuery<Q::Fetch>,
    batch: u32,
}

impl<'q, Q: Query> BatchedIter<'q, Q> {
    /// # Safety
    ///
    /// `'q` must be sufficient to guarantee that `Q` cannot violate borrow safety, either with
    /// dynamic borrow checks or by representing exclusive access to the `World`.
    unsafe fn new(
        meta: &'q [EntityMeta],
        archetypes: &'q [Archetype],
        batch_size: u32,
        cache: CachedQuery<Q::Fetch>,
    ) -> Self {
        Self {
            _marker: PhantomData,
            meta,
            archetypes,
            index_iter: (0..cache.archetype_count(archetypes)),
            batch_size,
            cache,
            batch: 0,
        }
    }
}

unsafe impl<Q: Query> Send for BatchedIter<'_, Q> where for<'a> Q::Item<'a>: Send {}
unsafe impl<Q: Query> Sync for BatchedIter<'_, Q> where for<'a> Q::Item<'a>: Send {}

impl<'q, Q: Query> Iterator for BatchedIter<'q, Q> {
    type Item = Batch<'q, Q>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let mut indices = self.index_iter.clone();
            let index = indices.next()?;
            let Some((archetype, state)) =
                (unsafe { self.cache.get_state(self.archetypes, index) })
            else {
                // Skip this archetype entirely
                self.index_iter = indices;
                continue;
            };
            let offset = self.batch_size * self.batch;
            if offset >= archetype.len() {
                // We've yielded the contents of this archetype already
                self.index_iter = indices;
                self.batch = 0;
                continue;
            }
            let fetch = Q::Fetch::execute(archetype, state);
            self.batch += 1;
            let mut state = ChunkIter::new(archetype, fetch);
            state.position = offset as usize;
            state.len = (offset + self.batch_size.min(archetype.len() - offset)) as usize;
            return Some(Batch {
                meta: self.meta,
                state,
            });
        }
    }
}

/// A sequence of entities yielded by [`BatchedIter`]
pub struct Batch<'q, Q: Query> {
    meta: &'q [EntityMeta],
    state: ChunkIter<Q>,
}

impl<'q, Q: Query> Iterator for Batch<'q, Q> {
    type Item = Q::Item<'q>;

    fn next(&mut self) -> Option<Self::Item> {
        unsafe { self.state.next(self.meta) }
    }
}

unsafe impl<Q: Query> Send for Batch<'_, Q> where for<'a> Q::Item<'a>: Send {}
unsafe impl<Q: Query> Sync for Batch<'_, Q> where for<'a> Q::Item<'a>: Send {}

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
            #[cold]
            fn prepare(archetype: &Archetype) -> Option<Self::State> {
                Some(($($name::prepare(archetype)?,)*))
            }
            #[allow(unused_variables, non_snake_case, clippy::unused_unit)]
            #[inline(always)]
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
            unsafe fn get<'q>(generation: NonZeroU32, fetch: &Self::Fetch, n: usize) -> Self::Item<'q> {
                #[allow(non_snake_case)]
                let ($(ref $name,)*) = *fetch;
                ($($name::get(generation, $name, n),)*)
            }
        }

        unsafe impl<$($name: QueryShared),*> QueryShared for ($($name,)*) {}
    };
}

//smaller_tuples_too!(tuple_impl, B, A);
smaller_tuples_too!(tuple_impl, O, N, M, L, K, J, I, H, G, F, E, D, C, B, A);

/// A prepared query can be stored independently of the [`World`] to fractionally reduce query
/// set-up costs.
///
/// Prepared queries are less convenient and usually do not measurably impact performance. Regular
/// queries should be preferred unless end-to-end performance measurements clearly indicate
/// otherwise.
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

        unsafe { PreparedQueryIter::new(meta, archetypes, self.state.iter()) }
    }

    /// Provide random access to query results for a uniquely borrow world
    pub fn view_mut<'q>(&'q mut self, world: &'q mut World) -> PreparedView<'q, Q> {
        assert_borrow::<Q>();

        if self.memo != world.memo() {
            *self = Self::prepare(world);
        }

        let meta = world.entities_meta();
        let archetypes = world.archetypes_inner();

        unsafe { PreparedView::new(meta, archetypes, self.state.iter(), &mut self.fetch) }
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
    pub fn iter(&mut self) -> PreparedQueryIter<'_, Q> {
        unsafe { PreparedQueryIter::new(self.meta, self.archetypes, self.state.iter()) }
    }

    /// Provides random access to the results of the prepared query
    pub fn view(&mut self) -> PreparedView<'_, Q> {
        unsafe { PreparedView::new(self.meta, self.archetypes, self.state.iter(), self.fetch) }
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

unsafe impl<Q: Query> Send for PreparedQueryIter<'_, Q> where for<'a> Q::Item<'a>: Send {}
unsafe impl<Q: Query> Sync for PreparedQueryIter<'_, Q> where for<'a> Q::Item<'a>: Send {}

impl<'q, Q: Query> Iterator for PreparedQueryIter<'q, Q> {
    type Item = Q::Item<'q>;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match unsafe { self.iter.next(self.meta) } {
                None => {
                    let (idx, state) = self.state.next()?;
                    let archetype = &self.archetypes[*idx];
                    self.iter = ChunkIter::new(archetype, Q::Fetch::execute(archetype, *state));
                    continue;
                }
                Some(components) => {
                    return Some(components);
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
///
/// Views borrow all components that they expose, from when the view is created until it's
/// dropped. As with any borrowing pattern, they should usually be short-lived to avoid conflicts
/// with distant code that might want to borrow the same components.
pub struct View<'q, Q: Query> {
    meta: &'q [EntityMeta],
    archetypes: &'q [Archetype],
    fetch: Box<[Option<Q::Fetch>]>,
}

unsafe impl<Q: Query> Send for View<'_, Q> where for<'a> Q::Item<'a>: Send {}
unsafe impl<Q: Query> Sync for View<'_, Q> where for<'a> Q::Item<'a>: Send {}

impl<'q, Q: Query> View<'q, Q> {
    /// # Safety
    ///
    /// `'q` must be sufficient to guarantee that `Q` cannot violate borrow safety, either with
    /// dynamic borrow checks or by representing exclusive access to the `World`.
    pub(crate) unsafe fn new(
        meta: &'q [EntityMeta],
        archetypes: &'q [Archetype],
        cache: CachedQuery<Q::Fetch>,
    ) -> Self {
        Self {
            meta,
            archetypes,
            fetch: cache.fetch_all(archetypes),
        }
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
            .map(|fetch| unsafe { Q::get(entity.generation, fetch, meta.location.index as usize) })
    }

    /// Retrieve the query results corresponding to `entity`
    ///
    /// Will yield `None` if the entity does not exist or does not match the query.
    pub fn get_mut(&mut self, entity: Entity) -> Option<Q::Item<'_>> {
        unsafe { self.get_unchecked(entity) }
    }

    /// Equivalent to `get(entity).is_some()`, but does not require `Q: QueryShared`
    pub fn contains(&self, entity: Entity) -> bool {
        let Some(meta) = self.meta.get(entity.id as usize) else {
            return false;
        };
        if meta.generation != entity.generation {
            return false;
        }
        self.fetch[meta.location.archetype as usize].is_some()
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
            .map(|fetch| Q::get(entity.generation, fetch, meta.location.index as usize))
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
    pub fn get_many_mut<const N: usize>(
        &mut self,
        entities: [Entity; N],
    ) -> [Option<Q::Item<'_>>; N] {
        assert_distinct(&entities);

        let mut items = [(); N].map(|()| None);

        for (item, entity) in items.iter_mut().zip(entities) {
            unsafe {
                *item = self.get_unchecked(entity);
            }
        }

        items
    }

    #[doc(hidden)]
    #[deprecated(since = "0.10.5", note = "renamed to `get_many_mut`")]
    pub fn get_mut_n<const N: usize>(&mut self, entities: [Entity; N]) -> [Option<Q::Item<'_>>; N] {
        self.get_many_mut(entities)
    }

    /// Iterate over all entities satisfying `Q`
    ///
    /// Equivalent to [`QueryBorrow::iter`].
    pub fn iter_mut(&mut self) -> ViewIter<'_, Q> {
        ViewIter {
            meta: self.meta,
            archetypes: self.archetypes.iter(),
            fetches: self.fetch.iter(),
            iter: ChunkIter::empty(),
        }
    }
}

impl<'a, Q: Query> IntoIterator for &'a mut View<'_, Q> {
    type IntoIter = ViewIter<'a, Q>;
    type Item = Q::Item<'a>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

pub struct ViewIter<'a, Q: Query> {
    meta: &'a [EntityMeta],
    archetypes: SliceIter<'a, Archetype>,
    fetches: SliceIter<'a, Option<Q::Fetch>>,
    iter: ChunkIter<Q>,
}

impl<'a, Q: Query> Iterator for ViewIter<'a, Q> {
    type Item = Q::Item<'a>;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match unsafe { self.iter.next(self.meta) } {
                None => {
                    let archetype = self.archetypes.next()?;
                    let fetch = self.fetches.next()?;
                    self.iter = fetch
                        .clone()
                        .map_or(ChunkIter::empty(), |fetch| ChunkIter::new(archetype, fetch));
                    continue;
                }
                Some(components) => {
                    return Some(components);
                }
            }
        }
    }
}

/// Provides random access to the results of a prepared query
pub struct PreparedView<'q, Q: Query> {
    meta: &'q [EntityMeta],
    archetypes: &'q [Archetype],
    fetch: &'q mut [Option<Q::Fetch>],
}

unsafe impl<Q: Query> Send for PreparedView<'_, Q> where for<'a> Q::Item<'a>: Send {}
unsafe impl<Q: Query> Sync for PreparedView<'_, Q> where for<'a> Q::Item<'a>: Send {}

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

        Self {
            meta,
            archetypes,
            fetch,
        }
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
            .map(|fetch| unsafe { Q::get(entity.generation, fetch, meta.location.index as usize) })
    }

    /// Retrieve the query results corresponding to `entity`
    ///
    /// Will yield `None` if the entity does not exist or does not match the query.
    pub fn get_mut(&mut self, entity: Entity) -> Option<Q::Item<'_>> {
        unsafe { self.get_unchecked(entity) }
    }

    /// Equivalent to `get(entity).is_some()`, but does not require `Q: QueryShared`
    pub fn contains(&self, entity: Entity) -> bool {
        let Some(meta) = self.meta.get(entity.id as usize) else {
            return false;
        };
        if meta.generation != entity.generation {
            return false;
        }
        self.fetch[meta.location.archetype as usize].is_some()
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
            .map(|fetch| Q::get(entity.generation, fetch, meta.location.index as usize))
    }

    /// Like `get_mut`, but allows checked simultaneous access to multiple entities
    ///
    /// See [`View::get_many_mut`] for details.
    pub fn get_many_mut<const N: usize>(
        &mut self,
        entities: [Entity; N],
    ) -> [Option<Q::Item<'_>>; N] {
        assert_distinct(&entities);

        let mut items = [(); N].map(|()| None);

        for (item, entity) in items.iter_mut().zip(entities) {
            unsafe {
                *item = self.get_unchecked(entity);
            }
        }

        items
    }

    #[doc(hidden)]
    #[deprecated(since = "0.10.5", note = "renamed to `get_many_mut`")]
    pub fn get_mut_n<const N: usize>(&mut self, entities: [Entity; N]) -> [Option<Q::Item<'_>>; N] {
        self.get_many_mut(entities)
    }

    /// Iterate over all entities satisfying `Q`
    ///
    /// Equivalent to [`PreparedQueryBorrow::iter`].
    pub fn iter_mut(&mut self) -> ViewIter<'_, Q> {
        ViewIter {
            meta: self.meta,
            archetypes: self.archetypes.iter(),
            fetches: self.fetch.iter(),
            iter: ChunkIter::empty(),
        }
    }
}

impl<'a, Q: Query> IntoIterator for &'a mut PreparedView<'_, Q> {
    type IntoIter = ViewIter<'a, Q>;
    type Item = Q::Item<'a>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

/// A borrow of a [`World`](crate::World) sufficient to random-access the results of the query `Q`.
///
/// Note that borrows are not released until this object is dropped.
///
/// This struct is a thin wrapper around [`View`]. See it for more documentation.
pub struct ViewBorrow<'w, Q: Query> {
    view: View<'w, Q>,
    cache: CachedQuery<Q::Fetch>,
}

impl<'w, Q: Query> ViewBorrow<'w, Q> {
    pub(crate) fn new(world: &'w World) -> Self {
        let cache = CachedQuery::get(world);
        cache.borrow(world.archetypes_inner());
        let view = unsafe {
            View::<Q>::new(
                world.entities_meta(),
                world.archetypes_inner(),
                cache.clone(),
            )
        };

        Self { view, cache }
    }

    /// Retrieve the query results corresponding to `entity`
    ///
    /// Will yield `None` if the entity does not exist or does not match the query.
    ///
    /// Does not require exclusive access to the map, but is defined only for queries yielding only shared references.
    ///
    /// See [`View::get``].
    pub fn get(&self, entity: Entity) -> Option<Q::Item<'_>>
    where
        Q: QueryShared,
    {
        self.view.get(entity)
    }

    /// Retrieve the query results corresponding to `entity`
    ///
    /// Will yield `None` if the entity does not exist or does not match the query.
    ///
    /// See [`View::get_mut``].
    pub fn get_mut(&mut self, entity: Entity) -> Option<Q::Item<'_>> {
        self.view.get_mut(entity)
    }

    /// Equivalent to `get(entity).is_some()`, but does not require `Q: QueryShared`
    ///
    /// See [`View::contains``].
    pub fn contains(&self, entity: Entity) -> bool {
        self.view.contains(entity)
    }

    /// Like `get_mut`, but allows simultaneous access to multiple entities
    ///
    /// See [`View::get_unchecked``].
    ///
    /// # Safety
    ///
    /// Must not be invoked while any unique borrow of the fetched components of `entity` is live.
    pub unsafe fn get_unchecked(&self, entity: Entity) -> Option<Q::Item<'_>> {
        self.view.get_unchecked(entity)
    }

    /// Like `get_mut`, but allows checked simultaneous access to multiple entities
    ///
    /// For N > 3, the check for distinct entities will clone the array and take O(N log N) time.
    ///
    /// See [`View::get_many_mut``].
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
    /// let mut view = world.view_mut::<&mut i32>();
    /// let [a, b, c] = view.get_mut_n([a, b, c]);
    ///
    /// assert_eq!(*a.unwrap(), 1);
    /// assert_eq!(*b.unwrap(), 2);
    /// assert_eq!(*c.unwrap(), 3);
    /// ```
    pub fn get_many_mut<const N: usize>(
        &mut self,
        entities: [Entity; N],
    ) -> [Option<Q::Item<'_>>; N] {
        self.view.get_many_mut(entities)
    }

    /// Iterate over all entities satisfying `Q`
    ///
    /// See [`View::iter_mut``]
    pub fn iter_mut(&mut self) -> ViewIter<'_, Q> {
        self.view.iter_mut()
    }
}

impl<Q: Query> Drop for ViewBorrow<'_, Q> {
    fn drop(&mut self) {
        self.cache.release_borrow(self.view.archetypes);
    }
}

impl<'a, Q: Query> IntoIterator for &'a mut ViewBorrow<'_, Q> {
    type IntoIter = ViewIter<'a, Q>;
    type Item = Q::Item<'a>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

pub(crate) fn assert_distinct<const N: usize>(entities: &[Entity; N]) {
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

#[cfg(feature = "std")]
pub(crate) type QueryCache = RwLock<TypeIdMap<Arc<dyn Any + Send + Sync>>>;

#[cfg(feature = "std")]
struct CachedQueryInner<F: Fetch> {
    state: Box<[(usize, F::State)]>,
    // In theory we could drop the cache eagerly when invalidated rather than tracking this, but
    // this is harder to screw up.
    archetypes_generation: crate::ArchetypesGeneration,
}

#[cfg(feature = "std")]
impl<F: Fetch> CachedQueryInner<F> {
    fn new(world: &World) -> Self {
        Self {
            state: world
                .archetypes()
                .enumerate()
                .filter_map(|(idx, x)| F::prepare(x).map(|state| (idx, state)))
                .collect(),
            archetypes_generation: world.archetypes_generation(),
        }
    }
}

pub(crate) struct CachedQuery<F: Fetch> {
    #[cfg(feature = "std")]
    inner: Arc<CachedQueryInner<F>>,
    #[cfg(not(feature = "std"))]
    _marker: PhantomData<F>,
}

impl<F: Fetch> CachedQuery<F> {
    pub(crate) fn get(world: &World) -> Self {
        #[cfg(feature = "std")]
        {
            let existing_cache = world
                .query_cache()
                .read()
                .unwrap()
                .get(&TypeId::of::<F>())
                .map(|x| Arc::downcast::<CachedQueryInner<F>>(x.clone()).unwrap())
                .filter(|x| x.archetypes_generation == world.archetypes_generation());
            let inner = existing_cache.unwrap_or_else(
                #[cold]
                || {
                    let mut cache = world.query_cache().write().unwrap();
                    let entry = cache.entry(TypeId::of::<F>());
                    let cached = match entry {
                        hash_map::Entry::Vacant(e) => {
                            let fresh = Arc::new(CachedQueryInner::<F>::new(world));
                            e.insert(fresh.clone());
                            fresh
                        }
                        hash_map::Entry::Occupied(mut e) => {
                            let value =
                                Arc::downcast::<CachedQueryInner<F>>(e.get().clone()).unwrap();
                            match value.archetypes_generation == world.archetypes_generation() {
                                false => {
                                    let fresh = Arc::new(CachedQueryInner::<F>::new(world));
                                    e.insert(fresh.clone());
                                    fresh
                                }
                                true => value,
                            }
                        }
                    };
                    cached
                },
            );
            Self { inner }
        }
        #[cfg(not(feature = "std"))]
        {
            _ = world;
            Self {
                _marker: PhantomData,
            }
        }
    }

    fn archetype_count(&self, archetypes: &[Archetype]) -> usize {
        #[cfg(feature = "std")]
        {
            _ = archetypes;
            self.inner.state.len()
        }
        #[cfg(not(feature = "std"))]
        {
            archetypes.len()
        }
    }

    /// Returns `None` if this index should be skipped.
    ///
    /// # Safety
    /// - `index` must be <= the value returned by `archetype_count`
    /// - `archetypes` must match that passed to `archetype_count` and the world passed to `get`
    unsafe fn get_state<'a>(
        &self,
        archetypes: &'a [Archetype],
        index: usize,
    ) -> Option<(&'a Archetype, F::State)> {
        #[cfg(feature = "std")]
        unsafe {
            let &(archetype, state) = self.inner.state.get_unchecked(index);
            let archetype = archetypes.get_unchecked(archetype);
            Some((archetype, state))
        }
        #[cfg(not(feature = "std"))]
        {
            let archetype = unsafe { archetypes.get_unchecked(index) };
            let state = F::prepare(archetype)?;
            Some((archetype, state))
        }
    }

    /// Returns `None` if this index should be skipped.
    ///
    /// # Safety
    /// - `index` must be <= the value returned by `archetype_count`
    /// - `archetypes` must match that passed to `archetype_count` and the world passed to `get`
    unsafe fn get_archetype<'a>(
        &self,
        archetypes: &'a [Archetype],
        index: usize,
    ) -> Option<&'a Archetype> {
        #[cfg(feature = "std")]
        unsafe {
            let &(archetype, _) = self.inner.state.get_unchecked(index);
            let archetype = archetypes.get_unchecked(archetype);
            Some(archetype)
        }
        #[cfg(not(feature = "std"))]
        {
            let x = unsafe { archetypes.get_unchecked(index) };
            if F::access(x).is_none() {
                return None;
            }
            Some(x)
        }
    }

    fn borrow(&self, archetypes: &[Archetype]) {
        #[cfg(feature = "std")]
        {
            for &(archetype, state) in &self.inner.state {
                let archetype = unsafe { archetypes.get_unchecked(archetype) };
                if archetype.is_empty() {
                    continue;
                }
                F::borrow(archetype, state);
            }
        }

        #[cfg(not(feature = "std"))]
        {
            for x in archetypes {
                if x.is_empty() {
                    continue;
                }
                // TODO: Release prior borrows on failure?
                if let Some(state) = F::prepare(x) {
                    F::borrow(x, state);
                }
            }
        }
    }

    fn release_borrow(&self, archetypes: &[Archetype]) {
        #[cfg(feature = "std")]
        {
            for &(archetype, state) in &self.inner.state {
                let archetype = unsafe { archetypes.get_unchecked(archetype) };
                if archetype.is_empty() {
                    continue;
                }
                F::release(archetype, state);
            }
        }

        #[cfg(not(feature = "std"))]
        {
            for x in archetypes {
                if x.is_empty() {
                    continue;
                }
                if let Some(state) = F::prepare(x) {
                    F::release(x, state);
                }
            }
        }
    }

    fn fetch_all(&self, archetypes: &[Archetype]) -> Box<[Option<F>]> {
        #[cfg(feature = "std")]
        {
            let mut fetch = (0..archetypes.len()).map(|_| None).collect::<Box<[_]>>();
            for &(archetype_index, state) in &self.inner.state {
                let archetype = &archetypes[archetype_index];
                fetch[archetype_index] = Some(F::execute(archetype, state));
            }
            fetch
        }
        #[cfg(not(feature = "std"))]
        {
            archetypes
                .iter()
                .map(|archetype| F::prepare(archetype).map(|state| F::execute(archetype, state)))
                .collect()
        }
    }
}

impl<F: Fetch> Clone for CachedQuery<F> {
    fn clone(&self) -> Self {
        Self {
            #[cfg(feature = "std")]
            inner: self.inner.clone(),
            #[cfg(not(feature = "std"))]
            _marker: PhantomData,
        }
    }
}

struct ArchetypeIter<Q: Query> {
    archetypes: core::ops::Range<usize>,
    cache: CachedQuery<Q::Fetch>,
}

impl<Q: Query> ArchetypeIter<Q> {
    fn new(world: &World, cache: CachedQuery<Q::Fetch>) -> Self {
        Self {
            archetypes: 0..cache.archetype_count(world.archetypes_inner()),
            cache,
        }
    }

    /// Safety: `world` must be the same as passed to `new`
    unsafe fn next(&mut self, world: &World) -> Option<ChunkIter<Q>> {
        loop {
            let Some((archetype, state)) = self
                .cache
                .get_state(world.archetypes_inner(), self.archetypes.next()?)
            else {
                continue;
            };
            let fetch = Q::Fetch::execute(archetype, state);
            return Some(ChunkIter::new(archetype, fetch));
        }
    }

    fn entity_len(&self, world: &World) -> usize {
        self.archetypes
            .clone()
            .filter_map(|x| unsafe { self.cache.get_archetype(world.archetypes_inner(), x) })
            .map(|x| x.len() as usize)
            .sum()
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
