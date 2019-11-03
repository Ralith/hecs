mod join;
mod vec;

pub use join::*;
pub use vec::*;

use std::marker::PhantomData;
use std::mem;
use std::ops::{Deref, DerefMut};
use std::sync::MutexGuard;

use downcast_rs::{impl_downcast, Downcast};
use hibitset::{BitIter, BitSet, BitSetLike};

pub(crate) trait AbstractStorage: Downcast + Send + 'static {
    /// If `i` is occupied, drop its contents
    fn free(&mut self, i: u32);
}
impl_downcast!(AbstractStorage);

impl<S: Storage> AbstractStorage for Masked<S> {
    fn free(&mut self, i: u32) {
        self.remove(i);
    }
}

/// A storage with external occupancy information
pub trait Storage: Default + Send + 'static {
    type Component;

    /// Insert `x` into empty slot `i`
    unsafe fn insert(&mut self, i: u32, x: Self::Component);
    /// Remove a value from occupied slot `i`
    unsafe fn remove(&mut self, i: u32) -> Self::Component;
    /// Borrow a value from occupied slot `i`
    unsafe fn get(&self, i: u32) -> &Self::Component;
    /// Mutably borrow a value from occupied slot `i`
    unsafe fn get_mut(&mut self, i: u32) -> &mut Self::Component;
}

#[derive(Default)]
pub struct Masked<S: Storage> {
    inner: S,
    mask: BitSet,
}

impl<S: Storage> Masked<S> {
    pub(crate) fn new(x: S) -> Self {
        Self {
            inner: x,
            mask: BitSet::new(),
        }
    }

    pub(crate) fn insert(&mut self, i: u32, x: S::Component) -> Option<S::Component> {
        unsafe {
            let old = match self.mask.add(i) {
                true => Some(self.inner.remove(i)),
                false => None,
            };
            self.inner.insert(i, x);
            old
        }
    }

    pub(crate) fn remove(&mut self, i: u32) -> Option<S::Component> {
        unsafe {
            match self.mask.remove(i) {
                true => Some(self.inner.remove(i)),
                false => None,
            }
        }
    }

    pub fn iter(&self) -> SingleIter<'_, S> {
        self.into_iter()
    }

    pub fn iter_mut(&mut self) -> SingleIterMut<'_, S> {
        self.into_iter()
    }
}

impl<S: Storage> Drop for Masked<S> {
    fn drop(&mut self) {
        for i in (&self.mask).iter() {
            unsafe {
                self.inner.remove(i);
            }
        }
    }
}

pub struct StorageRefMut<'a, S> {
    guard: MutexGuard<'a, Box<dyn AbstractStorage>>,
    marker: PhantomData<S>,
}

impl<'a, S> StorageRefMut<'a, S> {
    pub(crate) fn new(guard: MutexGuard<'a, Box<dyn AbstractStorage>>) -> Self {
        Self {
            guard,
            marker: PhantomData,
        }
    }
}

impl<'a, S: Storage> Deref for StorageRefMut<'a, S> {
    type Target = Masked<S>;
    fn deref(&self) -> &Masked<S> {
        (**self.guard).downcast_ref::<Masked<S>>().unwrap()
    }
}

impl<'a, S: Storage> DerefMut for StorageRefMut<'a, S> {
    fn deref_mut(&mut self) -> &mut Masked<S> {
        self.guard.downcast_mut::<Masked<S>>().unwrap()
    }
}

impl<'a, S: Storage> IntoIterator for &'a Masked<S> {
    type Item = &'a S::Component;
    type IntoIter = SingleIter<'a, S>;

    fn into_iter(self) -> SingleIter<'a, S> {
        SingleIter {
            bits: (&self.mask).iter(),
            storage: &self.inner,
        }
    }
}

pub struct SingleIter<'a, S> {
    bits: BitIter<&'a BitSet>,
    storage: &'a S,
}

impl<'a, S: Storage> Iterator for SingleIter<'a, S> {
    type Item = &'a S::Component;
    fn next(&mut self) -> Option<Self::Item> {
        let i = self.bits.next()?;
        unsafe { Some(self.storage.get(i)) }
    }
}

impl<'a, S: Storage> IntoIterator for &'a mut Masked<S> {
    type Item = &'a mut S::Component;
    type IntoIter = SingleIterMut<'a, S>;

    fn into_iter(self) -> SingleIterMut<'a, S> {
        SingleIterMut {
            bits: (&self.mask).iter(),
            storage: &mut self.inner,
        }
    }
}

pub struct SingleIterMut<'a, S> {
    bits: BitIter<&'a BitSet>,
    storage: &'a mut S,
}

impl<'a, S: Storage> Iterator for SingleIterMut<'a, S> {
    type Item = &'a mut S::Component;
    fn next(&mut self) -> Option<Self::Item> {
        let i = self.bits.next()?;
        unsafe { Some(mem::transmute::<&mut S, &'a mut S>(self.storage).get_mut(i)) }
    }
}
