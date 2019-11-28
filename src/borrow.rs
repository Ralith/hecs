use std::any::{type_name, TypeId};
use std::ops::{Deref, DerefMut};
use std::ptr::NonNull;

use fxhash::FxHashMap;
use lock_api::RawRwLock as _;
use parking_lot::RawRwLock;

use crate::archetype::Archetype;
use crate::world::Component;

#[derive(Default)]
pub struct BorrowState {
    states: FxHashMap<TypeId, RawRwLock>,
}

impl BorrowState {
    pub(crate) fn ensure(&mut self, ty: TypeId) {
        use std::collections::hash_map::Entry;
        match self.states.entry(ty) {
            Entry::Vacant(e) => {
                e.insert(RawRwLock::INIT);
            }
            Entry::Occupied(_) => {}
        }
    }

    pub fn borrow(&self, ty: TypeId, name: &str) {
        assert!(
            self.states.get(&ty).map_or(true, |x| x.try_lock_shared()),
            "{} already borrowed uniquely",
            name
        );
    }

    pub fn borrow_mut(&self, ty: TypeId, name: &str) {
        assert!(
            self.states
                .get(&ty)
                .map_or(true, |x| x.try_lock_exclusive()),
            "{} already borrowed",
            name
        );
    }

    pub fn release(&self, ty: TypeId) {
        self.states.get(&ty).map(|x| x.unlock_shared());
    }

    pub fn release_mut(&self, ty: TypeId) {
        self.states.get(&ty).map(|x| x.unlock_exclusive());
    }
}

/// Shared borrow of a particular component of a particular entity
#[derive(Clone)]
pub struct Ref<'a, T: Component> {
    borrow: &'a BorrowState,
    target: NonNull<T>,
}

impl<'a, T: Component> Ref<'a, T> {
    pub(crate) unsafe fn new(borrow: &'a BorrowState, target: NonNull<T>) -> Self {
        borrow.borrow(TypeId::of::<T>(), type_name::<T>());
        Self {
            borrow,
            target: target,
        }
    }
}

impl<'a, T: Component> Drop for Ref<'a, T> {
    fn drop(&mut self) {
        self.borrow.release(TypeId::of::<T>());
    }
}

impl<'a, T: Component> Deref for Ref<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { self.target.as_ref() }
    }
}

/// Unique borrow of a particular component of a particular entity
pub struct RefMut<'a, T: Component> {
    borrow: &'a BorrowState,
    target: NonNull<T>,
}

impl<'a, T: Component> RefMut<'a, T> {
    pub(crate) fn new(borrow: &'a BorrowState, target: NonNull<T>) -> Self {
        borrow.borrow_mut(TypeId::of::<T>(), type_name::<T>());
        Self { borrow, target }
    }
}

impl<'a, T: Component> Drop for RefMut<'a, T> {
    fn drop(&mut self) {
        self.borrow.release_mut(TypeId::of::<T>());
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

/// Handle to an entity with any component types
#[derive(Copy, Clone)]
pub struct EntityRef<'a> {
    borrow: &'a BorrowState,
    archetype: &'a Archetype,
    index: u32,
}

impl<'a> EntityRef<'a> {
    pub(crate) fn new(borrow: &'a BorrowState, archetype: &'a Archetype, index: u32) -> Self {
        Self {
            borrow,
            archetype,
            index,
        }
    }

    /// Borrow the component of type `T`, if it exists
    ///
    /// Panics if a component of type `T` is already uniquely borrowed from the world
    pub fn get<T: Component>(&self) -> Option<Ref<'a, T>> {
        Some(unsafe { Ref::new(self.borrow, self.archetype.get(self.index)?) })
    }

    /// Uniquely borrow the component of type `T`, if it exists
    ///
    /// Panics if a component of type `T` is already borrowed from the world
    pub fn get_mut<T: Component>(&self) -> Option<RefMut<'a, T>> {
        Some(unsafe { RefMut::new(self.borrow, self.archetype.get(self.index)?) })
    }
}
