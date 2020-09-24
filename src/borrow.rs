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

use core::any::TypeId;
use core::ops::{Deref, DerefMut};
use core::ptr::NonNull;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::archetype::Archetype;
use crate::{Component, Entity, MissingComponent, SmartComponent};

pub struct AtomicBorrow(AtomicUsize);

impl AtomicBorrow {
    pub const fn new() -> Self {
        Self(AtomicUsize::new(0))
    }

    pub fn borrow(&self) -> bool {
        let value = self.0.fetch_add(1, Ordering::Acquire).wrapping_add(1);
        if value == 0 {
            // Wrapped, this borrow is invalid!
            core::panic!()
        }
        if value & UNIQUE_BIT != 0 {
            self.0.fetch_sub(1, Ordering::Release);
            false
        } else {
            true
        }
    }

    pub fn borrow_mut(&self) -> bool {
        self.0
            .compare_exchange(0, UNIQUE_BIT, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }

    pub fn release(&self) {
        let value = self.0.fetch_sub(1, Ordering::Release);
        debug_assert!(value != 0, "unbalanced release");
        debug_assert!(value & UNIQUE_BIT == 0, "shared release of unique borrow");
    }

    pub fn release_mut(&self) {
        let value = self.0.fetch_and(!UNIQUE_BIT, Ordering::Release);
        debug_assert_ne!(value & UNIQUE_BIT, 0, "unique release of shared borrow");
    }
}

const UNIQUE_BIT: usize = !(usize::max_value() >> 1);

/// Shared borrow of an entity's component
#[derive(Clone)]
pub struct Ref<'a, T: SmartComponent<C>, C: Clone + 'a = ()> {
    archetype: &'a Archetype,
    target: NonNull<T>,
    entity: Entity,
    context: C,
}

impl<'a, T: SmartComponent<C>, C: Clone + 'a> Ref<'a, T, C> {
    pub(crate) unsafe fn new(
        archetype: &'a Archetype,
        index: u32,
        entity: Entity,
        context: C,
    ) -> Result<Self, MissingComponent> {
        let target = NonNull::new_unchecked(
            archetype
                .get::<T>()
                .ok_or_else(MissingComponent::new::<T>)?
                .as_ptr()
                .add(index as usize),
        );
        archetype.borrow::<T>();
        Ok(Self {
            archetype,
            target,
            entity,
            context,
        })
    }
}

unsafe impl<T: SmartComponent<C>, C: Clone + Sync> Send for Ref<'_, T, C> {}
unsafe impl<T: SmartComponent<C>, C: Clone + Sync> Sync for Ref<'_, T, C> {}

impl<'a, T: SmartComponent<C>, C: Clone> Drop for Ref<'a, T, C> {
    fn drop(&mut self) {
        self.archetype.release::<T>();
    }
}

impl<'a, T: SmartComponent<C>, C: Clone> Deref for Ref<'a, T, C> {
    type Target = T;
    fn deref(&self) -> &T {
        let value = unsafe { self.target.as_ref() };
        value.on_borrow(self.entity, self.context.clone());
        value
    }
}

/// Unique borrow of an entity's component
pub struct RefMut<'a, T: SmartComponent<C>, C: Clone = ()> {
    archetype: &'a Archetype,
    target: NonNull<T>,
    entity: Entity,
    context: C,
}

impl<'a, T: SmartComponent<C>, C: Clone> RefMut<'a, T, C> {
    pub(crate) unsafe fn new(
        archetype: &'a Archetype,
        index: u32,
        entity: Entity,
        context: C,
    ) -> Result<Self, MissingComponent> {
        let target = NonNull::new_unchecked(
            archetype
                .get::<T>()
                .ok_or_else(MissingComponent::new::<T>)?
                .as_ptr()
                .add(index as usize),
        );
        archetype.borrow_mut::<T>();
        Ok(Self {
            archetype,
            target,
            entity,
            context,
        })
    }
}

unsafe impl<T: SmartComponent<C>, C: Clone + Sync> Send for RefMut<'_, T, C> {}
unsafe impl<T: SmartComponent<C>, C: Clone + Sync> Sync for RefMut<'_, T, C> {}

impl<'a, T: SmartComponent<C>, C: Clone> Drop for RefMut<'a, T, C> {
    fn drop(&mut self) {
        self.archetype.release_mut::<T>();
    }
}

impl<'a, T: Component> Deref for RefMut<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        let value = unsafe { self.target.as_ref() };
        value.on_borrow(self.entity, self.context.clone());
        value
    }
}

impl<'a, T: Component> DerefMut for RefMut<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        let value = unsafe { self.target.as_mut() };
        value.on_borrow_mut(self.entity, self.context.clone());
        value
    }
}

/// Handle to an entity with any component types
#[derive(Copy, Clone)]
pub struct EntityRef<'a, C: Clone = ()> {
    archetype: Option<&'a Archetype>,
    index: u32,
    entity: Entity,
    context: C,
}

impl<'a, C: Clone> EntityRef<'a, C> {
    /// Construct a `Ref` for an entity with no components
    pub(crate) fn empty(entity: Entity, context: C) -> Self {
        Self {
            archetype: None,
            index: 0,
            entity,
            context,
        }
    }

    pub(crate) unsafe fn new(
        archetype: &'a Archetype,
        index: u32,
        entity: Entity,
        context: C,
    ) -> Self {
        Self {
            archetype: Some(archetype),
            index,
            entity,
            context,
        }
    }

    /// Borrow the component of type `T`, if it exists
    ///
    /// Panics if the component is already uniquely borrowed from another entity with the same
    /// components.
    pub fn get<T: SmartComponent<C>>(&self) -> Option<Ref<'a, T, C>> {
        Some(unsafe {
            Ref::new(
                self.archetype?,
                self.index,
                self.entity,
                self.context.clone(),
            )
            .ok()?
        })
    }

    /// Uniquely borrow the component of type `T`, if it exists
    ///
    /// Panics if the component is already borrowed from another entity with the same components.
    pub fn get_mut<T: SmartComponent<C>>(&self) -> Option<RefMut<'a, T, C>> {
        Some(unsafe {
            RefMut::new(
                self.archetype?,
                self.index,
                self.entity,
                self.context.clone(),
            )
            .ok()?
        })
    }

    /// Enumerate the types of the entity's components
    ///
    /// Convenient for dispatching component-specific logic for a single entity. For example, this
    /// can be combined with a `HashMap<TypeId, Box<dyn Handler>>` where `Handler` is some
    /// user-defined trait with methods for serialization, or to be called after spawning or before
    /// despawning to maintain secondary indices.
    pub fn component_types(&self) -> impl Iterator<Item = TypeId> + 'a {
        self.archetype
            .into_iter()
            .flat_map(|arch| arch.types().iter().map(|ty| ty.id()))
    }
}

unsafe impl<'a, C: Clone + Sync> Send for EntityRef<'a, C> {}
unsafe impl<'a, C: Clone + Sync> Sync for EntityRef<'a, C> {}
