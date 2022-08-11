// Copyright 2019 Google LLC
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use crate::alloc::vec::Vec;
use core::any::{type_name, TypeId};
use core::ptr::NonNull;
use core::{fmt, mem};

use crate::archetype::TypeInfo;
use crate::Component;

/// A dynamically typed collection of components
///
/// Bundles composed of exactly the same types are semantically equivalent, regardless of order. The
/// interface of this trait is a private implementation detail.
#[allow(clippy::missing_safety_doc)]
pub unsafe trait DynamicBundle {
    /// Returns a `TypeId` uniquely identifying the set of components, if known
    #[doc(hidden)]
    fn key(&self) -> Option<TypeId> {
        None
    }

    /// Invoke a callback on the fields' type IDs, sorted by descending alignment then id
    #[doc(hidden)]
    fn with_ids<T>(&self, f: impl FnOnce(&[TypeId]) -> T) -> T;

    /// Obtain the fields' TypeInfos, sorted by descending alignment then id
    #[doc(hidden)]
    fn type_info(&self) -> Vec<TypeInfo>;
    /// Allow a callback to move all components out of the bundle
    ///
    /// Must invoke `f` only with a valid pointer and the pointee's type and size.
    #[doc(hidden)]
    unsafe fn put(self, f: impl FnMut(*mut u8, TypeInfo));
}

/// A statically typed collection of components
///
/// Bundles composed of exactly the same types are semantically equivalent, regardless of order. The
/// interface of this trait is a private implementation detail.
#[allow(clippy::missing_safety_doc)]
pub unsafe trait Bundle: DynamicBundle {
    #[doc(hidden)]
    fn with_static_ids<T>(f: impl FnOnce(&[TypeId]) -> T) -> T;

    /// Obtain the fields' TypeInfos, sorted by descending alignment then id
    #[doc(hidden)]
    fn with_static_type_info<T>(f: impl FnOnce(&[TypeInfo]) -> T) -> T;

    /// Construct `Self` by moving components out of pointers fetched by `f`
    ///
    /// # Safety
    ///
    /// `f` must produce pointers to the expected fields. The implementation must not read from any
    /// pointers if any call to `f` returns `None`.
    #[doc(hidden)]
    unsafe fn get(f: impl FnMut(TypeInfo) -> Option<NonNull<u8>>) -> Result<Self, MissingComponent>
    where
        Self: Sized;
}

/// A dynamically typed collection of cloneable components
#[allow(clippy::missing_safety_doc)]
pub unsafe trait DynamicBundleClone: DynamicBundle {
    /// Allow a callback to move all components out of the bundle
    ///
    /// Must invoke `f` only with a valid pointer, the pointee's type and size, and a `DynamicClone`
    /// constructed for the pointee's type.
    #[doc(hidden)]
    unsafe fn put_with_clone(self, f: impl FnMut(*mut u8, TypeInfo, DynamicClone));
}

#[derive(Copy, Clone)]
/// Type-erased [`Clone`] implementation
pub struct DynamicClone {
    pub(crate) func: unsafe fn(*const u8, &mut dyn FnMut(*mut u8, TypeInfo)),
}

impl DynamicClone {
    /// Create a new type ereased cloner for the type T
    pub fn new<T: Component + Clone>() -> Self {
        Self {
            func: |src, f| {
                let mut tmp = unsafe { (*src.cast::<T>()).clone() };
                f((&mut tmp as *mut T).cast(), TypeInfo::of::<T>());
                core::mem::forget(tmp);
            },
        }
    }
}

/// Error indicating that an entity did not have a required component
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct MissingComponent(&'static str);

impl MissingComponent {
    /// Construct an error representing a missing `T`
    pub fn new<T: Component>() -> Self {
        Self(type_name::<T>())
    }
}

impl fmt::Display for MissingComponent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "missing {} component", self.0)
    }
}

#[cfg(feature = "std")]
impl std::error::Error for MissingComponent {}

macro_rules! tuple_impl {
    ($($name: ident),*) => {
        unsafe impl<$($name: Component),*> DynamicBundle for ($($name,)*) {
            fn key(&self) -> Option<TypeId> {
                Some(TypeId::of::<Self>())
            }

            fn with_ids<T>(&self, f: impl FnOnce(&[TypeId]) -> T) -> T {
                Self::with_static_ids(f)
            }

            fn type_info(&self) -> Vec<TypeInfo> {
                Self::with_static_type_info(|info| info.to_vec())
            }

            #[allow(unused_variables, unused_mut)]
            unsafe fn put(self, mut f: impl FnMut(*mut u8, TypeInfo)) {
                #[allow(non_snake_case)]
                let ($(mut $name,)*) = self;
                $(
                    f(
                        (&mut $name as *mut $name).cast::<u8>(),
                        TypeInfo::of::<$name>()
                    );
                    mem::forget($name);
                )*
            }
        }

        unsafe impl<$($name: Component + Clone),*> DynamicBundleClone for ($($name,)*) {
            // Compiler false positive warnings
            #[allow(unused_variables, unused_mut)]
            unsafe fn put_with_clone(self, mut f: impl FnMut(*mut u8, TypeInfo, DynamicClone)) {
                #[allow(non_snake_case)]
                let ($(mut $name,)*) = self;
                $(
                    f(
                        (&mut $name as *mut $name).cast::<u8>(),
                        TypeInfo::of::<$name>(),
                        DynamicClone::new::<$name>()
                    );
                    mem::forget($name);
                )*
            }
        }

        unsafe impl<$($name: Component),*> Bundle for ($($name,)*) {
            fn with_static_ids<T>(f: impl FnOnce(&[TypeId]) -> T) -> T {
                const N: usize = count!($($name),*);
                let mut xs: [(usize, TypeId); N] = [$((mem::align_of::<$name>(), TypeId::of::<$name>())),*];
                xs.sort_unstable_by(|x, y| x.0.cmp(&y.0).reverse().then(x.1.cmp(&y.1)));
                let mut ids = [TypeId::of::<()>(); N];
                for (slot, &(_, id)) in ids.iter_mut().zip(xs.iter()) {
                    *slot = id;
                }
                f(&ids)
            }

            fn with_static_type_info<T>(f: impl FnOnce(&[TypeInfo]) -> T) -> T {
                const N: usize = count!($($name),*);
                let mut xs: [TypeInfo; N] = [$(TypeInfo::of::<$name>()),*];
                xs.sort_unstable();
                f(&xs)
            }

            #[allow(unused_variables, unused_mut)]
            unsafe fn get(mut f: impl FnMut(TypeInfo) -> Option<NonNull<u8>>) -> Result<Self, MissingComponent> {
                #[allow(non_snake_case)]
                let ($(mut $name,)*) = ($(
                    f(TypeInfo::of::<$name>()).ok_or_else(MissingComponent::new::<$name>)?
                        .as_ptr()
                        .cast::<$name>(),)*
                );
                Ok(($($name.read(),)*))
            }
        }
    }
}

macro_rules! count {
    () => { 0 };
    ($x: ident $(, $rest: ident)*) => { 1 + count!($($rest),*) };
}

smaller_tuples_too!(
    tuple_impl, AA, BB, CC, DD, EE, FF, GG, HH, II, JJ, KK, LL, MM, NN, OO, PP, QQ, RR, SS, TT, UU, VV, WW, XX, YY, ZZ, Z, Y, X, W, V, U, S, R, Q, P, O, N, M, L, K, J, I, H, G, F, E, D, C, B, A
);
