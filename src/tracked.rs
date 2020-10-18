use core::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

use crate::{Access, Archetype, Component, Fetch, Query};

/// Query that tracks mutable access to a component
///
/// Using this in a query is equivalent to `(&mut T, &mut Modified<T>)`, except that it yields a
/// smart pointer to `T` which sets the flag inside `Modified<T>` to `true` when it's mutably
/// borrowed.
///
/// A `Modified<T>` component must exist on an entity for it to be exposed to this query.
///
/// # Example
/// ```
/// # use hecs::*;
/// let mut world = World::new();
/// let e = world.spawn((123, Modified::<i32>::new()));
/// for (_id, mut value) in world.query::<Tracked<i32>>().iter() {
///   assert_eq!(*value, 123);
/// }
/// assert!(!world.get::<Modified<i32>>(e).unwrap().is_set());
/// for (_id, mut value) in world.query::<Tracked<i32>>().iter() {
///   *value = 42;
/// }
/// assert!(world.get::<Modified<i32>>(e).unwrap().is_set());
/// ```
pub struct Tracked<'a, T: Component> {
    value: &'a mut T,
    modified: &'a mut Modified<T>,
}

impl<'a, T: Component> Deref for Tracked<'a, T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T {
        self.value
    }
}

impl<'a, T: Component> DerefMut for Tracked<'a, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        self.modified.0 = true;
        self.value
    }
}

impl<'a, T: Component> Query for Tracked<'a, T> {
    type Fetch = FetchTracked<T>;
}

/// A flag indicating whether the `T` component was modified
///
/// Must be manually added to components that will be queried with `Tracked`.
pub struct Modified<T>(bool, PhantomData<T>);

impl<T> Modified<T> {
    /// Constructs an unset flag
    #[inline]
    pub fn new() -> Self {
        Self(false, PhantomData)
    }

    /// Returns whether the `T` component was modified since the last `unset` call
    #[inline]
    pub fn is_set(&self) -> bool {
        self.0
    }

    /// Unsets the flag
    #[inline]
    pub fn unset(&mut self) {
        self.0 = false;
    }
}

impl<T> Default for Modified<T> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[doc(hidden)]
pub struct FetchTracked<T: Component> {
    value: <&'static mut T as Query>::Fetch,
    modified: <&'static mut Modified<T> as Query>::Fetch,
}

impl<'a, T: Component> Fetch<'a> for FetchTracked<T> {
    type Item = Tracked<'a, T>;

    const DANGLING: Self = Self {
        value: <&'static mut T as Query>::Fetch::DANGLING,
        modified: <&'static mut Modified<T> as Query>::Fetch::DANGLING,
    };

    fn access(archetype: &Archetype) -> Option<Access> {
        Some(
            <&'a mut T as Query>::Fetch::access(archetype)?
                .max(<&'a mut Modified<T> as Query>::Fetch::access(archetype)?),
        )
    }

    fn borrow(archetype: &Archetype) {
        <&'a mut T as Query>::Fetch::borrow(archetype);
        <&'a mut Modified<T> as Query>::Fetch::borrow(archetype);
    }

    fn new(archetype: &'a Archetype) -> Option<Self> {
        Some(Self {
            value: <&'a mut T as Query>::Fetch::new(archetype)?,
            modified: <&'a mut Modified<T> as Query>::Fetch::new(archetype)?,
        })
    }

    fn release(archetype: &Archetype) {
        <&'a mut T as Query>::Fetch::release(archetype);
        <&'a mut Modified<T> as Query>::Fetch::release(archetype);
    }

    unsafe fn get(&self, n: usize) -> Self::Item {
        Tracked {
            value: self.value.get(n),
            modified: self.modified.get(n),
        }
    }
}
