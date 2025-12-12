use core::any::TypeId;
use core::fmt::{self, Debug, Display, Formatter};
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut, FnOnce};
use core::ptr::NonNull;

use crate::archetype::Archetype;
use crate::entities::EntityMeta;
use crate::{
    ArchetypeColumn, ArchetypeColumnMut, Component, Entity, Fetch, MissingComponent, Query,
    QueryOne,
};

/// Handle to an entity with any component types
#[derive(Copy, Clone)]
pub struct EntityRef<'a> {
    meta: &'a [EntityMeta],
    archetype: &'a Archetype,
    /// Position of this entity in `archetype`
    index: u32,
}

impl<'a> EntityRef<'a> {
    pub(crate) unsafe fn new(meta: &'a [EntityMeta], archetype: &'a Archetype, index: u32) -> Self {
        Self {
            meta,
            archetype,
            index,
        }
    }

    /// Get the [`Entity`] handle associated with this entity
    #[inline]
    pub fn entity(&self) -> Entity {
        let id = self.archetype.entity_id(self.index);
        Entity {
            id,
            generation: self.meta[id as usize].generation,
        }
    }

    /// Determine whether this entity would satisfy the query `Q` without borrowing any components
    pub fn satisfies<Q: Query>(&self) -> bool {
        Q::Fetch::access(self.archetype).is_some()
    }

    /// Determine whether this entity has a `T` component without borrowing it
    ///
    /// Equivalent to [`satisfies::<&T>`](Self::satisfies)
    pub fn has<T: Component>(&self) -> bool {
        self.archetype.has::<T>()
    }

    /// Borrow a single component, if it exists
    ///
    /// `T` must be a shared or unique reference to a component type.
    ///
    /// # Example
    /// ```
    /// # use hecs::*;
    /// let mut world = World::new();
    /// let a = world.spawn((42, "abc"));
    /// let e = world.entity(a).unwrap();
    /// *e.get::<&mut i32>().unwrap() = 17;
    /// assert_eq!(*e.get::<&i32>().unwrap(), 17);
    /// ```
    ///
    /// Panics if `T` is a unique reference and the component is already borrowed, or if the
    /// component is already uniquely borrowed.
    pub fn get<T: ComponentRef<'a>>(&self) -> Option<T::Ref> {
        T::get_component(*self)
    }

    /// Run a query against this entity
    ///
    /// Equivalent to invoking [`World::query_one`](crate::World::query_one) on the entity. May
    /// outlive `self`.
    ///
    /// # Example
    /// ```
    /// # use hecs::*;
    /// let mut world = World::new();
    /// let a = world.spawn((123, true, "abc"));
    /// // The returned query must outlive the borrow made by `get`
    /// let mut query = world.entity(a).unwrap().query::<(&mut i32, &bool)>();
    /// let (number, flag) = query.get().unwrap();
    /// if *flag { *number *= 2; }
    /// assert_eq!(*number, 246);
    /// ```
    pub fn query<Q: Query>(&self) -> QueryOne<'a, Q> {
        unsafe { QueryOne::new(self.meta, self.archetype, self.index) }
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

unsafe impl Send for EntityRef<'_> {}
unsafe impl Sync for EntityRef<'_> {}

/// Shared borrow of an entity's component
pub struct Ref<'a, T: ?Sized> {
    borrow: ComponentBorrow<'a>,
    target: NonNull<T>,
    _phantom: PhantomData<&'a T>,
}

impl<'a, T: Component> Ref<'a, T> {
    pub(crate) unsafe fn new(
        archetype: &'a Archetype,
        index: u32,
    ) -> Result<Self, MissingComponent> {
        let (target, borrow) = ComponentBorrow::for_component::<T>(archetype, index)?;
        Ok(Self {
            borrow,
            target,
            _phantom: PhantomData,
        })
    }
}

unsafe impl<T: ?Sized + Sync> Send for Ref<'_, T> {}
unsafe impl<T: ?Sized + Sync> Sync for Ref<'_, T> {}

impl<'a, T: ?Sized> Ref<'a, T> {
    /// Transform the `Ref<'_, T>` to point to a part of the borrowed data, e.g.
    /// a struct field.
    ///
    /// The `Ref<'_, T>` is already borrowed, so this cannot fail.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use hecs::{EntityRef, Ref};
    /// struct Component {
    ///     member: i32,
    /// }
    ///
    /// # fn example(entity_ref: EntityRef<'_>) {
    /// let component_ref = entity_ref.get::<&Component>()
    ///     .expect("Entity does not contain an instance of \"Component\"");
    /// let member_ref = Ref::map(component_ref, |component| &component.member);
    /// println!("member = {:?}", *member_ref);
    /// # }
    /// ```
    pub fn map<U: ?Sized, F>(orig: Ref<'a, T>, f: F) -> Ref<'a, U>
    where
        F: FnOnce(&T) -> &U,
    {
        let target = NonNull::from(f(&*orig));
        Ref {
            borrow: orig.borrow,
            target,
            _phantom: PhantomData,
        }
    }
}

impl<T: ?Sized> Deref for Ref<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { self.target.as_ref() }
    }
}

impl<T: ?Sized + Debug> Debug for Ref<'_, T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Debug::fmt(self.deref(), f)
    }
}

impl<T: ?Sized + Display> Display for Ref<'_, T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(self.deref(), f)
    }
}

impl<T: ?Sized> Clone for Ref<'_, T> {
    fn clone(&self) -> Self {
        Self {
            borrow: self.borrow.clone(),
            target: self.target,
            _phantom: self._phantom,
        }
    }
}

/// Unique borrow of an entity's component
pub struct RefMut<'a, T: ?Sized> {
    borrow: ComponentBorrowMut<'a>,
    target: NonNull<T>,
    _phantom: PhantomData<&'a mut T>,
}

impl<'a, T: Component> RefMut<'a, T> {
    pub(crate) unsafe fn new(
        archetype: &'a Archetype,
        index: u32,
    ) -> Result<Self, MissingComponent> {
        let (target, borrow) = ComponentBorrowMut::for_component::<T>(archetype, index)?;
        Ok(Self {
            borrow,
            target,
            _phantom: PhantomData,
        })
    }
}

unsafe impl<T: ?Sized + Send> Send for RefMut<'_, T> {}
unsafe impl<T: ?Sized + Sync> Sync for RefMut<'_, T> {}

impl<'a, T: ?Sized> RefMut<'a, T> {
    /// Transform the `RefMut<'_, T>` to point to a part of the borrowed data, e.g.
    /// a struct field.
    ///
    /// The `RefMut<'_, T>` is already mutably borrowed, so this cannot fail.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use hecs::{EntityRef, RefMut};
    /// struct Component {
    ///     member: i32,
    /// }
    ///
    /// # fn example(entity_ref: EntityRef<'_>) {
    /// let component_ref = entity_ref.get::<&mut Component>()
    ///     .expect("Entity does not contain an instance of \"Component\"");
    /// let mut member_ref = RefMut::map(component_ref, |component| &mut component.member);
    /// *member_ref = 21;
    /// println!("member = {:?}", *member_ref);
    /// # }
    /// ```
    pub fn map<U: ?Sized, F>(mut orig: RefMut<'a, T>, f: F) -> RefMut<'a, U>
    where
        F: FnOnce(&mut T) -> &mut U,
    {
        let target = NonNull::from(f(&mut *orig));
        RefMut {
            borrow: orig.borrow,
            target,
            _phantom: PhantomData,
        }
    }
}

impl<T: ?Sized> Deref for RefMut<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { self.target.as_ref() }
    }
}

impl<T: ?Sized> DerefMut for RefMut<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { self.target.as_mut() }
    }
}

impl<T: ?Sized + Debug> Debug for RefMut<'_, T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Debug::fmt(self.deref(), f)
    }
}

impl<T: ?Sized + Display> Display for RefMut<'_, T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(self.deref(), f)
    }
}

/// `&T` or `&mut T` where `T` is some component type
///
/// The interface of this trait is a private implementation detail.
pub trait ComponentRef<'a> {
    /// Smart pointer to a component of the referenced type
    #[doc(hidden)]
    type Ref;

    /// Smart pointer to a column of the referenced type in an [`Archetype`](crate::Archetype)
    #[doc(hidden)]
    type Column;

    /// Component type referenced by `Ref`
    #[doc(hidden)]
    type Component: Component;

    /// Fetch the component from `entity`
    #[doc(hidden)]
    fn get_component(entity: EntityRef<'a>) -> Option<Self::Ref>;

    /// Construct from a raw pointer
    ///
    /// # Safety
    ///
    /// Dereferencing `raw` for lifetime `'a` must be sound
    #[doc(hidden)]
    unsafe fn from_raw(raw: *mut Self::Component) -> Self;

    /// Borrow a column from an archetype
    #[doc(hidden)]
    fn get_column(archetype: &'a Archetype) -> Option<Self::Column>;
}

impl<'a, T: Component> ComponentRef<'a> for &'a T {
    type Ref = Ref<'a, T>;

    type Column = ArchetypeColumn<'a, T>;

    type Component = T;

    fn get_component(entity: EntityRef<'a>) -> Option<Self::Ref> {
        Some(unsafe { Ref::new(entity.archetype, entity.index).ok()? })
    }

    unsafe fn from_raw(raw: *mut Self::Component) -> Self {
        &*raw
    }

    fn get_column(archetype: &'a Archetype) -> Option<Self::Column> {
        ArchetypeColumn::new(archetype)
    }
}

impl<'a, T: Component> ComponentRef<'a> for &'a mut T {
    type Ref = RefMut<'a, T>;

    type Column = ArchetypeColumnMut<'a, T>;

    type Component = T;

    fn get_component(entity: EntityRef<'a>) -> Option<Self::Ref> {
        Some(unsafe { RefMut::new(entity.archetype, entity.index).ok()? })
    }

    unsafe fn from_raw(raw: *mut Self::Component) -> Self {
        &mut *raw
    }

    fn get_column(archetype: &'a Archetype) -> Option<Self::Column> {
        ArchetypeColumnMut::new(archetype)
    }
}

/// `&T` where `T` is some component type
///
/// Used when consistency demands that references to component types, rather than component types
/// themselves, be supplied as a type parameter to a function that cannot operate on unique
/// references.
pub trait ComponentRefShared<'a>: ComponentRef<'a> {}

impl<'a, T: Component> ComponentRefShared<'a> for &'a T {}

struct ComponentBorrow<'a> {
    archetype: &'a Archetype,
    /// State index for the borrowed component in the `archetype`.
    state: usize,
}

impl<'a> ComponentBorrow<'a> {
    // This method is unsafe as if the `index` is out of bounds,
    // then this will cause undefined behavior as the returned
    // `target` will point to undefined memory.
    unsafe fn for_component<T: Component>(
        archetype: &'a Archetype,
        index: u32,
    ) -> Result<(NonNull<T>, Self), MissingComponent> {
        let state = archetype
            .get_state::<T>()
            .ok_or_else(MissingComponent::new::<T>)?;

        let target =
            NonNull::new_unchecked(archetype.get_base::<T>(state).as_ptr().add(index as usize));

        archetype.borrow::<T>(state);

        Ok((target, Self { archetype, state }))
    }
}

impl Clone for ComponentBorrow<'_> {
    fn clone(&self) -> Self {
        unsafe {
            self.archetype.borrow_raw(self.state);
        }
        Self {
            archetype: self.archetype,
            state: self.state,
        }
    }
}

impl Drop for ComponentBorrow<'_> {
    fn drop(&mut self) {
        unsafe {
            self.archetype.release_raw(self.state);
        }
    }
}

struct ComponentBorrowMut<'a> {
    archetype: &'a Archetype,
    /// State index for the borrowed component in the `archetype`.
    state: usize,
}

impl<'a> ComponentBorrowMut<'a> {
    // This method is unsafe as if the `index` is out of bounds,
    // then this will cause undefined behavior as the returned
    // `target` will point to undefined memory.
    unsafe fn for_component<T: Component>(
        archetype: &'a Archetype,
        index: u32,
    ) -> Result<(NonNull<T>, Self), MissingComponent> {
        let state = archetype
            .get_state::<T>()
            .ok_or_else(MissingComponent::new::<T>)?;

        let target =
            NonNull::new_unchecked(archetype.get_base::<T>(state).as_ptr().add(index as usize));

        archetype.borrow_mut::<T>(state);

        Ok((target, Self { archetype, state }))
    }
}

impl Drop for ComponentBorrowMut<'_> {
    fn drop(&mut self) {
        unsafe {
            self.archetype.release_raw_mut(self.state);
        }
    }
}
