// Copyright 2019 Google LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use core::fmt;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::ptr::NonNull;

use crate::archetype::Archetype;
use crate::entities::EntityMeta;
use crate::{Component, Entity, SmartComponent};

/// A collection of component types to fetch from a `World`
pub trait Query<'c, C: Clone + 'c = ()> {
    #[doc(hidden)]
    type Fetch: for<'q> Fetch<'q, 'c, C>;
}

/// Streaming iterators over contiguous homogeneous ranges of components
pub trait Fetch<'q, 'c, C: Clone + 'c>: Sized {
    /// Type of value to be fetched
    type Item;

    /// How this query will access `archetype`, if at all
    fn access(archetype: &Archetype) -> Option<Access>;

    /// Acquire dynamic borrows from `archetype`
    fn borrow(archetype: &Archetype);
    /// Construct a `Fetch` for `archetype` if it should be traversed
    ///
    /// # Safety
    /// `offset` must be in bounds of `archetype`
    unsafe fn get(archetype: &'q Archetype, offset: usize) -> Option<Self>;
    /// Release dynamic borrows acquired by `borrow`
    fn release(archetype: &Archetype);

    /// Access the next item in this archetype without bounds checking
    ///
    /// # Safety
    /// - Must only be called after `borrow`
    /// - `release` must not be called while `'a` is still live
    /// - Bounds-checking must be performed externally
    /// - Any resulting borrows must be legal (e.g. no &mut to something another iterator might access)
    unsafe fn next(&mut self, id: u32, context: C) -> Self::Item;
}

/// Type of access a `Query` may have to an `Archetype`
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub enum Access {
    /// Read entity IDs only, no components
    Iterate,
    /// Read components
    Read,
    /// Read and write components
    Write,
}

impl<'a, T: SmartComponent<C>, C: Clone + 'a> Query<'a, C> for &'a T {
    type Fetch = FetchRead<T>;
}

#[doc(hidden)]
pub struct FetchRead<T>(NonNull<T>);

pub struct Ref<'a, T, C> {
    value: &'a T,
    id: u32,
    context: C,
}

impl<'a, T, C: Clone> Clone for Ref<'a, T, C> {
    fn clone(&self) -> Self {
        Self {
            value: self.value,
            id: self.id,
            context: self.context.clone(),
        }
    }
}

impl<'a, T, C: Copy> Copy for Ref<'a, T, C> {}

impl<'a, T: fmt::Debug, C> fmt::Debug for Ref<'a, T, C> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.value.fmt(f)
    }
}

impl<'a, T: SmartComponent<C>, C: Clone> Deref for Ref<'a, T, C> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.value.on_borrow(self.id, &self.context);
        self.value
    }
}

impl<'q, 'c, T: SmartComponent<C>, C: Clone + 'c> Fetch<'q, 'c, C> for FetchRead<T> {
    type Item = Ref<'q, T, C>;

    fn access(archetype: &Archetype) -> Option<Access> {
        if archetype.has::<T>() {
            Some(Access::Read)
        } else {
            None
        }
    }

    fn borrow(archetype: &Archetype) {
        archetype.borrow::<T>();
    }

    unsafe fn get(archetype: &'q Archetype, offset: usize) -> Option<Self> {
        archetype
            .get::<T>()
            .map(|x| Self(NonNull::new_unchecked(x.as_ptr().add(offset))))
    }

    fn release(archetype: &Archetype) {
        archetype.release::<T>();
    }

    unsafe fn next(&mut self, id: u32, context: C) -> Ref<'q, T, C> {
        let x = self.0.as_ptr();
        self.0 = NonNull::new_unchecked(x.add(1));
        Ref {
            value: &*x,
            id,
            context,
        }
    }
}

impl<'a, T: SmartComponent<C>, C: Clone + 'a> Query<'a, C> for &'a mut T {
    type Fetch = FetchWrite<T>;
}

#[doc(hidden)]
pub struct FetchWrite<T>(NonNull<T>);

pub struct RefMut<'a, T, C> {
    value: &'a mut T,
    id: u32,
    context: C,
}

impl<'a, T: fmt::Debug, C> fmt::Debug for RefMut<'a, T, C> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.value.fmt(f)
    }
}

impl<'a, T: SmartComponent<C>, C: Clone> Deref for RefMut<'a, T, C> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        (&*self.value).on_borrow(self.id, &self.context);
        self.value
    }
}

impl<'a, T: SmartComponent<C>, C: Clone> DerefMut for RefMut<'a, T, C> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value.on_borrow_mut(self.id, &self.context);
        self.value
    }
}

impl<'q, 'c, T: SmartComponent<C>, C: Clone + 'c> Fetch<'q, 'c, C> for FetchWrite<T> {
    type Item = RefMut<'q, T, C>;

    fn access(archetype: &Archetype) -> Option<Access> {
        if archetype.has::<T>() {
            Some(Access::Write)
        } else {
            None
        }
    }

    fn borrow(archetype: &Archetype) {
        archetype.borrow_mut::<T>();
    }

    unsafe fn get(archetype: &'q Archetype, offset: usize) -> Option<Self> {
        archetype
            .get::<T>()
            .map(|x| Self(NonNull::new_unchecked(x.as_ptr().add(offset))))
    }

    fn release(archetype: &Archetype) {
        archetype.release_mut::<T>();
    }

    unsafe fn next(&mut self, id: u32, context: C) -> RefMut<'q, T, C> {
        let x = self.0.as_ptr();
        self.0 = NonNull::new_unchecked(x.add(1));
        RefMut {
            value: &mut *x,
            id,
            context,
        }
    }
}

impl<'c, T: Query<'c, C>, C: Clone + 'c> Query<'c, C> for Option<T> {
    type Fetch = TryFetch<T::Fetch>;
}

#[doc(hidden)]
pub struct TryFetch<T>(Option<T>);

impl<'q, 'c, T: Fetch<'q, 'c, C>, C: Clone + 'c> Fetch<'q, 'c, C> for TryFetch<T> {
    type Item = Option<T::Item>;

    fn access(archetype: &Archetype) -> Option<Access> {
        Some(T::access(archetype).unwrap_or(Access::Iterate))
    }

    fn borrow(archetype: &Archetype) {
        T::borrow(archetype)
    }

    unsafe fn get(archetype: &'q Archetype, offset: usize) -> Option<Self> {
        Some(Self(T::get(archetype, offset)))
    }

    fn release(archetype: &Archetype) {
        T::release(archetype)
    }

    unsafe fn next(&mut self, id: u32, context: C) -> Option<T::Item> {
        Some(self.0.as_mut()?.next(id, context))
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

impl<'c, T: Component, Q: Query<'c, C>, C: Clone + 'c> Query<'c, C> for Without<T, Q> {
    type Fetch = FetchWithout<T, Q::Fetch>;
}

#[doc(hidden)]
pub struct FetchWithout<T, F>(F, PhantomData<fn(T)>);

impl<'q, 'c, T: Component, F: Fetch<'q, 'c, C>, C: Clone + 'c> Fetch<'q, 'c, C>
    for FetchWithout<T, F>
{
    type Item = F::Item;

    fn access(archetype: &Archetype) -> Option<Access> {
        if archetype.has::<T>() {
            None
        } else {
            F::access(archetype)
        }
    }

    fn borrow(archetype: &Archetype) {
        F::borrow(archetype)
    }
    unsafe fn get(archetype: &'q Archetype, offset: usize) -> Option<Self> {
        if archetype.has::<T>() {
            return None;
        }
        Some(Self(F::get(archetype, offset)?, PhantomData))
    }
    fn release(archetype: &Archetype) {
        F::release(archetype)
    }

    unsafe fn next(&mut self, id: u32, context: C) -> F::Item {
        self.0.next(id, context)
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

impl<'c, T: Component, Q: Query<'c, C>, C: Clone + 'c> Query<'c, C> for With<T, Q> {
    type Fetch = FetchWith<T, Q::Fetch>;
}

#[doc(hidden)]
pub struct FetchWith<T, F>(F, PhantomData<fn(T)>);

impl<'q, 'c, T: Component, F: Fetch<'q, 'c, C>, C: Clone + 'c> Fetch<'q, 'c, C>
    for FetchWith<T, F>
{
    type Item = F::Item;

    fn access(archetype: &Archetype) -> Option<Access> {
        if archetype.has::<T>() {
            F::access(archetype)
        } else {
            None
        }
    }

    fn borrow(archetype: &Archetype) {
        F::borrow(archetype)
    }

    unsafe fn get(archetype: &'q Archetype, offset: usize) -> Option<Self> {
        if !archetype.has::<T>() {
            return None;
        }
        Some(Self(F::get(archetype, offset)?, PhantomData))
    }

    fn release(archetype: &Archetype) {
        F::release(archetype)
    }

    unsafe fn next(&mut self, id: u32, context: C) -> F::Item {
        self.0.next(id, context)
    }
}

/// A borrow of a `World` sufficient to execute the query `Q`
///
/// Note that borrows are not released until this object is dropped.
pub struct QueryBorrow<'w, Q: Query<'w, C>, C: Clone + 'w> {
    meta: &'w [EntityMeta],
    archetypes: &'w [Archetype],
    borrowed: bool,
    context: C,
    _marker: PhantomData<Q>,
}

impl<'w, Q: Query<'w, C>, C: Clone + 'w> QueryBorrow<'w, Q, C> {
    pub(crate) fn new(meta: &'w [EntityMeta], archetypes: &'w [Archetype], context: C) -> Self {
        Self {
            meta,
            archetypes,
            borrowed: false,
            context,
            _marker: PhantomData,
        }
    }

    /// Execute the query
    ///
    /// Must be called only once per query.
    pub fn iter<'q>(&'q mut self) -> QueryIter<'q, 'w, Q, C> {
        self.borrow();
        QueryIter {
            borrow: self,
            archetype_index: 0,
            iter: None,
        }
    }

    /// Like `iter`, but returns child iterators of at most `batch_size` elements
    ///
    /// Useful for distributing work over a threadpool.
    pub fn iter_batched<'q>(&'q mut self, batch_size: u32) -> BatchedIter<'q, 'w, Q, C> {
        self.borrow();
        BatchedIter {
            borrow: self,
            archetype_index: 0,
            batch_size,
            batch: 0,
        }
    }

    fn borrow(&mut self) {
        if self.borrowed {
            panic!(
                "called QueryBorrow::iter twice on the same borrow; construct a new query instead"
            );
        }
        for x in self.archetypes {
            // TODO: Release prior borrows on failure?
            if Q::Fetch::access(x) >= Some(Access::Read) {
                Q::Fetch::borrow(x);
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
    ///     .map(|(e, &i)| (e, i)) // Clone out of the world
    ///     .collect::<Vec<_>>();
    /// assert!(entities.contains(&(a, 123)));
    /// assert!(entities.contains(&(b, 456)));
    /// ```
    pub fn with<T: Component>(self) -> QueryBorrow<'w, With<T, Q>, C> {
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
    ///     .map(|(e, &i)| (e, i)) // Clone out of the world
    ///     .collect::<Vec<_>>();
    /// assert_eq!(entities, &[(c, 42)]);
    /// ```
    pub fn without<T: Component>(self) -> QueryBorrow<'w, Without<T, Q>, C> {
        self.transform()
    }

    /// Helper to change the type of the query
    fn transform<R: Query<'w, C>>(mut self) -> QueryBorrow<'w, R, C> {
        let x = QueryBorrow {
            meta: self.meta,
            archetypes: self.archetypes,
            borrowed: self.borrowed,
            context: self.context.clone(),
            _marker: PhantomData,
        };
        // Ensure `Drop` won't fire redundantly
        self.borrowed = false;
        x
    }
}

unsafe impl<'w, Q: Query<'w, C>, C: Clone + Sync + 'w> Send for QueryBorrow<'w, Q, C> {}
unsafe impl<'w, Q: Query<'w, C>, C: Clone + Sync + 'w> Sync for QueryBorrow<'w, Q, C> {}

impl<'w, Q: Query<'w, C>, C: Clone + 'w> Drop for QueryBorrow<'w, Q, C> {
    fn drop(&mut self) {
        if self.borrowed {
            for x in self.archetypes {
                if Q::Fetch::access(x) >= Some(Access::Read) {
                    Q::Fetch::release(x);
                }
            }
        }
    }
}

impl<'q, 'w, Q: Query<'w, C>, C: Clone + 'w> IntoIterator for &'q mut QueryBorrow<'w, Q, C> {
    type Item = (Entity, <Q::Fetch as Fetch<'q, 'w, C>>::Item);
    type IntoIter = QueryIter<'q, 'w, Q, C>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Iterator over the set of entities with the components in `Q`
pub struct QueryIter<'q, 'w, Q: Query<'w, C>, C: Clone + 'w> {
    borrow: &'q mut QueryBorrow<'w, Q, C>,
    archetype_index: u32,
    iter: Option<ChunkIter<'w, Q, C>>,
}

unsafe impl<'q, 'w, Q: Query<'w, C>, C: Clone + Sync + 'w> Send for QueryIter<'q, 'w, Q, C> {}
unsafe impl<'q, 'w, Q: Query<'w, C>, C: Clone + Sync + 'w> Sync for QueryIter<'q, 'w, Q, C> {}

impl<'q, 'w, Q: Query<'w, C>, C: Clone + 'w> Iterator for QueryIter<'q, 'w, Q, C> {
    type Item = (Entity, <Q::Fetch as Fetch<'q, 'w, C>>::Item);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.iter {
                None => {
                    let archetype = self.borrow.archetypes.get(self.archetype_index as usize)?;
                    self.archetype_index += 1;
                    unsafe {
                        self.iter = Q::Fetch::get(archetype, 0).map(|fetch| ChunkIter {
                            entities: archetype.entities(),
                            fetch,
                            len: archetype.len(),
                            _context: PhantomData,
                        });
                    }
                }
                Some(ref mut iter) => match unsafe { iter.next(self.borrow.context.clone()) } {
                    None => {
                        self.iter = None;
                        continue;
                    }
                    Some((id, components)) => {
                        return Some((
                            Entity {
                                id,
                                generation: self.borrow.meta[id as usize].generation,
                            },
                            components,
                        ));
                    }
                },
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.len();
        (n, Some(n))
    }
}

impl<'q, 'w, Q: Query<'w, C>, C: Clone + 'w> ExactSizeIterator for QueryIter<'q, 'w, Q, C> {
    fn len(&self) -> usize {
        self.borrow
            .archetypes
            .iter()
            .filter(|&x| Q::Fetch::access(x).is_some())
            .map(|x| x.len() as usize)
            .sum()
    }
}

struct ChunkIter<'c, Q: Query<'c, C>, C: Clone + 'c> {
    entities: NonNull<u32>,
    fetch: Q::Fetch,
    len: u32,
    _context: PhantomData<*const C>,
}

impl<'c, Q: Query<'c, C>, C: Clone + 'c> ChunkIter<'c, Q, C> {
    #[inline]
    unsafe fn next<'a>(
        &mut self,
        context: C,
    ) -> Option<(u32, <Q::Fetch as Fetch<'a, 'c, C>>::Item)> {
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        let entity = self.entities.as_ptr();
        let id = *entity;
        self.entities = NonNull::new_unchecked(entity.add(1));
        Some((id, self.fetch.next(id, context)))
    }
}

/// Batched version of `QueryIter`
pub struct BatchedIter<'q, 'w, Q: Query<'w, C>, C: Clone + 'w> {
    borrow: &'q mut QueryBorrow<'w, Q, C>,
    archetype_index: u32,
    batch_size: u32,
    batch: u32,
}

unsafe impl<'q, 'w, Q: Query<'w, C>, C: Clone + Sync + 'w> Send for BatchedIter<'q, 'w, Q, C> {}
unsafe impl<'q, 'w, Q: Query<'w, C>, C: Clone + Sync + 'w> Sync for BatchedIter<'q, 'w, Q, C> {}

impl<'q, 'w, Q: Query<'w, C>, C: Clone + 'w> Iterator for BatchedIter<'q, 'w, Q, C> {
    type Item = Batch<'q, 'w, Q, C>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let archetype = self.borrow.archetypes.get(self.archetype_index as usize)?;
            let offset = self.batch_size * self.batch;
            if offset >= archetype.len() {
                self.archetype_index += 1;
                self.batch = 0;
                continue;
            }
            if let Some(fetch) = unsafe { Q::Fetch::get(archetype, offset as usize) } {
                self.batch += 1;
                return Some(Batch {
                    _marker: PhantomData,
                    meta: self.borrow.meta,
                    state: ChunkIter {
                        entities: unsafe {
                            NonNull::new_unchecked(
                                archetype.entities().as_ptr().add(offset as usize),
                            )
                        },
                        fetch,
                        len: self.batch_size.min(archetype.len() - offset),
                        _context: PhantomData,
                    },
                    context: self.borrow.context.clone(),
                });
            } else {
                self.archetype_index += 1;
                debug_assert_eq!(
                    self.batch, 0,
                    "query fetch should always reject at the first batch or not at all"
                );
                continue;
            }
        }
    }
}

/// A sequence of entities yielded by `BatchedIter`
pub struct Batch<'q, 'w, Q: Query<'w, C>, C: Clone + 'w> {
    _marker: PhantomData<&'q ()>,
    meta: &'w [EntityMeta],
    state: ChunkIter<'w, Q, C>,
    context: C,
}

impl<'q, 'w, Q: Query<'w, C>, C: Clone + 'w> Iterator for Batch<'q, 'w, Q, C> {
    type Item = (Entity, <Q::Fetch as Fetch<'q, 'w, C>>::Item);

    fn next(&mut self) -> Option<Self::Item> {
        let (id, components) = unsafe { self.state.next(self.context.clone())? };
        Some((
            Entity {
                id,
                generation: self.meta[id as usize].generation,
            },
            components,
        ))
    }
}

unsafe impl<'q, 'w, Q: Query<'w, C>, C: Clone + Sync + 'w> Send for Batch<'q, 'w, Q, C> {}
unsafe impl<'q, 'w, Q: Query<'w, C>, C: Clone + Sync + 'w> Sync for Batch<'q, 'w, Q, C> {}

macro_rules! tuple_impl {
    ($($name: ident),*) => {
        impl<'a, 'z, Z: Clone + 'z, $($name: Fetch<'a, 'z, Z>),*> Fetch<'a, 'z, Z> for ($($name,)*) {
            type Item = ($($name::Item,)*);

            #[allow(unused_variables, unused_mut)]
            fn access(archetype: &Archetype) -> Option<Access> {
                let mut access = Access::Iterate;
                $(
                    access = access.max($name::access(archetype)?);
                )*
                Some(access)
            }

            #[allow(unused_variables)]
            fn borrow(archetype: &Archetype) {
                $($name::borrow(archetype);)*
            }

            #[allow(unused_variables)]
            unsafe fn get(archetype: &'a Archetype, offset: usize) -> Option<Self> {
                Some(($($name::get(archetype, offset)?,)*))
            }

            #[allow(unused_variables)]
            fn release(archetype: &Archetype) {
                $($name::release(archetype);)*
            }

            #[allow(unused_variables)]
            unsafe fn next(&mut self, id: u32, context: Z) -> Self::Item {
                #[allow(non_snake_case)]
                let ($($name,)*) = self;
                ($($name.next(id, context.clone()),)*)
            }
        }

        impl<'z, Z: Clone + 'z, $($name: Query<'z, Z>),*> Query<'z, Z> for ($($name,)*) {
            type Fetch = ($($name::Fetch,)*);
        }
    };
}

//smaller_tuples_too!(tuple_impl, B, A);
smaller_tuples_too!(tuple_impl, O, N, M, L, K, J, I, H, G, F, E, D, C, B, A);

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
