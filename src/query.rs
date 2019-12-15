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
pub trait Query<'a>: Sized {
    /// Helper used to process the query
    type Fetch: Fetch<'a, Item = Self>;
}

/// Streaming iterators over contiguous homogeneous ranges of components
pub trait Fetch<'a>: Sized {
    /// Type of value to be fetched
    type Item;
    /// Whether `get` would succeed
    fn wants(archetype: &Archetype) -> bool;
    /// Construct a `Fetch` for `archetype` if it should be traversed
    fn get(archetype: &'a Archetype) -> Option<Self>;
    /// Release dynamic borrows acquired by `get`
    fn release(archetype: &Archetype);
    /// Access the next item in this archetype without bounds checking
    unsafe fn next(&mut self) -> Self::Item;
}

impl<'a, T: Component> Query<'a> for &'a T {
    type Fetch = FetchRead<T>;
}

#[doc(hidden)]
pub struct FetchRead<T>(NonNull<T>);

impl<'a, T: Component> Fetch<'a> for FetchRead<T> {
    type Item = &'a T;
    fn wants(archetype: &Archetype) -> bool {
        archetype.has::<T>()
    }
    fn get(archetype: &'a Archetype) -> Option<Self> {
        archetype.borrow::<T>().map(Self)
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

impl<'a, T: Component> Query<'a> for &'a mut T {
    type Fetch = FetchWrite<T>;
}

#[doc(hidden)]
pub struct FetchWrite<T>(NonNull<T>);

impl<'a, T: Component> Fetch<'a> for FetchWrite<T> {
    type Item = &'a mut T;
    fn wants(archetype: &Archetype) -> bool {
        archetype.has::<T>()
    }
    fn get(archetype: &'a Archetype) -> Option<Self> {
        archetype.borrow_mut::<T>().map(Self)
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

impl<'a, T: Query<'a>> Query<'a> for Option<T> {
    type Fetch = TryFetch<T::Fetch>;
}

#[doc(hidden)]
pub struct TryFetch<T>(Option<T>);

impl<'a, T: Fetch<'a>> Fetch<'a> for TryFetch<T> {
    type Item = Option<T::Item>;
    fn wants(_: &Archetype) -> bool {
        true
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

/// Iterator over the set of entities with the components in `Q`
pub struct QueryIter<'a, Q: Query<'a>> {
    meta: &'a [EntityMeta],
    archetypes: core::slice::Iter<'a, Archetype>,
    iter: Option<ChunkIter<'a, Q::Fetch>>,
}

impl<'a, Q: Query<'a>> QueryIter<'a, Q> {
    pub(crate) fn new(meta: &'a [EntityMeta], archetypes: &'a [Archetype]) -> Self {
        Self {
            meta,
            archetypes: archetypes.iter(),
            iter: None,
        }
    }
}

impl<'a, Q: Query<'a>> Iterator for QueryIter<'a, Q> {
    type Item = (Entity, Q);
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.iter {
                None => {
                    let archetype = self.archetypes.next()?;
                    self.iter = Q::Fetch::get(archetype).map(|fetch| ChunkIter {
                        archetype,
                        entities: archetype.entities(),
                        fetch,
                        len: archetype.len(),
                        _marker: PhantomData,
                    });
                }
                Some(ref mut iter) => match iter.next() {
                    None => {
                        self.iter = None;
                    }
                    Some((id, item)) => {
                        return Some((
                            Entity {
                                id,
                                generation: self.meta[id as usize].generation,
                            },
                            item,
                        ));
                    }
                },
            }
        }
    }
}

struct ChunkIter<'a, T: Fetch<'a>> {
    archetype: &'a Archetype,
    entities: NonNull<u32>,
    fetch: T,
    len: usize,
    _marker: PhantomData<&'a ()>,
}

impl<'a, T: Fetch<'a>> Iterator for ChunkIter<'a, T> {
    type Item = (u32, T::Item);
    fn next(&mut self) -> Option<Self::Item> {
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        let entity = self.entities.as_ptr();
        unsafe {
            self.entities = NonNull::new_unchecked(entity.add(1));
            Some((*entity, self.fetch.next()))
        }
    }
}

impl<'a, T: Fetch<'a>> Drop for ChunkIter<'a, T> {
    fn drop(&mut self) {
        T::release(self.archetype);
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

        #[allow(unused_variables)]
        impl<'a, $($name: Query<'a>),*> Query<'a> for ($($name,)*) {
            type Fetch = ($($name::Fetch,)*);
        }
    }
}

smaller_tuples_too!(tuple_impl, O, N, M, L, K, J, I, H, G, F, E, D, C, B, A);
