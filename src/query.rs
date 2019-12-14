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

use std::any::{type_name, TypeId};
use std::marker::PhantomData;
use std::ptr::NonNull;

use crate::archetype::Archetype;
use crate::borrow::BorrowState;
use crate::world::EntityMeta;
use crate::{Component, Entity};

/// A collection of component types to fetch from a `World`
pub trait Query<'a>: Sized {
    /// Helper used to process the query
    type Fetch: Fetch<'a, Item = Self>;
    /// Dynamically borrow the component types to be accessed, or panic if borrows can't be acquired
    fn borrow(state: &BorrowState);
    /// Release dynamic borrows
    fn release(state: &BorrowState);
}

/// Streaming iterators over contiguous homogeneous ranges of components
pub trait Fetch<'a>: Sized {
    /// Type of value to be fetched
    type Item;
    /// Construct a `Fetch` for `archetype` if it should be traversed
    fn get(archetype: &'a Archetype) -> Option<Self>;
    /// Access the next item in this archetype without bounds checking
    unsafe fn next(&mut self) -> Self::Item;
}

impl<'a, T: Component> Query<'a> for &'a T {
    type Fetch = FetchRead<T>;
    fn borrow(state: &BorrowState) {
        state.borrow(TypeId::of::<T>(), type_name::<T>())
    }
    fn release(state: &BorrowState) {
        state.release(TypeId::of::<T>())
    }
}

#[doc(hidden)]
pub struct FetchRead<T>(NonNull<T>);

impl<'a, T: Component> Fetch<'a> for FetchRead<T> {
    type Item = &'a T;
    fn get(archetype: &'a Archetype) -> Option<Self> {
        archetype.data::<T>().map(Self)
    }
    unsafe fn next(&mut self) -> &'a T {
        let x = self.0.as_ptr();
        self.0 = NonNull::new_unchecked(x.add(1));
        &*x
    }
}

impl<'a, T: Component> Query<'a> for &'a mut T {
    type Fetch = FetchWrite<T>;
    fn borrow(state: &BorrowState) {
        state.borrow_mut(TypeId::of::<T>(), type_name::<T>())
    }
    fn release(state: &BorrowState) {
        state.release_mut(TypeId::of::<T>())
    }
}

#[doc(hidden)]
pub struct FetchWrite<T>(NonNull<T>);

impl<'a, T: Component> Fetch<'a> for FetchWrite<T> {
    type Item = &'a mut T;
    fn get(archetype: &'a Archetype) -> Option<Self> {
        archetype.data::<T>().map(Self)
    }
    unsafe fn next(&mut self) -> &'a mut T {
        let x = self.0.as_ptr();
        self.0 = NonNull::new_unchecked(x.add(1));
        &mut *x
    }
}

impl<'a, T: Query<'a>> Query<'a> for Option<T> {
    type Fetch = TryFetch<T::Fetch>;
    fn borrow(state: &BorrowState) {
        T::borrow(state);
    }
    fn release(state: &BorrowState) {
        T::release(state);
    }
}

#[doc(hidden)]
pub struct TryFetch<T>(Option<T>);

impl<'a, T: Fetch<'a>> Fetch<'a> for TryFetch<T> {
    type Item = Option<T::Item>;
    fn get(archetype: &'a Archetype) -> Option<Self> {
        Some(Self(T::get(archetype)))
    }
    unsafe fn next(&mut self) -> Option<T::Item> {
        Some(self.0.as_mut()?.next())
    }
}

/// Iterator over the set of entities with the components in `Q`
pub struct QueryIter<'a, Q: Query<'a>> {
    borrows: &'a BorrowState,
    meta: &'a [EntityMeta],
    archetypes: std::slice::Iter<'a, Archetype>,
    iter: Option<ChunkIter<'a, Q::Fetch>>,
}

impl<'a, Q: Query<'a>> QueryIter<'a, Q> {
    pub(crate) fn new(
        borrows: &'a BorrowState,
        meta: &'a [EntityMeta],
        archetypes: &'a [Archetype],
    ) -> Self {
        Q::borrow(borrows);
        Self {
            borrows,
            meta,
            archetypes: archetypes.iter(),
            iter: None,
        }
    }
}

impl<'a, Q: Query<'a>> Drop for QueryIter<'a, Q> {
    fn drop(&mut self) {
        Q::release(self.borrows);
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

macro_rules! tuple_impl {
    ($($name: ident),*) => {
        impl<'a, $($name: Fetch<'a>),*> Fetch<'a> for ($($name,)*) {
            type Item = ($($name::Item,)*);
            #[allow(unused_variables)]
            fn get(archetype: &'a Archetype) -> Option<Self> {
                Some(($($name::get(archetype)?,)*))
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
            fn borrow(state: &BorrowState) {
                $($name::borrow(state);)*
            }
            fn release(state: &BorrowState) {
                $($name::release(state);)*
            }
        }
    }
}

tuple_impl!();
tuple_impl!(A);
tuple_impl!(A, B);
tuple_impl!(A, B, C);
tuple_impl!(A, B, C, D);
tuple_impl!(A, B, C, D, E);
tuple_impl!(A, B, C, D, E, F);
tuple_impl!(A, B, C, D, E, F, G);
tuple_impl!(A, B, C, D, E, F, G, H);
tuple_impl!(A, B, C, D, E, F, G, H, I);
tuple_impl!(A, B, C, D, E, F, G, H, I, J);
tuple_impl!(A, B, C, D, E, F, G, H, I, J, K);
tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L);
tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M);
tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N);
tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z, AA);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z, AA, AB);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z, AA, AB, AC);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z, AA, AB, AC, AD);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z, AA, AB, AC, AD, AE);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z, AA, AB, AC, AD, AE, AF);
// tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z, AA, AB, AC, AD, AE, AF, AG);
