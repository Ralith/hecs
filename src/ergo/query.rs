use core::cell::RefCell;
use core::slice::Iter as SliceIter;
use core::{marker::PhantomData, ptr::NonNull};

use alloc::rc::Rc;

use crate::{entities::EntityMeta, Archetype, Component, Entity};
use crate::{ComponentError, ErgoScope, MissingComponent, TypeInfo};

use super::access::ComponentRef;
use super::scope::ActiveQueryState;

/// Errors that arise when fetching
#[derive(Debug, Clone, Eq, PartialEq)]
#[doc(hidden)]
pub enum FetchError {
    /// The entity was already despawned
    NoSuchEntity,
    /// The entity did not have a requested component
    MissingComponent(MissingComponent),
    /// The entity no longer matches the query, happens with Without
    InvalidMatch,
}

impl From<ComponentError> for FetchError {
    fn from(e: ComponentError) -> Self {
        match e {
            ComponentError::NoSuchEntity => Self::NoSuchEntity,
            ComponentError::MissingComponent(c) => Self::MissingComponent(c),
        }
    }
}

/// A collection of component types to fetch from a [`World`](crate::World)
///
/// The interface of this trait is a private implementation detail.
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

    /// Look up state for `archetype` if it should be traversed
    fn prepare(archetype: &Archetype) -> Option<Self::State>;
    /// Construct a `Fetch` for `archetype` based on the associated state
    fn execute(archetype: &'a Archetype, state: Self::State) -> Self;

    /// Access the `n`th item in this archetype without bounds checking
    ///
    /// # Safety
    /// - Must only be called after `borrow`
    /// - `release` must not be called while `'a` is still live
    /// - Bounds-checking must be performed externally
    /// - Any resulting borrows must be legal (e.g. no &mut to something another iterator might access)
    unsafe fn get_in_world(&self, scope: &ErgoScope, n: usize, entity: Entity) -> Self::Item;

    unsafe fn get_overridden(scope: &ErgoScope, entity: Entity) -> Result<Self::Item, FetchError>;
}

impl<'a, T: Component> Query for &'a T {
    type Fetch = FetchGet<T>;
}

impl<'a, T: Component> Query for &'a mut T {
    type Fetch = FetchGet<T>;
}

#[doc(hidden)]
pub struct FetchGet<T> {
    data_base: NonNull<T>,
    entities_base: NonNull<u32>,
    type_info: TypeInfo,
}

unsafe impl<'a, T: Component> Fetch<'a> for FetchGet<T> {
    type Item = ComponentRef<T>;

    type State = usize;

    fn dangling() -> Self {
        Self {
            data_base: NonNull::dangling(),
            entities_base: NonNull::dangling(),
            type_info: TypeInfo::of::<T>(),
        }
    }

    fn prepare(archetype: &Archetype) -> Option<Self::State> {
        archetype.get_state::<T>()
    }
    fn execute(archetype: &'a Archetype, state: Self::State) -> Self {
        unsafe {
            Self {
                data_base: archetype.get_base(state),
                entities_base: NonNull::new_unchecked(archetype.entities().as_ptr()),
                type_info: TypeInfo::of::<T>(),
            }
        }
    }

    unsafe fn get_in_world(&self, scope: &ErgoScope, n: usize, entity: Entity) -> Self::Item {
        let data_ptr = self.data_base.as_ptr().add(n);
        let entity_id = *self.entities_base.as_ptr().add(n);
        let entity = scope.world.entities().resolve_unknown_gen(entity_id);
        scope.access.get_typed_component_ref(
            entity,
            &self.type_info,
            NonNull::new_unchecked(data_ptr.cast()),
        )
    }

    unsafe fn get_overridden(scope: &ErgoScope, entity: Entity) -> Result<Self::Item, FetchError> {
        scope.get_overriden(entity).map_err(|e| e.into())
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

    fn prepare(archetype: &Archetype) -> Option<Self::State> {
        Some(T::prepare(archetype))
    }
    fn execute(archetype: &'a Archetype, state: Self::State) -> Self {
        Self(state.map(|state| T::execute(archetype, state)))
    }

    unsafe fn get_in_world(&self, scope: &ErgoScope, n: usize, entity: Entity) -> Option<T::Item> {
        Some(self.0.as_ref()?.get_in_world(scope, n, entity))
    }

    unsafe fn get_overridden(
        scope: &ErgoScope,
        entity: Entity,
    ) -> Result<Option<T::Item>, FetchError> {
        match T::get_overridden(scope, entity) {
            Ok(item) => Ok(Some(item)),
            Err(FetchError::MissingComponent(..)) => Ok(None),
            Err(err) => Err(err),
        }
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

    fn prepare(archetype: &Archetype) -> Option<Self::State> {
        Or::new(L::prepare(archetype), R::prepare(archetype))
    }

    fn execute(archetype: &'a Archetype, state: Self::State) -> Self {
        Self(state.map(|l| L::execute(archetype, l), |r| R::execute(archetype, r)))
    }

    unsafe fn get_in_world(&self, scope: &ErgoScope, n: usize, entity: Entity) -> Self::Item {
        self.0.as_ref().map(
            |l| l.get_in_world(scope, n, entity),
            |r| r.get_in_world(scope, n, entity),
        )
    }

    unsafe fn get_overridden(scope: &ErgoScope, entity: Entity) -> Result<Self::Item, FetchError> {
        let left = L::get_overridden(scope, entity);
        let right = R::get_overridden(scope, entity);
        match (left, right) {
            (Ok(item), Err(_)) => Ok(Or::Left(item)),
            (Ok(left), Ok(right)) => Ok(Or::Both(left, right)),
            (Err(..), Ok(right)) => Ok(Or::Right(right)),
            (Err(e), Err(_)) => Err(e),
        }
    }
}

/// Query transformer skipping entities that have a `T` component
///
/// See also `QueryBorrow::without`.
///
/// # Example
/// ```
/// # use hecs::ergo::*;
/// let mut world = World::new();
/// let a = world.spawn((123, true, "abc"));
/// let b = world.spawn((456, false));
/// let c = world.spawn((42, "def"));
/// let ergo = ErgoScope::new(&mut world);
/// let entities = ergo.query::<Without<&i32, &bool>>()
///     .iter()
///     .map(|(e, i)| (e, *i.read()))
///     .collect::<Vec<_>>();
/// assert_eq!(entities, &[(c, 42)]);
/// ```
pub struct Without<Q, R>(PhantomData<(Q, fn(R))>);

impl<Q: Query, R: Query> Query for Without<Q, R> {
    type Fetch = FetchWithout<Q::Fetch, R::Fetch>;
}

#[doc(hidden)]
pub struct FetchWithout<F, G>(F, PhantomData<fn(G)>);

unsafe impl<'a, F: Fetch<'a>, G: Fetch<'a>> Fetch<'a> for FetchWithout<F, G> {
    type Item = F::Item;

    type State = F::State;

    fn dangling() -> Self {
        Self(F::dangling(), PhantomData)
    }

    fn prepare(archetype: &Archetype) -> Option<Self::State> {
        if G::prepare(archetype).is_some() {
            return None;
        }
        F::prepare(archetype)
    }
    fn execute(archetype: &'a Archetype, state: Self::State) -> Self {
        Self(F::execute(archetype, state), PhantomData)
    }

    unsafe fn get_in_world(&self, scope: &ErgoScope, n: usize, entity: Entity) -> F::Item {
        self.0.get_in_world(scope, n, entity)
    }

    unsafe fn get_overridden(scope: &ErgoScope, entity: Entity) -> Result<Self::Item, FetchError> {
        F::get_overridden(scope, entity)
    }
}

/// Query transformer skipping entities that do not have a `T` component
///
/// See also `QueryBorrow::with`.
///
/// # Example
/// ```
/// # use hecs::ergo::*;
/// let mut world = World::new();
/// let a = world.spawn((123, true, "abc"));
/// let b = world.spawn((456, false));
/// let c = world.spawn((42, "def"));
/// let ergo = ErgoScope::new(&mut world);
/// let entities = ergo.query::<With<&i32, &bool>>()
///     .iter()
///     .map(|(e, i)| (e, *i.read()))
///     .collect::<Vec<_>>();
/// assert_eq!(entities.len(), 2);
/// assert!(entities.contains(&(a, 123)));
/// assert!(entities.contains(&(b, 456)));
/// ```
pub struct With<Q, R>(PhantomData<(Q, fn(R))>);

impl<Q: Query, R: Query> Query for With<Q, R> {
    type Fetch = FetchWith<Q::Fetch, R::Fetch>;
}

#[doc(hidden)]
pub struct FetchWith<F, G>(F, PhantomData<fn(G)>);

unsafe impl<'a, F: Fetch<'a>, G: Fetch<'a>> Fetch<'a> for FetchWith<F, G> {
    type Item = F::Item;

    type State = F::State;

    fn dangling() -> Self {
        Self(F::dangling(), PhantomData)
    }

    fn prepare(archetype: &Archetype) -> Option<Self::State> {
        G::prepare(archetype)?;
        F::prepare(archetype)
    }
    fn execute(archetype: &'a Archetype, state: Self::State) -> Self {
        Self(F::execute(archetype, state), PhantomData)
    }

    unsafe fn get_in_world(&self, scope: &ErgoScope, n: usize, entity: Entity) -> F::Item {
        self.0.get_in_world(scope, n, entity)
    }

    unsafe fn get_overridden(scope: &ErgoScope, entity: Entity) -> Result<Self::Item, FetchError> {
        F::get_overridden(scope, entity)
    }
}

/// A query that yields `true` iff an entity would satisfy the query `Q`
///
/// Does not borrow any components, making it faster and more concurrency-friendly than `Option<Q>`.
///
/// # Example
/// ```
/// # use hecs::ergo::*;
/// let mut world = World::new();
/// let a = world.spawn((123, true, "abc"));
/// let b = world.spawn((456, false));
/// let c = world.spawn((42, "def"));
/// let ergo = ErgoScope::new(&mut world);
/// let entities = ergo.query::<Satisfies<&bool>>()
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

    fn prepare(archetype: &Archetype) -> Option<Self::State> {
        Some(F::prepare(archetype).is_some())
    }
    fn execute(_archetype: &'a Archetype, state: Self::State) -> Self {
        Self(state, PhantomData)
    }

    unsafe fn get_in_world(&self, _: &ErgoScope, _: usize, _: Entity) -> bool {
        self.0
    }

    unsafe fn get_overridden(scope: &ErgoScope, entity: Entity) -> Result<Self::Item, FetchError> {
        F::get_overridden(scope, entity).map(|v| true)
    }
}

/// A borrow of a [`World`](crate::World) sufficient to execute the query `Q`
///
/// Note that borrows are not released until this object is dropped.
pub struct QueryBorrow<'w, Q: Query> {
    scope: &'w ErgoScope<'w>,
    meta: &'w [EntityMeta],
    archetypes: &'w [Archetype],
    query_state: Rc<RefCell<ActiveQueryState>>,
    _marker: PhantomData<Q>,
}

impl<'w, Q: Query> QueryBorrow<'w, Q> {
    pub(crate) fn new(
        scope: &'w ErgoScope<'w>,
        meta: &'w [EntityMeta],
        archetypes: &'w [Archetype],
    ) -> Self {
        Self {
            scope,
            meta,
            archetypes,
            query_state: scope.alloc_query_state(),
            _marker: PhantomData,
        }
    }

    /// Execute the query
    // The lifetime narrowing here is required for soundness.
    pub fn iter(&mut self) -> QueryIter<'_, Q> {
        unsafe {
            QueryIter::new(
                self.scope,
                self.meta,
                self.archetypes.iter(),
                self.query_state.clone(),
            )
        }
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
    /// # use hecs::ergo::*;
    /// let mut world = World::new();
    /// let a = world.spawn((123, true, "abc"));
    /// let b = world.spawn((456, false));
    /// let c = world.spawn((42, "def"));
    /// let ergo = ErgoScope::new(&mut world);
    /// let entities = ergo.query::<&i32>()
    ///     .with::<&bool>()
    ///     .iter()
    ///     .map(|(e, i)| (e, *i.read())) // Copy out of the world
    ///     .collect::<Vec<_>>();
    /// assert!(entities.contains(&(a, 123)));
    /// assert!(entities.contains(&(b, 456)));
    /// ```
    pub fn with<R: Query>(self) -> QueryBorrow<'w, With<Q, R>> {
        self.transform()
    }

    /// Transform the query into one that skips entities having a certain component
    ///
    /// Equivalent to using a query type wrapped in `Without`.
    ///
    /// # Example
    /// ```
    /// # use hecs::ergo::*;
    /// let mut world = World::new();
    /// let a = world.spawn((123, true, "abc"));
    /// let b = world.spawn((456, false));
    /// let c = world.spawn((42, "def"));
    /// let ergo = ErgoScope::new(&mut world);
    /// let entities = ergo.query::<&i32>()
    ///     .without::<&bool>()
    ///     .iter()
    ///     .map(|(e, i)| (e, *i.read())) // Copy out of the world
    ///     .collect::<Vec<_>>();
    /// assert_eq!(entities, &[(c, 42)]);
    /// ```
    pub fn without<R: Query>(self) -> QueryBorrow<'w, Without<Q, R>> {
        self.transform()
    }

    // TODO implement
    /// Determine whether this entity would satisfy the query `Q`
    // pub fn satisfies<R: Query>(&self) -> bool {
    //     R::Fetch::prepare(self.archetype).is_some()
    // }

    /// Helper to change the type of the query
    fn transform<R: Query>(self) -> QueryBorrow<'w, R> {
        QueryBorrow {
            scope: self.scope,
            meta: self.meta,
            archetypes: self.archetypes,
            query_state: self.query_state,
            _marker: PhantomData,
        }
    }
}

impl<'q, 'w: 'q, Q: Query> IntoIterator for &'q mut QueryBorrow<'w, Q> {
    type Item = (Entity, QueryItem<'q, Q>);
    type IntoIter = QueryIter<'q, Q>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Iterator over the set of entities with the components in `Q`
pub struct QueryIter<'q, Q: Query> {
    scope: &'q ErgoScope<'q>,
    meta: &'q [EntityMeta],
    archetypes: SliceIter<'q, Archetype>,
    iter: ChunkIter<Q>,
    query_state: Rc<RefCell<ActiveQueryState>>,
}

impl<'q, Q: Query> QueryIter<'q, Q> {
    /// # Safety
    ///
    /// `'q` must be sufficient to guarantee that `Q` cannot violate borrow safety, either with
    /// dynamic borrow checks or by representing exclusive access to the `World`.
    unsafe fn new(
        scope: &'q ErgoScope<'q>,
        meta: &'q [EntityMeta],
        archetypes: SliceIter<'q, Archetype>,
        query_state: Rc<RefCell<ActiveQueryState>>,
    ) -> Self {
        let f: fn(&ErgoScope, Entity) -> bool =
            |scope, entity| <<Q as Query>::Fetch as Fetch>::get_overridden(scope, entity).is_ok();
        query_state.borrow_mut().entity_match_fn = Some(f);
        Self {
            scope,
            meta,
            archetypes,
            iter: ChunkIter::empty(),
            query_state,
        }
    }
}

impl<'q, Q: Query> Iterator for QueryIter<'q, Q> {
    type Item = (Entity, QueryItem<'q, Q>);

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match unsafe { self.iter.next(self.scope, &self.query_state) } {
                None => {
                    if let Some(archetype) = self.archetypes.next() {
                        let state = Q::Fetch::prepare(archetype);
                        let fetch = state.map(|state| Q::Fetch::execute(archetype, state));
                        self.iter = fetch.map_or(ChunkIter::empty(), |fetch| ChunkIter {
                            entities: archetype.entities(),
                            fetch,
                            position: 0,
                            len: archetype.len() as usize,
                        });
                        let mut query_state = self.query_state.borrow_mut();
                        let archetype_ptr: *const Archetype = archetype;
                        // Safety: the pointers are from the same linear archetype storage
                        query_state.archetype_idx = unsafe {
                            archetype_ptr.offset_from(self.scope.world.archetypes_inner().as_ptr())
                                as u32
                        };
                        query_state.archetype_iter_pos = 0;
                        continue;
                    } else {
                        let mut query_state = self.query_state.borrow_mut();
                        query_state.archetype_idx = self.archetypes.len() as u32;
                        // done iterating through regular world, check added entity list
                        while let Some((entity, processed)) = query_state
                            .new_entities
                            .get(query_state.new_entity_iter_pos as usize)
                            .cloned()
                        {
                            if processed {
                                query_state.new_entity_iter_pos += 1;
                                continue;
                            }
                            unsafe {
                                match <<Q as Query>::Fetch as Fetch>::get_overridden(
                                    self.scope, entity,
                                ) {
                                    Ok(item) => {
                                        query_state.new_entity_iter_pos += 1;
                                        return Some((entity, item));
                                    }
                                    Err(_) => {
                                        let iter_pos = query_state.new_entity_iter_pos;
                                        // archetype mismatch, remove entity from list.
                                        // this is in case the archetype changes again, we will re-check it
                                        query_state.new_entities.swap_remove(iter_pos as usize);
                                    }
                                }
                            }
                        }
                        return None;
                    }
                }
                Some((entity, components)) => {
                    let mut query_state = self.query_state.borrow_mut();
                    query_state.archetype_iter_pos = self.iter.position as u32;
                    return Some((entity, components));
                }
            }
        }
    }
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
    unsafe fn next<'a>(
        &mut self,
        scope: &ErgoScope,
        query_state: &Rc<RefCell<ActiveQueryState>>,
    ) -> Option<(Entity, <Q::Fetch as Fetch<'a>>::Item)> {
        loop {
            if self.position == self.len {
                return None;
            }
            let entity = self.entities.as_ptr().add(self.position);
            let generation = scope
                .world
                .entities()
                .meta
                .get_unchecked(*entity as usize)
                .generation;
            let entity = Entity {
                generation,
                id: *entity,
            };
            let item = if !scope.access.is_entity_overridden(entity) {
                Some(self.fetch.get_in_world(scope, self.position, entity))
            } else {
                match <Q::Fetch as Fetch<'a>>::get_overridden(scope, entity) {
                    Ok(item) => {
                        let mut state = query_state.borrow_mut();
                        let mut found = false;
                        for (new_entity, processed) in &mut state.new_entities {
                            if entity == *new_entity {
                                *processed = true;
                                found = true;
                                break;
                            }
                        }
                        if !found {
                            state.new_entities.push((entity, true));
                        }
                        Some(item)
                    }
                    Err(_) => None,
                }
            };
            self.position += 1;
            match item {
                Some(item) => return Some((entity, item)),
                None => continue,
            }
        }
        // TODO: return newly matching results,
        // like newly spawned entities, or newly matching entities
        // due to inserts or removes,
        // given the entity has not been yielded by this iterator before.

        // this can be implemented by maintaining a per-active-query list
        // of newly-matching entities, which will be processed after
        // the world's entities have been processed.
        // mutations to entities will update these lists.
    }
}

macro_rules! tuple_impl {
    ($($name: ident),*) => {
        unsafe impl<'a, $($name: Fetch<'a>),*> Fetch<'a> for ($($name,)*) {
            type Item = ($($name::Item,)*);

            type State = ($($name::State,)*);

            #[allow(clippy::unused_unit)]
            fn dangling() -> Self {
                ($($name::dangling(),)*)
            }

            #[allow(unused_variables)]
            fn prepare(archetype: &Archetype) -> Option<Self::State> {
                Some(($($name::prepare(archetype)?,)*))
            }
            #[allow(unused_variables, non_snake_case, clippy::unused_unit)]
            fn execute(archetype: &'a Archetype, state: Self::State) -> Self {
                let ($($name,)*) = state;
                ($(<$name as Fetch<'a>>::execute(archetype, $name),)*)
            }

            #[allow(unused_variables, clippy::unused_unit)]
            unsafe fn get_in_world(&self, scope: &ErgoScope, n: usize, entity: Entity) -> Self::Item {
                #[allow(non_snake_case)]
                let ($($name,)*) = self;
                ($($name.get_in_world(scope, n, entity),)*)
            }

            #[allow(unused_variables, clippy::unused_unit)]
            unsafe fn get_overridden(scope: &ErgoScope, entity: Entity) -> Result<Self::Item, FetchError> {
                #[allow(non_snake_case)]
                Ok(($(<$name as Fetch<'a>>::get_overridden(scope, entity)?,)*))
            }
        }

        impl<$($name: Query),*> Query for ($($name,)*) {
            type Fetch = ($($name::Fetch,)*);
        }
    };
}

//smaller_tuples_too!(tuple_impl, B, A);
smaller_tuples_too!(tuple_impl, O, N, M, L, K, J, I, H, G, F);

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
    use alloc::vec::Vec;

    use crate::{ErgoScope, World};

    #[test]
    fn ergo_query_iter() {
        let mut world = World::new();
        let e1 = world.spawn((5i32, 1.5f32));
        let e2 = world.spawn((6i32, 2.5f32));
        assert!(world.len() == 2);
        let ergo_scope = ErgoScope::new(&mut world);
        let entities = ergo_scope
            .query::<(&i32, &f32)>()
            .iter()
            .map(|(e, (i, b))| (e, i, b)) // Copy out of the world
            .collect::<Vec<_>>();
        assert!(entities.len() == 2);

        assert_eq!(entities[0].0, e1);
        assert_eq!(*entities[0].1.read(), 5i32);
        assert_eq!(*entities[0].2.read(), 1.5f32);

        assert_eq!(entities[1].0, e2);
        assert_eq!(*entities[1].1.read(), 6i32);
        assert_eq!(*entities[1].2.read(), 2.5f32);
    }

    #[test]
    fn ergo_query_iter_remove() {
        let mut world = World::new();
        let e1 = world.spawn((5i32, 1.5f32));
        let e2 = world.spawn((6i32, 2.5f32));
        assert!(world.len() == 2);
        let ergo_scope = ErgoScope::new(&mut world);
        let mut query = ergo_scope.query::<(&i32, &f32)>();
        let mut entities = query.iter();

        ergo_scope.despawn(e1).expect("failed to despawn entity");

        let (entity, (c2, d2)) = entities.next().expect("expected entity");
        assert_eq!(entity, e2);
        assert_eq!(*c2.read(), 6i32);
        assert_eq!(*d2.read(), 2.5f32);

        assert!(entities.next().is_none());
    }

    #[test]
    fn ergo_query_iter_insert() {
        let mut world = World::new();
        let e1 = world.spawn((5i32, 1.5f32));
        let e2 = world.spawn((6i32,));
        assert!(world.len() == 2);
        let ergo_scope = ErgoScope::new(&mut world);
        let mut query = ergo_scope.query::<(&i32, &f32)>();
        let mut entities = query.iter();

        let (entity, (c1, d1)) = entities.next().expect("expected entity 1");
        assert_eq!(entity, e1);
        assert_eq!(*c1.read(), 5i32);
        assert_eq!(*d1.read(), 1.5f32);

        ergo_scope
            .insert(e2, (2.5f32,))
            .expect("failed to insert component");

        let (entity, (c2, d2)) = entities.next().expect("expected entity 2");
        assert_eq!(entity, e2);
        assert_eq!(*c2.read(), 6i32);
        assert_eq!(*d2.read(), 2.5f32);

        ergo_scope
            .insert(e2, (4.5f32,))
            .expect("failed to insert component");

        assert!(entities.next().is_none());
    }

    #[test]
    fn ergo_query_iter_insert_remove() {
        let mut world = World::new();
        let e1 = world.spawn((5i32, 1.5f32));
        let e2 = world.spawn((6i32,));
        assert!(world.len() == 2);
        let ergo_scope = ErgoScope::new(&mut world);
        let mut query = ergo_scope.query::<(&i32, &f32)>();
        let mut entities = query.iter();

        let (entity, _) = entities.next().expect("expected entity 1");
        assert_eq!(entity, e1);

        ergo_scope
            .insert(e2, (2.5f32,))
            .expect("failed to insert component");

        ergo_scope
            .remove::<(f32,)>(e2)
            .expect("failed to remove component");

        assert!(entities.next().is_none());
    }

    #[test]
    fn ergo_query_iter_remove_insert() {
        let mut world = World::new();
        let e1 = world.spawn((5i32, 1.5f32));
        let e2 = world.spawn((6i32, 1.8f32));
        assert!(world.len() == 2);
        let ergo_scope = ErgoScope::new(&mut world);
        let mut query = ergo_scope.query::<(&i32, &f32)>();
        let mut entities = query.iter();

        let (entity, _) = entities.next().expect("expected entity 1");
        assert_eq!(entity, e1);

        let (entity, _) = entities.next().expect("expected entity 2");
        assert_eq!(entity, e2);

        ergo_scope
            .remove::<(f32,)>(e2)
            .expect("failed to remove component");

        ergo_scope
            .insert(e2, (2.5f32,))
            .expect("failed to insert component");

        assert!(entities.next().is_none());
    }

    #[test]
    fn ergo_query_iter_despawn() {
        let mut world = World::new();
        let e1 = world.spawn((5i32, 1.5f32));
        let e2 = world.spawn((6i32, 1.8f32));
        assert!(world.len() == 2);
        let ergo_scope = ErgoScope::new(&mut world);
        let mut query = ergo_scope.query::<(&i32, &f32)>();
        let mut entities = query.iter();

        let (entity, _) = entities.next().expect("expected entity 1");
        assert_eq!(entity, e1);

        ergo_scope.despawn(e2).expect("failed to despawn");

        assert!(entities.next().is_none());
    }
}
