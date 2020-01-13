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

use core::marker::PhantomData;
use core::ptr::NonNull;

use crate::archetype::Archetype;
use crate::world::EntityMeta;
use crate::{Component, Entity};

/// A collection of component types to fetch from a `World`
pub trait Query {
    #[doc(hidden)]
    type Fetch: for<'a> Fetch<'a>;
}

/// Streaming iterators over contiguous homogeneous ranges of components
pub trait Fetch<'a>: Sized {
    /// Type of value to be fetched
    type Item;

    /// Whether `get` will borrow from `archetype`
    fn wants(archetype: &Archetype) -> bool;

    /// Acquire dynamic borrows from `archetype`
    fn borrow(archetype: &Archetype);
    /// Construct a `Fetch` for `archetype` if it should be traversed
    fn get(archetype: &'a Archetype) -> Option<Self>;
    /// Release dynamic borrows acquired by `borrow`
    fn release(archetype: &Archetype);

    /// Access the next item in this archetype without bounds checking
    ///
    /// # Safety
    /// - Must only be called after `borrow`
    /// - `release` must not be called while `'a` is still live
    /// - bounds-checking must be performed externally
    unsafe fn next(&mut self) -> Self::Item;
}

impl<'a, T: Component> Query for &'a T {
    type Fetch = FetchRead<T>;
}

#[doc(hidden)]
pub struct FetchRead<T>(NonNull<T>);

impl<'a, T: Component> Fetch<'a> for FetchRead<T> {
    type Item = &'a T;
    fn wants(archetype: &Archetype) -> bool {
        archetype.has::<T>()
    }

    fn borrow(archetype: &Archetype) {
        archetype.borrow::<T>();
    }
    fn get(archetype: &'a Archetype) -> Option<Self> {
        archetype.get::<T>().map(Self)
    }
    fn release(archetype: &Archetype) {
        archetype.release::<T>();
    }

    unsafe fn next(&mut self) -> &'a T {
        let x = self.0.as_ptr();
        self.0 = NonNull::new_unchecked(x.add(1));
        &*x
    }
}

impl<'a, T: Component> Query for &'a mut T {
    type Fetch = FetchWrite<T>;
}

#[doc(hidden)]
pub struct FetchWrite<T>(NonNull<T>);

impl<'a, T: Component> Fetch<'a> for FetchWrite<T> {
    type Item = &'a mut T;
    fn wants(archetype: &Archetype) -> bool {
        archetype.has::<T>()
    }

    fn borrow(archetype: &Archetype) {
        archetype.borrow_mut::<T>();
    }
    fn get(archetype: &'a Archetype) -> Option<Self> {
        archetype.get::<T>().map(Self)
    }
    fn release(archetype: &Archetype) {
        archetype.release_mut::<T>();
    }

    unsafe fn next(&mut self) -> &'a mut T {
        let x = self.0.as_ptr();
        self.0 = NonNull::new_unchecked(x.add(1));
        &mut *x
    }
}

impl<T: Query> Query for Option<T> {
    type Fetch = TryFetch<T::Fetch>;
}

#[doc(hidden)]
pub struct TryFetch<T>(Option<T>);

impl<'a, T: Fetch<'a>> Fetch<'a> for TryFetch<T> {
    type Item = Option<T::Item>;
    fn wants(_: &Archetype) -> bool {
        true
    }

    fn borrow(archetype: &Archetype) {
        T::borrow(archetype)
    }
    fn get(archetype: &'a Archetype) -> Option<Self> {
        Some(Self(T::get(archetype)))
    }
    fn release(archetype: &Archetype) {
        T::release(archetype)
    }

    unsafe fn next(&mut self) -> Option<T::Item> {
        Some(self.0.as_mut()?.next())
    }
}

/// Query transformer skipping entities that have a `T` component
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

impl<'a, T: Component, F: Fetch<'a>> Fetch<'a> for FetchWithout<T, F> {
    type Item = F::Item;
    fn wants(archetype: &Archetype) -> bool {
        !archetype.has::<T>()
    }

    fn borrow(archetype: &Archetype) {
        F::borrow(archetype)
    }
    fn get(archetype: &'a Archetype) -> Option<Self> {
        if archetype.has::<T>() {
            return None;
        }
        Some(Self(F::get(archetype)?, PhantomData))
    }
    fn release(archetype: &Archetype) {
        F::release(archetype)
    }

    unsafe fn next(&mut self) -> F::Item {
        self.0.next()
    }
}

/// Query transformer skipping entities that do not have a `T` component
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

impl<'a, T: Component, F: Fetch<'a>> Fetch<'a> for FetchWith<T, F> {
    type Item = F::Item;
    fn wants(archetype: &Archetype) -> bool {
        archetype.has::<T>()
    }

    fn borrow(archetype: &Archetype) {
        F::borrow(archetype)
    }
    fn get(archetype: &'a Archetype) -> Option<Self> {
        if !archetype.has::<T>() {
            return None;
        }
        Some(Self(F::get(archetype)?, PhantomData))
    }
    fn release(archetype: &Archetype) {
        F::release(archetype)
    }

    unsafe fn next(&mut self) -> F::Item {
        self.0.next()
    }
}

/// A borrow of a `World` sufficient to execute the query `Q`
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
    ///
    /// Must be called only once per query.
    pub fn iter<'q>(&'q mut self) -> QueryIter<'q, 'w, Q> {
        if self.borrowed {
            panic!(
                "called QueryBorrow::iter twice on the same borrow; construct a new query instead"
            );
        }
        for x in self.archetypes {
            // TODO: Release prior borrows on failure?
            if Q::Fetch::wants(x) {
                Q::Fetch::borrow(x);
            }
        }
        self.borrowed = true;
        QueryIter {
            borrow: self,
            archetype_index: 0,
            iter: None,
        }
    }
}

unsafe impl<'w, Q: Query> Send for QueryBorrow<'w, Q> {}
unsafe impl<'w, Q: Query> Sync for QueryBorrow<'w, Q> {}

impl<'w, Q: Query> Drop for QueryBorrow<'w, Q> {
    fn drop(&mut self) {
        if self.borrowed {
            for x in self.archetypes {
                if Q::Fetch::wants(x) {
                    Q::Fetch::release(x);
                }
            }
        }
    }
}

impl<'q, 'w, Q: Query> IntoIterator for &'q mut QueryBorrow<'w, Q> {
    type Item = (Entity, <Q::Fetch as Fetch<'q>>::Item);
    type IntoIter = QueryIter<'q, 'w, Q>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

struct ChunkIter<Q: Query> {
    entities: NonNull<u32>,
    fetch: Q::Fetch,
    len: usize,
}

/// Iterator over the set of entities with the components in `Q`
pub struct QueryIter<'q, 'w, Q: Query> {
    borrow: &'q mut QueryBorrow<'w, Q>,
    archetype_index: u32,
    iter: Option<ChunkIter<Q>>,
}

unsafe impl<'q, 'w, Q: Query> Send for QueryIter<'q, 'w, Q> {}
unsafe impl<'q, 'w, Q: Query> Sync for QueryIter<'q, 'w, Q> {}

impl<'q, 'w, Q: Query> Iterator for QueryIter<'q, 'w, Q> {
    type Item = (Entity, <Q::Fetch as Fetch<'q>>::Item);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.iter {
                None => {
                    let archetype = self.borrow.archetypes.get(self.archetype_index as usize)?;
                    self.archetype_index += 1;
                    self.iter = Q::Fetch::get(archetype).map(|fetch| ChunkIter {
                        entities: archetype.entities(),
                        fetch,
                        len: archetype.len(),
                    });
                }
                Some(ref mut iter) => {
                    if iter.len == 0 {
                        self.iter = None;
                        continue;
                    }
                    iter.len -= 1;
                    let entity = iter.entities.as_ptr();
                    unsafe {
                        iter.entities = NonNull::new_unchecked(entity.add(1));
                        return Some((
                            Entity {
                                id: *entity,
                                generation: self.borrow.meta[*entity as usize].generation,
                            },
                            iter.fetch.next(),
                        ));
                    }
                }
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.len();
        (n, Some(n))
    }
}

impl<'q, 'w, Q: Query> ExactSizeIterator for QueryIter<'q, 'w, Q> {
    fn len(&self) -> usize {
        self.borrow
            .archetypes
            .iter()
            .filter(|&x| Q::Fetch::wants(x))
            .map(|x| x.len())
            .sum()
    }
}

macro_rules! tuple_impl {
    ($($name: ident),*) => {
        impl<'a, $($name: Fetch<'a>),*> Fetch<'a> for ($($name,)*) {
            type Item = ($($name::Item,)*);
            #[allow(unused_variables)]
            fn wants(archetype: &Archetype) -> bool {
                $($name::wants(archetype) &&)* true
            }

            #[allow(unused_variables)]
            fn borrow(archetype: &Archetype) {
                $($name::borrow(archetype);)*
            }
            #[allow(unused_variables)]
            fn get(archetype: &'a Archetype) -> Option<Self> {
                Some(($($name::get(archetype)?,)*))
            }
            #[allow(unused_variables)]
            fn release(archetype: &Archetype) {
                $($name::release(archetype);)*
            }

            unsafe fn next(&mut self) -> Self::Item {
                #[allow(non_snake_case)]
                let ($($name,)*) = self;
                ($($name.next(),)*)
            }
        }

        impl<$($name: Query),*> Query for ($($name,)*) {
            type Fetch = ($($name::Fetch,)*);
        }
    };
}

//smaller_tuples_too!(tuple_impl, B, A);
smaller_tuples_too!(tuple_impl, O, N, M, L, K, J, I, H, G, F, E, D, C, B, A);
