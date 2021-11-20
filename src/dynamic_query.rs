use core::{
    any::{Any, TypeId},
    ptr::NonNull,
};
use core::{marker::PhantomData, ops::Deref};
use core::{ops::DerefMut, slice::Iter as SliceIter};

use alloc::{boxed::Box, sync::Arc, vec::Vec};
use hashbrown::HashMap;

use crate::{
    borrow::AtomicBorrow, entities::EntityMeta, Access, Archetype, Component, Entity, Fetch, Query,
};

#[derive(Default)]
pub struct DynamicFetch {
    boxed: Option<Box<dyn Any>>,
}

impl DynamicFetch {
    fn new() -> Self {
        Self { boxed: None }
    }

    fn set<T: Any>(&mut self, value: T) -> &mut T {
        match &mut self.boxed {
            Some(boxed) => {
                let ref_mut = boxed.downcast_mut().expect("dynamic fetch invalidated!");
                *ref_mut = value;
                ref_mut
            }
            empty @ None => empty
                .insert(Box::new(value))
                .downcast_mut()
                .expect("dynamic fetch invalidated!"),
        }
    }

    fn get_or_insert_default<T: Any + Default>(&mut self) -> &mut T {
        self.boxed
            .get_or_insert_with(|| Box::new(T::default()))
            .downcast_mut()
            .expect("DynamicFetch type mismatch!")
    }

    unsafe fn borrow_unchecked<T: Any>(&self) -> &T {
        self.boxed
            .as_ref()
            .unwrap_unchecked()
            .downcast_ref()
            .unwrap_unchecked()
    }
}

pub struct DynamicState {
    boxed: Option<Box<dyn Any>>,
    requires_borrow: bool,
}

impl Default for DynamicState {
    fn default() -> Self {
        Self::new()
    }
}

impl DynamicState {
    fn new() -> Self {
        Self {
            boxed: None,
            requires_borrow: false,
        }
    }

    fn set<T: Any>(&mut self, value: Option<T>) {
        match value {
            Some(value) => {
                match &mut self.boxed {
                    Some(boxed) => {
                        let ref_mut = boxed.downcast_mut().expect("dynamic fetch invalidated!");
                        *ref_mut = value;
                    }
                    empty @ None => *empty = Some(Box::new(value)),
                }
                self.requires_borrow = true;
            }
            None => {
                self.boxed = None;
                self.requires_borrow = false;
            }
        }
    }

    fn get_or_insert_default<T: Any + Default>(&mut self) -> &mut T {
        if matches!(self.boxed.as_ref(), Some(t) if t.is::<T>()) {
            self.boxed.as_mut().unwrap().downcast_mut().unwrap()
        } else {
            self.boxed
                .insert(Box::new(T::default()))
                .downcast_mut()
                .unwrap()
        }
    }

    fn get<T: Any + Copy>(&self) -> T {
        let inner = match self.boxed.as_ref() {
            Some(t) => t,
            None => panic!(
                "uninitialized DynamicState! (expected type {}; requires_borrow {})",
                std::any::type_name::<T>(),
                self.requires_borrow
            ),
        };

        inner
            .downcast_ref()
            .copied()
            .expect("DynamicState is of the wrong type!")
    }

    fn borrow<T: Any>(&self) -> &T {
        self.boxed
            .as_ref()
            .expect("uninitialized DynamicState!")
            .downcast_ref()
            .expect("DynamicState type mismatch!")
    }
}

enum DynamicItemAccess {
    Read(*const ()),
    Write(*mut ()),
}

impl DynamicItemAccess {
    unsafe fn as_ref<T>(&self) -> &T {
        match *self {
            DynamicItemAccess::Read(ptr) => &*(ptr as *const T),
            DynamicItemAccess::Write(ptr) => &*(ptr as *const T),
        }
    }

    unsafe fn as_mut<T>(&self) -> Option<&mut T> {
        match *self {
            DynamicItemAccess::Write(ptr) => Some(&mut *(ptr as *mut T)),
            _ => None,
        }
    }
}

/// Immutable borrow of a [`DynamicComponent`].
pub struct Ref<'a, T> {
    read: &'a T,
    // Borrow used as a reference count on the entire dynamic query.
    borrow: &'a AtomicBorrow,
}

impl<'a, T> Ref<'a, T> {
    pub(crate) fn new(read: &'a T, borrow: &'a AtomicBorrow) -> Self {
        // The borrow will be permanently mutably borrowed on drop.
        assert!(
            borrow.borrow(),
            "component of `DynamicItem` is already dropped!"
        );
        Self { read, borrow }
    }
}

impl<'a, T> Drop for Ref<'a, T> {
    fn drop(&mut self) {
        self.borrow.release();
    }
}

impl<'a, T> Deref for Ref<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.read
    }
}

/// Mutable borrow of a [`DynamicComponent`].
pub struct RefMut<'a, T> {
    write: &'a mut T,
    // Borrow used as a reference count on the entire dynamic query; hence this struct actually
    // still has an *immutable* borrow taken from it, not a mutable one, even though it's a mutable
    // borrow (since access to the `DynamicComponent` itself is guaranteed unique.)
    borrow: &'a AtomicBorrow,
}

impl<'a, T> RefMut<'a, T> {
    pub(crate) fn new(write: &'a mut T, borrow: &'a AtomicBorrow) -> Self {
        // The borrow will be permanently mutably borrowed on drop.
        assert!(
            borrow.borrow(),
            "component of `DynamicItem` is already dropped!"
        );
        Self { write, borrow }
    }
}

impl<'a, T> Drop for RefMut<'a, T> {
    fn drop(&mut self) {
        self.borrow.release();
    }
}

impl<'a, T> Deref for RefMut<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.write
    }
}

impl<'a, T> DerefMut for RefMut<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.write
    }
}

/// A handle to a component of type `T` returned from a dynamic query, obtained by
/// [`DynamicItem::take`].
pub struct DynamicComponent<T> {
    access: DynamicItemAccess,
    borrow: Arc<AtomicBorrow>,
    _phantom: PhantomData<T>,
}

unsafe impl<T: Send + Sync> Send for DynamicComponent<T> {}
unsafe impl<T: Send + Sync> Sync for DynamicComponent<T> {}

impl<T> DynamicComponent<T> {
    /// Attempt to immutably borrow this component. Panicks if the dynamic query is already dropped.
    pub fn borrow(&self) -> Ref<T> {
        Ref::new(unsafe { self.access.as_ref() }, &self.borrow)
    }

    /// Attempt to mutably borrow this component. Panicks if the dynamic query is already dropped,
    /// or if the query only borrowed this component immutably.
    pub fn borrow_mut(&mut self) -> RefMut<T> {
        RefMut::new(unsafe { self.access.as_mut() }.unwrap(), &self.borrow)
    }

    /// Attempt to mutably borrow this component, returning `None` if the query only borrowed this
    /// component immutably. Panicks if the dynamic query is already dropped.
    pub fn try_borrow_mut(&mut self) -> Option<RefMut<T>> {
        let borrow = &self.borrow;
        unsafe { self.access.as_mut() }.map(|write| RefMut::new(write, borrow))
    }
}

impl<T: Clone> DynamicComponent<T> {
    /// Clone the value out of the `DynamicComponent`. Panicks if the dynamic query is already
    /// dropped.
    pub fn get(&self) -> T {
        assert!(self.borrow.borrow(), "query dropped!");
        let t = unsafe { self.access.as_ref::<T>() }.clone();
        self.borrow.release();
        t
    }
}

/// A set of components returned from some dynamic query, which belong to some entity satisfying
/// that query.
pub struct DynamicItem {
    components: HashMap<TypeId, DynamicItemAccess>,
    borrow: Arc<AtomicBorrow>,
}

unsafe impl Send for DynamicItem {}
unsafe impl Sync for DynamicItem {}

impl DynamicItem {
    fn new(borrow: Arc<AtomicBorrow>) -> Self {
        Self {
            components: HashMap::new(),
            borrow,
        }
    }

    fn insert_ref<T: Component>(&mut self, t: &T) {
        self.components.insert(
            TypeId::of::<T>(),
            DynamicItemAccess::Read(t as *const _ as *const ()),
        );
    }

    fn insert_mut<T: Component>(&mut self, t: &mut T) {
        self.components.insert(
            TypeId::of::<T>(),
            DynamicItemAccess::Write(t as *mut _ as *mut ()),
        );
    }

    /// Take ownership of this item's borrow of a dynamically queried component.
    pub fn take<T: Component>(&mut self) -> Option<DynamicComponent<T>> {
        self.components
            .remove(&TypeId::of::<T>())
            .map(|access| DynamicComponent {
                access,
                borrow: self.borrow.clone(),
                _phantom: PhantomData,
            })
    }
}

pub trait ErasedFetch<'a>: Send + Sync {
    fn dangling(&self, fetch: &mut DynamicFetch);

    fn access(&self, archetype: &Archetype) -> Option<Access>;
    fn borrow(&self, archetype: &Archetype, state: &DynamicState);
    fn prepare(&self, archetype: &Archetype, state: &mut DynamicState);
    fn execute(&self, archetype: &'a Archetype, state: &DynamicState, fetch: &mut DynamicFetch);
    fn release(&self, archetype: &Archetype, state: &DynamicState);

    fn for_each_borrow(&self, f: &mut dyn FnMut(TypeId, bool));

    unsafe fn get(&self, fetch: &DynamicFetch, n: usize, item: &mut DynamicItem);
}

pub trait ErasedItem<'a> {
    fn into_dynamic_item(self, item: &mut DynamicItem);
}

pub struct StaticFetch<Q: Query + ?Sized>(PhantomData<Q>);

impl<'a, Q: Query + Send + Sync> ErasedFetch<'a> for StaticFetch<Q>
where
    Q::Fetch: 'static,
    <Q::Fetch as Fetch<'a>>::State: Any,
    <Q::Fetch as Fetch<'a>>::Item: ErasedItem<'a>,
{
    fn dangling(&self, fetch: &mut DynamicFetch) {
        fetch.set(Q::Fetch::dangling());
    }

    fn access(&self, archetype: &Archetype) -> Option<Access> {
        Q::Fetch::access(archetype)
    }

    fn borrow(&self, archetype: &Archetype, state: &DynamicState) {
        Q::Fetch::borrow(archetype, state.get())
    }

    fn prepare(&self, archetype: &Archetype, state: &mut DynamicState) {
        state.set(Q::Fetch::prepare(archetype));
    }

    fn execute(&self, archetype: &'a Archetype, state: &DynamicState, fetch: &mut DynamicFetch) {
        fetch.set(Q::Fetch::execute(archetype, state.get()));
    }

    fn release(&self, archetype: &Archetype, state: &DynamicState) {
        Q::Fetch::release(archetype, state.get());
    }

    fn for_each_borrow(&self, f: &mut dyn FnMut(TypeId, bool)) {
        Q::Fetch::for_each_borrow(f)
    }

    unsafe fn get(&self, fetch: &DynamicFetch, n: usize, item: &mut DynamicItem) {
        Q::Fetch::get(fetch.borrow_unchecked(), n).into_dynamic_item(item);
    }
}

impl<'a, T: Component> ErasedItem<'a> for &'a T {
    fn into_dynamic_item(self, item: &mut DynamicItem) {
        item.insert_ref(self);
    }
}

impl<'a, T: Component> ErasedItem<'a> for &'a mut T {
    fn into_dynamic_item(self, item: &mut DynamicItem) {
        item.insert_mut(self);
    }
}

impl<'a, T: ErasedItem<'a>> ErasedItem<'a> for Option<T> {
    fn into_dynamic_item(self, item: &mut DynamicItem) {
        match self {
            None => {}
            Some(value) => value.into_dynamic_item(item),
        }
    }
}

impl<'a, T: ErasedItem<'a>, U: ErasedItem<'a>> ErasedItem<'a> for (T, U) {
    fn into_dynamic_item(self, item: &mut DynamicItem) {
        self.0.into_dynamic_item(item);
        self.1.into_dynamic_item(item);
    }
}

pub trait ErasedQuery: for<'a> ErasedFetch<'a> + Any {}
impl<T: for<'a> ErasedFetch<'a> + Any> ErasedQuery for T {}

/// A dynamic query; the dynamic equivalent of the [`Query`] trait. Used with
/// [`World::dynamic_query`] and [`World::dynamic_query_one`].
///
/// [`World::dynamic_query`]: crate::World::dynamic_query
/// [`World::dynamic_query_one`]: crate::World::dynamic_query_one
#[derive(Clone)]
pub struct DynamicQuery(Arc<dyn for<'a> ErasedFetch<'a>>);

impl DynamicQuery {
    /// Create a new `DynamicQuery`. Use this to compose dynamic queries in a similar way to how you
    /// would use tuples to compose [`Query`]s.
    pub fn new(query: impl ErasedQuery) -> Self {
        Self(Arc::new(query))
    }

    /// Lift a static [`Query`] into a `DynamicQuery`.
    pub fn lift<Q: Query + Send + Sync>() -> Self
    where
        Q: 'static,
        for<'a> <Q::Fetch as Fetch<'a>>::State: Any,
        for<'a> <Q::Fetch as Fetch<'a>>::Item: ErasedItem<'a>,
    {
        Self(Arc::new(StaticFetch::<Q>(PhantomData)))
    }
}

impl<'a> ErasedFetch<'a> for DynamicQuery {
    fn dangling(&self, fetch: &mut DynamicFetch) {
        self.0.dangling(fetch)
    }

    fn access(&self, archetype: &Archetype) -> Option<Access> {
        self.0.access(archetype)
    }

    fn borrow(&self, archetype: &Archetype, state: &DynamicState) {
        self.0.borrow(archetype, state)
    }

    fn prepare(&self, archetype: &Archetype, state: &mut DynamicState) {
        self.0.prepare(archetype, state)
    }

    fn execute(&self, archetype: &'a Archetype, state: &DynamicState, fetch: &mut DynamicFetch) {
        self.0.execute(archetype, state, fetch)
    }

    fn release(&self, archetype: &Archetype, state: &DynamicState) {
        self.0.release(archetype, state)
    }

    fn for_each_borrow(&self, f: &mut dyn FnMut(TypeId, bool)) {
        self.0.for_each_borrow(f)
    }

    unsafe fn get(&self, fetch: &DynamicFetch, n: usize, item: &mut DynamicItem) {
        self.0.get(fetch, n, item)
    }
}

impl<'a, T: ErasedFetch<'a> + ?Sized> ErasedFetch<'a> for Box<T> {
    fn dangling(&self, fetch: &mut DynamicFetch) {
        (**self).dangling(fetch)
    }

    fn access(&self, archetype: &Archetype) -> Option<Access> {
        (**self).access(archetype)
    }

    fn borrow(&self, archetype: &Archetype, state: &DynamicState) {
        (**self).borrow(archetype, state)
    }

    fn prepare(&self, archetype: &Archetype, state: &mut DynamicState) {
        (**self).prepare(archetype, state)
    }

    fn execute(&self, archetype: &'a Archetype, state: &DynamicState, fetch: &mut DynamicFetch) {
        (**self).execute(archetype, state, fetch)
    }

    fn release(&self, archetype: &Archetype, state: &DynamicState) {
        (**self).release(archetype, state)
    }

    fn for_each_borrow(&self, f: &mut dyn FnMut(TypeId, bool)) {
        (**self).for_each_borrow(f)
    }

    unsafe fn get(&self, fetch: &DynamicFetch, n: usize, item: &mut DynamicItem) {
        (**self).get(fetch, n, item)
    }
}

impl<'a, T: ErasedFetch<'a>> ErasedFetch<'a> for [T] {
    fn dangling(&self, fetch: &mut DynamicFetch) {
        let v = fetch.get_or_insert_default::<Vec<DynamicFetch>>();
        if v.len() < self.len() {
            v.resize_with(self.len(), DynamicFetch::new);
        }
        self.iter()
            .zip(v.iter_mut())
            .for_each(|(erased_fetch, dynamic_fetch)| erased_fetch.dangling(dynamic_fetch));
    }

    fn access(&self, archetype: &Archetype) -> Option<Access> {
        self.iter().filter_map(|f| f.access(archetype)).max()
    }

    fn borrow(&self, archetype: &Archetype, state: &DynamicState) {
        self.iter()
            .zip(state.borrow::<Vec<DynamicState>>())
            .for_each(|(erased_fetch, state)| erased_fetch.borrow(archetype, state));
    }

    fn prepare(&self, archetype: &Archetype, state: &mut DynamicState) {
        let v = state.get_or_insert_default::<Vec<DynamicState>>();
        if v.len() < self.len() {
            v.resize_with(self.len(), DynamicState::new);
        }

        state.requires_borrow = self
            .iter()
            .zip(v.iter_mut())
            .map(|(erased_fetch, state)| {
                erased_fetch.prepare(archetype, state);
                state.requires_borrow
            })
            .fold(false, |a, b| a | b);
    }

    fn execute(&self, archetype: &'a Archetype, state: &DynamicState, fetch: &mut DynamicFetch) {
        let states = state.borrow::<Vec<DynamicState>>();
        let fetches = fetch.get_or_insert_default::<Vec<DynamicFetch>>();
        if fetches.len() < self.len() {
            fetches.resize_with(self.len(), DynamicFetch::new);
        }
        self.iter()
            .zip(states.iter().zip(fetches.iter_mut()))
            .for_each(|(erased_fetch, (state, fetch))| {
                erased_fetch.execute(archetype, state, fetch)
            });
    }

    fn release(&self, archetype: &Archetype, state: &DynamicState) {
        self.iter()
            .zip(state.borrow::<Vec<DynamicState>>())
            .for_each(|(erased_fetch, state)| erased_fetch.release(archetype, state));
    }

    fn for_each_borrow(&self, f: &mut dyn FnMut(TypeId, bool)) {
        self.iter()
            .for_each(|erased_fetch| erased_fetch.for_each_borrow(f))
    }

    unsafe fn get(&self, fetch: &DynamicFetch, n: usize, item: &mut DynamicItem) {
        let fetches = fetch.borrow_unchecked::<Vec<DynamicFetch>>();
        self.iter()
            .zip(fetches.iter())
            .for_each(|(erased_fetch, fetch)| erased_fetch.get(fetch, n, item));
    }
}

impl<'a, T: ErasedFetch<'a>> ErasedFetch<'a> for Vec<T> {
    fn dangling(&self, fetch: &mut DynamicFetch) {
        self.as_slice().dangling(fetch)
    }

    fn access(&self, archetype: &Archetype) -> Option<Access> {
        self.as_slice().access(archetype)
    }

    fn borrow(&self, archetype: &Archetype, state: &DynamicState) {
        self.as_slice().borrow(archetype, state)
    }

    fn prepare(&self, archetype: &Archetype, state: &mut DynamicState) {
        self.as_slice().prepare(archetype, state)
    }

    fn execute(&self, archetype: &'a Archetype, state: &DynamicState, fetch: &mut DynamicFetch) {
        self.as_slice().execute(archetype, state, fetch)
    }

    fn release(&self, archetype: &Archetype, state: &DynamicState) {
        self.as_slice().release(archetype, state)
    }

    fn for_each_borrow(&self, f: &mut dyn FnMut(TypeId, bool)) {
        self.as_slice().for_each_borrow(f)
    }

    unsafe fn get(&self, fetch: &DynamicFetch, n: usize, item: &mut DynamicItem) {
        <[T] as ErasedFetch<'a>>::get(self.as_slice(), fetch, n, item)
    }
}

/// A dynamic query transformer which requires that entities satisfying some dynamic query also have
/// some component type, without needing to borrow it.
pub struct DynamicWith<Q> {
    subquery: Q,
    type_id: TypeId,
}

impl<'a, Q: ErasedFetch<'a>> ErasedFetch<'a> for DynamicWith<Q> {
    fn dangling(&self, fetch: &mut DynamicFetch) {
        self.subquery.dangling(fetch)
    }

    fn access(&self, archetype: &Archetype) -> Option<Access> {
        if archetype.has_dynamic(self.type_id) {
            self.subquery.access(archetype)
        } else {
            None
        }
    }

    fn borrow(&self, archetype: &Archetype, state: &DynamicState) {
        self.subquery.borrow(archetype, state)
    }

    fn prepare(&self, archetype: &Archetype, state: &mut DynamicState) {
        if archetype.has_dynamic(self.type_id) {
            self.subquery.prepare(archetype, state);
        }
    }

    fn execute(&self, archetype: &'a Archetype, state: &DynamicState, fetch: &mut DynamicFetch) {
        self.subquery.execute(archetype, state, fetch)
    }

    fn release(&self, archetype: &Archetype, state: &DynamicState) {
        self.subquery.release(archetype, state)
    }

    fn for_each_borrow(&self, f: &mut dyn FnMut(TypeId, bool)) {
        self.subquery.for_each_borrow(f)
    }

    unsafe fn get(&self, fetch: &DynamicFetch, n: usize, item: &mut DynamicItem) {
        self.subquery.get(fetch, n, item)
    }
}

/// A dynamic query transformer which requires that entities satisfying the subquery also *not*
/// contain some component type.
pub struct DynamicWithout<Q> {
    subquery: Q,
    type_id: TypeId,
}

impl<'a, Q: ErasedFetch<'a>> ErasedFetch<'a> for DynamicWithout<Q> {
    fn dangling(&self, fetch: &mut DynamicFetch) {
        self.subquery.dangling(fetch)
    }

    fn access(&self, archetype: &Archetype) -> Option<Access> {
        if archetype.has_dynamic(self.type_id) {
            None
        } else {
            self.subquery.access(archetype)
        }
    }

    fn borrow(&self, archetype: &Archetype, state: &DynamicState) {
        self.subquery.borrow(archetype, state)
    }

    fn prepare(&self, archetype: &Archetype, state: &mut DynamicState) {
        if !archetype.has_dynamic(self.type_id) {
            self.subquery.prepare(archetype, state);
        }
    }

    fn execute(&self, archetype: &'a Archetype, state: &DynamicState, fetch: &mut DynamicFetch) {
        self.subquery.execute(archetype, state, fetch)
    }

    fn release(&self, archetype: &Archetype, state: &DynamicState) {
        self.subquery.release(archetype, state)
    }

    fn for_each_borrow(&self, f: &mut dyn FnMut(TypeId, bool)) {
        self.subquery.for_each_borrow(f)
    }

    unsafe fn get(&self, fetch: &DynamicFetch, n: usize, item: &mut DynamicItem) {
        self.subquery.get(fetch, n, item)
    }
}

/// A borrow of a [`World`](crate::World) sufficient to execute some dynamic query.
///
/// As with [`QueryBorrow`](crate::QueryBorrow), borrows are not released until this object is
/// dropped. When this object is dropped, all [`DynamicItem`]s and [`DynamicComponent`]s created
/// from it are invalidated (and if any are still borrowed, a panic will occur.)
pub struct DynamicQueryBorrow<'w> {
    meta: &'w [EntityMeta],
    archetypes: &'w [Archetype],
    borrowed: bool,
    borrow: Arc<AtomicBorrow>,
    dynamic_query: &'w DynamicQuery,
    dynamic_state: DynamicState,
    dynamic_fetch: DynamicFetch,
}

impl<'w> DynamicQueryBorrow<'w> {
    pub(crate) fn new(
        meta: &'w [EntityMeta],
        archetypes: &'w [Archetype],
        dynamic_query: &'w DynamicQuery,
    ) -> Self {
        Self {
            meta,
            archetypes,
            borrowed: false,
            borrow: Arc::new(AtomicBorrow::new()),
            dynamic_query,
            dynamic_state: DynamicState::new(),
            dynamic_fetch: DynamicFetch::new(),
        }
    }

    /// Execute the query.
    ///
    /// Must be called only once per query.
    pub fn iter(&mut self) -> DynamicQueryIter<'_> {
        self.borrow();
        unsafe {
            DynamicQueryIter::new(
                self.meta,
                self.archetypes.iter(),
                self.dynamic_query,
                &mut self.dynamic_fetch,
                &mut self.dynamic_state,
                self.borrow.clone(),
            )
        }
    }

    fn borrow(&mut self) {
        if self.borrowed {
            return;
        }

        for archetype in self.archetypes {
            // TODO: Release prior borrows on failure? (see query.rs)
            self.dynamic_query
                .prepare(archetype, &mut self.dynamic_state);
            if self.dynamic_state.requires_borrow {
                self.dynamic_query.borrow(archetype, &self.dynamic_state);
            }
        }
    }
}

impl<'w> Drop for DynamicQueryBorrow<'w> {
    fn drop(&mut self) {
        if self.borrowed {
            // Once dropped, we mutably borrow the `Arc<AtomicBorrow>` in order to ensure that any
            // `DynamicComponents` from this query can no longer be dereferenced. In addition, we
            // must panic if any components are still actively borrowed at this time.
            assert!(self.borrow.borrow_mut(), "attempted to drop dynamic query borrow while components are still actively borrowed from it");

            for archetype in self.archetypes {
                // TODO: Release prior borrows on failure? (see query.rs)
                self.dynamic_query
                    .prepare(archetype, &mut self.dynamic_state);
                if self.dynamic_state.requires_borrow {
                    self.dynamic_query.release(archetype, &self.dynamic_state);
                }
            }
        }
    }
}

/// Iterator over the set of entities which satisfy some [`DynamicQuery`].
pub struct DynamicQueryIter<'q> {
    meta: &'q [EntityMeta],
    archetypes: SliceIter<'q, Archetype>,
    iter: DynamicChunkIter,
    dynamic_query: &'q DynamicQuery,
    dynamic_fetch: &'q mut DynamicFetch,
    dynamic_state: &'q mut DynamicState,
    borrow: Arc<AtomicBorrow>,
}

impl<'q> DynamicQueryIter<'q> {
    /// # Safety
    ///
    /// `'q` must be sufficient to guarantee that `Q` cannot violate borrow safety, either with
    /// dynamic borrow checks or by representing exclusive access to the `World`.
    unsafe fn new(
        meta: &'q [EntityMeta],
        archetypes: SliceIter<'q, Archetype>,
        dynamic_query: &'q DynamicQuery,
        dynamic_fetch: &'q mut DynamicFetch,
        dynamic_state: &'q mut DynamicState,
        borrow: Arc<AtomicBorrow>,
    ) -> Self {
        Self {
            meta,
            archetypes,
            iter: DynamicChunkIter::empty(dynamic_query, dynamic_fetch),
            dynamic_query,
            dynamic_fetch,
            dynamic_state,
            borrow,
        }
    }
}

impl<'q> Iterator for DynamicQueryIter<'q> {
    type Item = (Entity, DynamicItem);

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        let mut dynamic_item = DynamicItem::new(self.borrow.clone());
        loop {
            match unsafe {
                self.iter
                    .next(self.dynamic_query, self.dynamic_fetch, &mut dynamic_item)
            } {
                None => {
                    let archetype = self.archetypes.next()?;
                    self.dynamic_query.prepare(archetype, self.dynamic_state);
                    self.iter = if self.dynamic_state.requires_borrow {
                        self.dynamic_query.execute(
                            archetype,
                            self.dynamic_state,
                            self.dynamic_fetch,
                        );

                        DynamicChunkIter {
                            entities: archetype.entities(),
                            position: 0,
                            len: archetype.len() as usize,
                        }
                    } else {
                        DynamicChunkIter::empty(self.dynamic_query, self.dynamic_fetch)
                    };
                    continue;
                }
                Some(id) => {
                    return Some((
                        Entity {
                            id,
                            generation: unsafe { self.meta.get_unchecked(id as usize).generation },
                        },
                        dynamic_item,
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

impl<'q> ExactSizeIterator for DynamicQueryIter<'q> {
    fn len(&self) -> usize {
        self.archetypes
            .clone()
            .filter(|&x| self.dynamic_query.access(x).is_some())
            .map(|x| x.len() as usize)
            .sum::<usize>()
            + self.iter.remaining()
    }
}

struct DynamicChunkIter {
    entities: NonNull<u32>,
    position: usize,
    len: usize,
}

impl DynamicChunkIter {
    fn empty(dynamic_query: &DynamicQuery, dynamic_fetch: &mut DynamicFetch) -> Self {
        dynamic_query.dangling(dynamic_fetch);

        Self {
            entities: NonNull::dangling(),
            position: 0,
            len: 0,
        }
    }

    #[inline]
    unsafe fn next(
        &mut self,
        dynamic_query: &DynamicQuery,
        dynamic_fetch: &mut DynamicFetch,
        dynamic_item: &mut DynamicItem,
    ) -> Option<u32> {
        if self.position == self.len {
            return None;
        }
        let entity = self.entities.as_ptr().add(self.position);
        dynamic_query.get(dynamic_fetch, self.position, dynamic_item);
        self.position += 1;
        Some(*entity)
    }

    fn remaining(&self) -> usize {
        self.len - self.position
    }
}

pub struct DynamicQueryOne<'w> {
    archetype: &'w Archetype,
    index: u32,
    borrowed: bool,
    borrow: Arc<AtomicBorrow>,
    dynamic_query: &'w DynamicQuery,
    dynamic_state: DynamicState,
    dynamic_fetch: DynamicFetch,
}

impl<'w> DynamicQueryOne<'w> {
    pub(crate) unsafe fn new(
        archetype: &'w Archetype,
        index: u32,
        dynamic_query: &'w DynamicQuery,
    ) -> Self {
        Self {
            archetype,
            index,
            borrowed: false,
            borrow: Arc::new(AtomicBorrow::new()),
            dynamic_query,
            dynamic_state: DynamicState::new(),
            dynamic_fetch: DynamicFetch::new(),
        }
    }

    pub fn get(&mut self) -> Option<DynamicItem> {
        if self.borrowed {
            panic!("called DynamicQueryOne::get twice; construct a new query instead");
        }
        unsafe {
            self.dynamic_query
                .prepare(self.archetype, &mut self.dynamic_state);
            if !self.dynamic_state.requires_borrow {
                return None;
            }
            self.dynamic_query
                .borrow(self.archetype, &self.dynamic_state);
            self.dynamic_query.execute(
                self.archetype,
                &self.dynamic_state,
                &mut self.dynamic_fetch,
            );
            self.borrowed = true;
            let mut dynamic_item = DynamicItem::new(self.borrow.clone());
            self.dynamic_query
                .get(&self.dynamic_fetch, self.index as usize, &mut dynamic_item);
            Some(dynamic_item)
        }
    }
}

unsafe impl Send for DynamicQueryOne<'_> {}
unsafe impl Sync for DynamicQueryOne<'_> {}

impl Drop for DynamicQueryOne<'_> {
    fn drop(&mut self) {
        if self.borrowed {
            assert!(self.borrow.borrow_mut(), "attempted to drop dynamic query borrow while components are still actively borrowed from it");

            // No need to prepare here - we're guaranteed a prior prepare is still valid.
            self.dynamic_query
                .release(self.archetype, &self.dynamic_state);
        }
    }
}

macro_rules! tuple_repeat {
    (
        $subst:tt,
        ($orig_head:tt $(, $orig_tail:tt)* $(,)?)
        $($result:tt)*
    ) => { tuple_repeat!($subst, ($($orig_tail),*) $($result)* $subst, ) };
    (
        $subst:tt,
        ()
        $($result:tt)*
    ) => { ($($result)*) };
}

macro_rules! tuple_impl {
    ($(($name:ident, $field:tt)),*) => {
        impl<'a, $($name: ErasedFetch<'a>),*> ErasedFetch<'a> for ($($name,)*) {
            #[allow(unused_variables)]
            fn dangling(&self, fetch: &mut DynamicFetch) {
                let fs = fetch.get_or_insert_default::<tuple_repeat!(DynamicFetch, ($($name,)*))>();
                $(self.$field.dangling(&mut fs.$field);)*
            }

            #[allow(unused_variables, unused_mut)]
            fn access(&self, archetype: &Archetype) -> Option<Access> {
                let mut access = Access::Iterate;
                $(access = access.max(self.$field.access(archetype)?);)*
                Some(access)
            }

            #[allow(unused_variables)]
            fn borrow(&self, archetype: &Archetype, state: &DynamicState) {
                let st = state.borrow::<tuple_repeat!(DynamicState, ($($name,)*))>();
                $(self.$field.borrow(archetype, &st.$field);)*
            }

            #[allow(unused_variables)]
            fn prepare(&self, archetype: &Archetype, state: &mut DynamicState) {
                let st = state.get_or_insert_default::<tuple_repeat!(DynamicState, ($($name,)*))>();
                $(self.$field.prepare(archetype, &mut st.$field);)*
            }

            #[allow(unused_variables)]
            fn execute(&self, archetype: &'a Archetype, state: &DynamicState, fetch: &mut DynamicFetch) {
                let st = state.borrow::<tuple_repeat!(DynamicState, ($($name,)*))>();
                let fs = fetch.get_or_insert_default::<tuple_repeat!(DynamicFetch, ($($name,)*))>();
                $(self.$field.execute(archetype, &st.$field, &mut fs.$field);)*
            }

            #[allow(unused_variables)]
            fn release(&self, archetype: &Archetype, state: &DynamicState) {
                let st = state.borrow::<tuple_repeat!(DynamicState, ($($name,)*))>();
                $(self.$field.release(archetype, &st.$field);)*
            }

            #[allow(unused_variables)]
            fn for_each_borrow(&self, f: &mut dyn FnMut(TypeId, bool)) {
                $(self.$field.for_each_borrow(f);)*
            }

            #[allow(unused_variables)]
            unsafe fn get(&self, fetch: &DynamicFetch, n: usize, item: &mut DynamicItem) {
                let fs = fetch.borrow_unchecked::<tuple_repeat!(DynamicFetch, ($($name,)*))>();
                $(self.$field.get(&fs.$field, n, item);)*
            }
        }
    };
}

smaller_tuples_too!(
    tuple_impl,
    (L, 11),
    (K, 10),
    (J, 9),
    (I, 8),
    (H, 7),
    (G, 6),
    (F, 5),
    (E, 4),
    (D, 3),
    (C, 2),
    (B, 1),
    (A, 0)
);

impl<'a, T: ErasedFetch<'a>, const N: usize> ErasedFetch<'a> for [T; N]
where
    [DynamicFetch; N]: Default,
    [DynamicState; N]: Default,
{
    fn dangling(&self, fetch: &mut DynamicFetch) {
        let fs = fetch.get_or_insert_default::<[DynamicFetch; N]>();
        for i in 0..N {
            self[i].dangling(&mut fs[i]);
        }
    }

    fn access(&self, archetype: &Archetype) -> Option<Access> {
        let mut access = Access::Iterate;
        for q in self {
            access = access.max(q.access(archetype)?);
        }
        Some(access)
    }

    fn borrow(&self, archetype: &Archetype, state: &DynamicState) {
        let st = state.borrow::<[DynamicState; N]>();
        for i in 0..N {
            self[i].borrow(archetype, &st[i]);
        }
    }

    fn prepare(&self, archetype: &Archetype, state: &mut DynamicState) {
        let st = state.get_or_insert_default::<[DynamicState; N]>();
        for i in 0..N {
            self[i].prepare(archetype, &mut st[i]);
        }
    }

    fn execute(&self, archetype: &'a Archetype, state: &DynamicState, fetch: &mut DynamicFetch) {
        let st = state.borrow::<[DynamicState; N]>();
        let fs = fetch.get_or_insert_default::<[DynamicFetch; N]>();
        for i in 0..N {
            self[i].execute(archetype, &st[i], &mut fs[i]);
        }
    }

    fn release(&self, archetype: &Archetype, state: &DynamicState) {
        let st = state.borrow::<[DynamicState; N]>();
        for i in 0..N {
            self[i].release(archetype, &st[i]);
        }
    }

    fn for_each_borrow(&self, f: &mut dyn FnMut(TypeId, bool)) {
        for q in self {
            q.for_each_borrow(f);
        }
    }

    unsafe fn get(&self, fetch: &DynamicFetch, n: usize, item: &mut DynamicItem) {
        let fs = fetch.borrow_unchecked::<[DynamicFetch; N]>();
        for i in 0..N {
            self[i].get(&fs[i], n, item);
        }
    }
}
