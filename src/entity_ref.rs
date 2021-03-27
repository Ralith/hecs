use core::any::TypeId;
use core::ops::{Deref, DerefMut};
use core::ptr::NonNull;

use crate::archetype::Archetype;
use crate::{Component, MissingComponent};

/// Handle to an entity with any component types
#[derive(Copy, Clone)]
pub struct EntityRef<'a> {
    archetype: &'a Archetype,
    index: u32,
}

impl<'a> EntityRef<'a> {
    pub(crate) unsafe fn new(archetype: &'a Archetype, index: u32) -> Self {
        Self { archetype, index }
    }

    /// Borrow the component of type `T`, if it exists
    ///
    /// Panics if the component is already uniquely borrowed from another entity with the same
    /// components.
    pub fn get<T: Component>(&self) -> Option<Ref<'a, T>> {
        Some(unsafe { Ref::new(self.archetype, self.index).ok()? })
    }

    /// Uniquely borrow the component of type `T`, if it exists
    ///
    /// Panics if the component is already borrowed from another entity with the same components.
    pub fn get_mut<T: Component>(&self) -> Option<RefMut<'a, T>> {
        Some(unsafe { RefMut::new(self.archetype, self.index).ok()? })
    }

    /// Enumerate the types of the entity's components
    ///
    /// Convenient for dispatching component-specific logic for a single entity. For example, this
    /// can be combined with a `HashMap<TypeId, Box<dyn Handler>>` where `Handler` is some
    /// user-defined trait with methods for serialization, or to be called after spawning or before
    /// despawning to maintain secondary indices.
    pub fn component_types(&self) -> impl Iterator<Item = TypeId> + 'a {
        self.archetype.types().iter().map(|ty| ty.id())
    }

    /// Number of components in this entity
    pub fn len(&self) -> usize {
        self.archetype.types().len()
    }

    /// Shorthand for `self.len() == 0`
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

unsafe impl<'a> Send for EntityRef<'a> {}
unsafe impl<'a> Sync for EntityRef<'a> {}

/// Shared borrow of an entity's component
#[derive(Clone)]
pub struct Ref<'a, T: Component> {
    archetype: &'a Archetype,
    target: NonNull<T>,
}

impl<'a, T: Component> Ref<'a, T> {
    pub(crate) unsafe fn new(
        archetype: &'a Archetype,
        index: u32,
    ) -> Result<Self, MissingComponent> {
        let target = NonNull::new_unchecked(
            archetype
                .get_base::<T>()
                .ok_or_else(MissingComponent::new::<T>)?
                .as_ptr()
                .add(index as usize),
        );
        archetype.borrow::<T>();
        Ok(Self { archetype, target })
    }
}

unsafe impl<T: Component> Send for Ref<'_, T> {}
unsafe impl<T: Component> Sync for Ref<'_, T> {}

impl<'a, T: Component> Drop for Ref<'a, T> {
    fn drop(&mut self) {
        self.archetype.release::<T>();
    }
}

impl<'a, T: Component> Deref for Ref<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { self.target.as_ref() }
    }
}

/// Unique borrow of an entity's component
pub struct RefMut<'a, T: Component> {
    archetype: &'a Archetype,
    target: NonNull<T>,
}

impl<'a, T: Component> RefMut<'a, T> {
    pub(crate) unsafe fn new(
        archetype: &'a Archetype,
        index: u32,
    ) -> Result<Self, MissingComponent> {
        let target = NonNull::new_unchecked(
            archetype
                .get_base::<T>()
                .ok_or_else(MissingComponent::new::<T>)?
                .as_ptr()
                .add(index as usize),
        );
        archetype.borrow_mut::<T>();
        Ok(Self { archetype, target })
    }
}

unsafe impl<T: Component> Send for RefMut<'_, T> {}
unsafe impl<T: Component> Sync for RefMut<'_, T> {}

impl<'a, T: Component> Drop for RefMut<'a, T> {
    fn drop(&mut self) {
        self.archetype.release_mut::<T>();
    }
}

impl<'a, T: Component> Deref for RefMut<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { self.target.as_ref() }
    }
}

impl<'a, T: Component> DerefMut for RefMut<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { self.target.as_mut() }
    }
}
