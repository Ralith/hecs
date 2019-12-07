use std::any::{type_name, TypeId};
use std::ops::{Deref, DerefMut};
use std::ptr::NonNull;

use fxhash::FxHashMap;
use lock_api::RawRwLock as _;
use parking_lot::RawRwLock;

use crate::archetype::Archetype;
use crate::world::Component;

/// Tracks which components of a world are borrowed in what ways
#[derive(Default)]
pub struct BorrowState {
    states: FxHashMap<TypeId, RawRwLock>,
}

impl BorrowState {
    pub(crate) fn ensure(&mut self, ty: TypeId) {
        self.states.entry(ty).or_insert(RawRwLock::INIT);
    }

    /// Acquire a shared borrow
    pub fn borrow(&self, ty: TypeId, name: &str) {
        if self.states.get(&ty).map_or(false, |x| !x.try_lock_shared()) {
            panic!("{} already borrowed uniquely", name);
        }
    }

    /// Acquire a unique borrow
    pub fn borrow_mut(&self, ty: TypeId, name: &str) {
        if self
            .states
            .get(&ty)
            .map_or(false, |x| !x.try_lock_exclusive())
        {
            panic!("{} already borrowed", name);
        }
    }

    /// Release a shared borrow
    pub fn release(&self, ty: TypeId) {
        if let Some(x) = self.states.get(&ty) {
            x.unlock_shared();
        }
    }

    /// Release a unique borrow
    pub fn release_mut(&self, ty: TypeId) {
        if let Some(x) = self.states.get(&ty) {
            x.unlock_exclusive();
        }
    }
}

/// Shared borrow of an entity's component
#[derive(Clone)]
pub struct Ref<'a, T: Component> {
    borrow: &'a BorrowState,
    target: NonNull<T>,
}

impl<'a, T: Component> Ref<'a, T> {
    pub(crate) unsafe fn new(borrow: &'a BorrowState, target: NonNull<T>) -> Self {
        borrow.borrow(TypeId::of::<T>(), type_name::<T>());
        Self { borrow, target }
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

/// Unique borrow of an entity's component
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
