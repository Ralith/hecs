// Copyright 2019 Google LLC
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use crate::alloc::alloc::{alloc, dealloc, Layout};
use crate::alloc::boxed::Box;
use crate::alloc::{vec, vec::Vec};
use crate::cloning::TypeUnknownToCloner;
use core::any::{type_name, Any, TypeId};
use core::fmt;
use core::hash::{BuildHasher, BuildHasherDefault, Hasher};
use core::ops::{Deref, DerefMut};
use core::ptr::{self, NonNull};

use hashbrown::{hash_map::DefaultHashBuilder, HashMap};

use crate::borrow::AtomicBorrow;
use crate::query::Fetch;
use crate::{Access, Component, ComponentRef, Query};

/// A collection of entities having the same component types
///
/// Accessing `Archetype`s is only required in niche cases. Typical use should go through the
/// [`World`](crate::World).
pub struct Archetype {
    types: Vec<TypeInfo>,
    type_ids: Box<[TypeId]>,
    index: OrderedTypeIdMap<usize>,
    len: u32,
    entities: Box<[u32]>,
    /// One allocation per type, in the same order as `types`
    data: Box<[Data]>,
}

impl Archetype {
    fn assert_type_info(types: &[TypeInfo]) {
        types.windows(2).for_each(|x| match x[0].cmp(&x[1]) {
            core::cmp::Ordering::Less => (),
            #[cfg(debug_assertions)]
            core::cmp::Ordering::Equal => panic!(
                "attempted to allocate entity with duplicate {} components; \
                 each type must occur at most once!",
                x[0].type_name
            ),
            #[cfg(not(debug_assertions))]
            core::cmp::Ordering::Equal => panic!(
                "attempted to allocate entity with duplicate components; \
                 each type must occur at most once!"
            ),
            core::cmp::Ordering::Greater => panic!("type info is unsorted"),
        });
    }

    pub(crate) fn new(types: Vec<TypeInfo>) -> Self {
        let max_align = types.first().map_or(1, |ty| ty.layout.align());
        Self::assert_type_info(&types);
        let component_count = types.len();
        Self {
            index: OrderedTypeIdMap::new(types.iter().enumerate().map(|(i, ty)| (ty.id, i))),
            type_ids: types.iter().map(|ty| ty.id()).collect(),
            types,
            entities: Box::new([]),
            len: 0,
            data: (0..component_count)
                .map(|_| Data {
                    state: AtomicBorrow::new(),
                    storage: NonNull::new(max_align as *mut u8).unwrap(),
                })
                .collect(),
        }
    }

    pub(crate) fn clear(&mut self) {
        for (ty, data) in self.types.iter().zip(&*self.data) {
            for index in 0..self.len {
                unsafe {
                    let removed = data.storage.as_ptr().add(index as usize * ty.layout.size());
                    (ty.drop)(removed);
                }
            }
        }
        self.len = 0;
    }

    /// Whether this archetype contains `T` components
    pub fn has<T: Component>(&self) -> bool {
        self.has_dynamic(TypeId::of::<T>())
    }

    /// Whether this archetype contains components with the type identified by `id`
    pub fn has_dynamic(&self, id: TypeId) -> bool {
        self.index.contains_key(&id)
    }

    /// Find the state index associated with `T`, if present
    pub(crate) fn get_state<T: Component>(&self) -> Option<usize> {
        self.index.get(&TypeId::of::<T>()).copied()
    }

    /// Get the address of the first `T` component using an index from `get_state::<T>`
    pub(crate) fn get_base<T: Component>(&self, state: usize) -> NonNull<T> {
        assert_eq!(self.types[state].id, TypeId::of::<T>());

        unsafe {
            NonNull::new_unchecked(self.data.get_unchecked(state).storage.as_ptr().cast::<T>())
        }
    }

    /// Borrow all components of a single type from these entities, if present
    ///
    /// `T` must be a shared or unique reference to a component type.
    ///
    /// Useful for efficient serialization.
    pub fn get<'a, T: ComponentRef<'a>>(&'a self) -> Option<T::Column> {
        T::get_column(self)
    }

    pub(crate) fn borrow<T: Component>(&self, state: usize) {
        assert_eq!(self.types[state].id, TypeId::of::<T>());

        if !self.data[state].state.borrow() {
            panic!("{} already borrowed uniquely", type_name::<T>());
        }
    }

    pub(crate) unsafe fn borrow_raw(&self, state: usize) {
        if !self.data[state].state.borrow() {
            panic!("state index {} already borrowed uniquely", state);
        }
    }

    pub(crate) fn borrow_mut<T: Component>(&self, state: usize) {
        assert_eq!(self.types[state].id, TypeId::of::<T>());

        if !self.data[state].state.borrow_mut() {
            panic!("{} already borrowed", type_name::<T>());
        }
    }

    pub(crate) fn release<T: Component>(&self, state: usize) {
        assert_eq!(self.types[state].id, TypeId::of::<T>());
        self.data[state].state.release();
    }

    pub(crate) fn release_mut<T: Component>(&self, state: usize) {
        assert_eq!(self.types[state].id, TypeId::of::<T>());
        self.data[state].state.release_mut();
    }

    pub(crate) unsafe fn release_raw(&self, state: usize) {
        self.data[state].state.release();
    }

    pub(crate) unsafe fn release_raw_mut(&self, state: usize) {
        self.data[state].state.release_mut();
    }

    /// Number of entities in this archetype
    #[inline]
    pub fn len(&self) -> u32 {
        self.len
    }

    /// Whether this archetype contains no entities
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub(crate) fn entities(&self) -> NonNull<u32> {
        unsafe { NonNull::new_unchecked(self.entities.as_ptr() as *mut _) }
    }

    pub(crate) fn entity_id(&self, index: u32) -> u32 {
        self.entities[index as usize]
    }

    #[inline]
    pub(crate) fn set_entity_id(&mut self, index: usize, id: u32) {
        self.entities[index] = id;
    }

    pub(crate) fn types(&self) -> &[TypeInfo] {
        &self.types
    }

    pub(crate) fn type_ids(&self) -> &[TypeId] {
        &self.type_ids
    }

    /// Enumerate the types of the components of entities stored in this archetype.
    ///
    /// Convenient for dispatching logic which needs to be performed on sets of type ids.  For
    /// example, suppose you're building a scripting system, and you want to integrate the scripting
    /// language with your ECS. This functionality allows you to iterate through all of the
    /// archetypes of the world with [`World::archetypes()`](crate::World::archetypes()) and extract
    /// all possible combinations of component types which are currently stored in the `World`.
    /// From there, you can then create a mapping of archetypes to wrapper objects for your
    /// scripting language that provide functionality based off of the components of any given
    /// [`Entity`], and bind them onto an [`Entity`] when passed into your scripting language by
    /// looking up the [`Entity`]'s archetype using
    /// [`EntityRef::component_types`](crate::EntityRef::component_types).
    ///
    /// [`Entity`]: crate::Entity
    pub fn component_types(&self) -> impl ExactSizeIterator<Item = TypeId> + '_ {
        self.types.iter().map(|typeinfo| typeinfo.id)
    }

    /// `index` must be in-bounds or just past the end
    pub(crate) unsafe fn get_dynamic(
        &self,
        ty: TypeId,
        size: usize,
        index: u32,
    ) -> Option<NonNull<u8>> {
        debug_assert!(index <= self.len);
        Some(NonNull::new_unchecked(
            self.data
                .get_unchecked(*self.index.get(&ty)?)
                .storage
                .as_ptr()
                .add(size * index as usize)
                .cast::<u8>(),
        ))
    }

    /// Every type must be written immediately after this call
    pub(crate) unsafe fn allocate(&mut self, id: u32) -> u32 {
        if self.len as usize == self.entities.len() {
            self.grow(64);
        }

        self.entities[self.len as usize] = id;
        self.len += 1;
        self.len - 1
    }

    pub(crate) unsafe fn set_len(&mut self, len: u32) {
        debug_assert!(len <= self.capacity());
        self.len = len;
    }

    pub(crate) fn reserve(&mut self, additional: u32) {
        if additional > (self.capacity() - self.len()) {
            let increment = additional - (self.capacity() - self.len());
            self.grow(increment.max(64));
        }
    }

    pub(crate) fn capacity(&self) -> u32 {
        self.entities.len() as u32
    }

    /// Increase capacity by at least `min_increment`
    fn grow(&mut self, min_increment: u32) {
        // Double capacity or increase it by `min_increment`, whichever is larger.
        self.grow_exact(self.capacity().max(min_increment))
    }

    /// Increase capacity by exactly `increment`
    fn grow_exact(&mut self, increment: u32) {
        let old_count = self.len as usize;
        let old_cap = self.entities.len();
        let new_cap = self.entities.len() + increment as usize;
        let mut new_entities = vec![!0; new_cap].into_boxed_slice();
        new_entities[0..old_count].copy_from_slice(&self.entities[0..old_count]);
        self.entities = new_entities;

        let new_data = self
            .types
            .iter()
            .zip(&*self.data)
            .map(|(info, old)| {
                let storage = if info.layout.size() == 0 {
                    NonNull::new(info.layout.align() as *mut u8).unwrap()
                } else {
                    let layout =
                        Layout::from_size_align(info.layout.size() * new_cap, info.layout.align())
                            .unwrap();
                    unsafe {
                        let mem = alloc(layout);
                        let mem = NonNull::new(mem)
                            .unwrap_or_else(|| alloc::alloc::handle_alloc_error(layout));
                        ptr::copy_nonoverlapping(
                            old.storage.as_ptr(),
                            mem.as_ptr(),
                            info.layout.size() * old_count,
                        );
                        mem
                    }
                };
                Data {
                    state: AtomicBorrow::new(), // &mut self guarantees no outstanding borrows
                    storage,
                }
            })
            .collect::<Box<[_]>>();

        // Now that we've successfully constructed a replacement, we can
        // deallocate the old column data without risking `self.data` being left
        // partially deallocated on OOM.
        if old_cap > 0 {
            for (info, data) in self.types.iter().zip(&*self.data) {
                if info.layout.size() == 0 {
                    continue;
                }
                unsafe {
                    dealloc(
                        data.storage.as_ptr(),
                        Layout::from_size_align(info.layout.size() * old_cap, info.layout.align())
                            .unwrap(),
                    );
                }
            }
        }

        self.data = new_data;
    }

    /// Returns the ID of the entity moved into `index`, if any
    pub(crate) unsafe fn remove(&mut self, index: u32, drop: bool) -> Option<u32> {
        let last = self.len - 1;
        for (ty, data) in self.types.iter().zip(&*self.data) {
            let removed = data.storage.as_ptr().add(index as usize * ty.layout.size());
            if drop {
                (ty.drop)(removed);
            }
            if index != last {
                let moved = data.storage.as_ptr().add(last as usize * ty.layout.size());
                ptr::copy_nonoverlapping(moved, removed, ty.layout.size());
            }
        }
        self.len = last;
        if index != last {
            self.entities[index as usize] = self.entities[last as usize];
            Some(self.entities[last as usize])
        } else {
            None
        }
    }

    /// Returns the ID of the entity moved into `index`, if any
    pub(crate) unsafe fn move_to(
        &mut self,
        index: u32,
        mut f: impl FnMut(*mut u8, TypeId, usize),
    ) -> Option<u32> {
        let last = self.len - 1;
        for (ty, data) in self.types.iter().zip(&*self.data) {
            let moved_out = data.storage.as_ptr().add(index as usize * ty.layout.size());
            f(moved_out, ty.id(), ty.layout().size());
            if index != last {
                let moved = data.storage.as_ptr().add(last as usize * ty.layout.size());
                ptr::copy_nonoverlapping(moved, moved_out, ty.layout.size());
            }
        }
        self.len -= 1;
        if index != last {
            self.entities[index as usize] = self.entities[last as usize];
            Some(self.entities[last as usize])
        } else {
            None
        }
    }

    pub(crate) unsafe fn put_dynamic(
        &mut self,
        component: *mut u8,
        ty: TypeId,
        size: usize,
        index: u32,
    ) {
        let ptr = self
            .get_dynamic(ty, size, index)
            .unwrap()
            .as_ptr()
            .cast::<u8>();
        ptr::copy_nonoverlapping(component, ptr, size);
    }

    /// How, if at all, `Q` will access entities in this archetype
    pub fn access<Q: Query>(&self) -> Option<Access> {
        Q::Fetch::access(self)
    }

    /// Determine whether this archetype would satisfy the query `Q`
    pub fn satisfies<Q: Query>(&self) -> bool {
        self.access::<Q>().is_some()
    }

    /// Add components from another archetype with identical components
    ///
    /// # Safety
    ///
    /// Component types must match exactly.
    pub(crate) unsafe fn merge(&mut self, mut other: Archetype) {
        self.reserve(other.len);
        for ((info, dst), src) in self.types.iter().zip(&*self.data).zip(&*other.data) {
            dst.storage
                .as_ptr()
                .add(self.len as usize * info.layout.size())
                .copy_from_nonoverlapping(
                    src.storage.as_ptr(),
                    other.len as usize * info.layout.size(),
                )
        }
        self.len += other.len;
        other.len = 0;
    }

    /// Raw IDs of the entities in this archetype
    ///
    /// Convertible into [`Entity`](crate::Entity)s with
    /// [`World::find_entity_from_id()`](crate::World::find_entity_from_id). Useful for efficient
    /// serialization.
    #[inline]
    pub fn ids(&self) -> &[u32] {
        &self.entities[0..self.len as usize]
    }

    pub(crate) fn try_clone(
        &mut self,
        cloner: &crate::cloning::Cloner,
    ) -> Result<Self, TypeUnknownToCloner> {
        // Make sure all sized component types are known, so we don't leak memory if we fail halfway
        if let Some(unsupported_type) = self
            .types
            .iter()
            .filter(|info| info.layout.size() != 0)
            .find(|info| cloner.typeid_to_clone_fn.get(&info.id).is_none())
        {
            return Err(TypeUnknownToCloner {
                #[cfg(debug_assertions)]
                type_name: unsupported_type.type_name,
                type_id: unsupported_type.type_id(),
            });
        }

        // Allocate the new data block with the same number of entities & same component types
        let new_data: Box<[Data]> = self
            .types
            .iter()
            .zip(&*self.data)
            .map(|(info, old)| {
                let storage: NonNull<u8> = if info.layout.size() == 0 {
                    // This is a zero-sized type, so no need to allocate or copy any data
                    NonNull::new(info.layout.align() as *mut u8).unwrap()
                } else {
                    // Allocate memory for the cloned data
                    let layout = Layout::from_size_align(
                        info.layout.size() * self.capacity() as usize,
                        info.layout.align(),
                    )
                    .unwrap();
                    let new_storage = {
                        let mem = unsafe { alloc(layout) };
                        NonNull::new(mem)
                            .unwrap_or_else(|| alloc::alloc::handle_alloc_error(layout))
                    };

                    // Clone the data for all instances of the component
                    let clone_fn = cloner.typeid_to_clone_fn.get(&info.id).expect(
                        "all types should be supported by cloner (because we checked earlier)",
                    );
                    unsafe {
                        clone_fn.call(
                            old.storage.as_ptr(),
                            new_storage.as_ptr(),
                            self.len as usize,
                        )
                    };

                    new_storage
                };
                Ok(Data {
                    state: AtomicBorrow::new(), // &mut self guarantees no outstanding borrows
                    storage,
                })
            })
            .collect::<Result<_, _>>()?;

        // Clone the other fields of the archetype and return the new instance
        Ok(Self {
            types: self.types.clone(),
            type_ids: self.type_ids.clone(),
            index: self.index.clone(),
            len: self.len,
            entities: self.entities.clone(),
            data: new_data,
        })
    }
}

impl Drop for Archetype {
    fn drop(&mut self) {
        self.clear();
        if self.entities.len() == 0 {
            return;
        }
        for (info, data) in self.types.iter().zip(&*self.data) {
            if info.layout.size() != 0 {
                unsafe {
                    dealloc(
                        data.storage.as_ptr(),
                        Layout::from_size_align_unchecked(
                            info.layout.size() * self.entities.len(),
                            info.layout.align(),
                        ),
                    );
                }
            }
        }
    }
}

struct Data {
    state: AtomicBorrow,
    storage: NonNull<u8>,
}

/// A hasher optimized for hashing a single TypeId.
///
/// TypeId is already thoroughly hashed, so there's no reason to hash it again.
/// Just leave the bits unchanged.
#[derive(Default)]
pub(crate) struct TypeIdHasher {
    hash: u64,
}

impl Hasher for TypeIdHasher {
    fn write_u64(&mut self, n: u64) {
        // Only a single value can be hashed, so the old hash should be zero.
        debug_assert_eq!(self.hash, 0);
        self.hash = n;
    }

    // Tolerate TypeId being either u64 or u128.
    fn write_u128(&mut self, n: u128) {
        debug_assert_eq!(self.hash, 0);
        self.hash = n as u64;
    }

    fn write(&mut self, bytes: &[u8]) {
        debug_assert_eq!(self.hash, 0);

        // This will only be called if TypeId is neither u64 nor u128, which is not anticipated.
        // In that case we'll just fall back to using a different hash implementation.
        let mut hasher = <DefaultHashBuilder as BuildHasher>::Hasher::default();
        hasher.write(bytes);
        self.hash = hasher.finish();
    }

    fn finish(&self) -> u64 {
        self.hash
    }
}

/// A HashMap with TypeId keys
///
/// Because TypeId is already a fully-hashed u64 (including data in the high seven bits,
/// which hashbrown needs), there is no need to hash it again. Instead, this uses the much
/// faster no-op hash.
pub(crate) type TypeIdMap<V> = HashMap<TypeId, V, BuildHasherDefault<TypeIdHasher>>;

#[derive(Clone)]
struct OrderedTypeIdMap<V>(Box<[(TypeId, V)]>);

impl<V> OrderedTypeIdMap<V> {
    fn new(iter: impl Iterator<Item = (TypeId, V)>) -> Self {
        let mut vals = iter.collect::<Box<[_]>>();
        vals.sort_unstable_by_key(|(id, _)| *id);
        Self(vals)
    }

    fn search(&self, id: &TypeId) -> Option<usize> {
        self.0.binary_search_by_key(id, |(id, _)| *id).ok()
    }

    fn contains_key(&self, id: &TypeId) -> bool {
        self.search(id).is_some()
    }

    fn get(&self, id: &TypeId) -> Option<&V> {
        self.search(id).map(move |idx| &self.0[idx].1)
    }
}

/// Metadata required to store a component.
///
/// All told, this means a [`TypeId`], to be able to dynamically name/check the component type; a
/// [`Layout`], so that we know how to allocate memory for this component type; and a drop function
/// which internally calls [`core::ptr::drop_in_place`] with the correct type parameter.
#[derive(Debug, Copy, Clone)]
pub struct TypeInfo {
    id: TypeId,
    layout: Layout,
    drop: unsafe fn(*mut u8),
    #[cfg(debug_assertions)]
    type_name: &'static str,
}

impl TypeInfo {
    /// Construct a `TypeInfo` directly from the static type.
    pub fn of<T: 'static>() -> Self {
        unsafe fn drop_ptr<T>(x: *mut u8) {
            x.cast::<T>().drop_in_place()
        }

        Self {
            id: TypeId::of::<T>(),
            layout: Layout::new::<T>(),
            drop: drop_ptr::<T>,
            #[cfg(debug_assertions)]
            type_name: core::any::type_name::<T>(),
        }
    }

    /// Construct a `TypeInfo` from its components. This is useful in the rare case that you have
    /// some kind of pointer to raw bytes/erased memory holding a component type, coming from a
    /// source unrelated to hecs, and you want to treat it as an insertable component by
    /// implementing the `DynamicBundle` API.
    pub fn from_parts(id: TypeId, layout: Layout, drop: unsafe fn(*mut u8)) -> Self {
        Self {
            id,
            layout,
            drop,
            #[cfg(debug_assertions)]
            type_name: "<unknown> (TypeInfo constructed from parts)",
        }
    }

    /// Access the `TypeId` for this component type.
    pub fn id(&self) -> TypeId {
        self.id
    }

    /// Access the `Layout` of this component type.
    pub fn layout(&self) -> Layout {
        self.layout
    }

    /// Directly call the destructor on a pointer to data of this component type.
    ///
    /// # Safety
    ///
    /// All of the caveats of [`core::ptr::drop_in_place`] apply, with the additional requirement
    /// that this method is being called on a pointer to an object of the correct component type.
    pub unsafe fn drop(&self, data: *mut u8) {
        (self.drop)(data)
    }

    /// Get the function pointer encoding the destructor for the component type this `TypeInfo`
    /// represents.
    pub fn drop_shim(&self) -> unsafe fn(*mut u8) {
        self.drop
    }
}

impl PartialOrd for TypeInfo {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TypeInfo {
    /// Order by alignment, descending. Ties broken with TypeId.
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.layout
            .align()
            .cmp(&other.layout.align())
            .reverse()
            .then_with(|| self.id.cmp(&other.id))
    }
}

impl PartialEq for TypeInfo {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for TypeInfo {}

/// Shared reference to a single column of component data in an [`Archetype`]
pub struct ArchetypeColumn<'a, T: Component> {
    archetype: &'a Archetype,
    column: &'a [T],
}

impl<'a, T: Component> ArchetypeColumn<'a, T> {
    pub(crate) fn new(archetype: &'a Archetype) -> Option<Self> {
        let state = archetype.get_state::<T>()?;
        let ptr = archetype.get_base::<T>(state);
        let column = unsafe { core::slice::from_raw_parts(ptr.as_ptr(), archetype.len() as usize) };
        archetype.borrow::<T>(state);
        Some(Self { archetype, column })
    }
}

impl<T: Component> Deref for ArchetypeColumn<'_, T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        self.column
    }
}

impl<T: Component> Drop for ArchetypeColumn<'_, T> {
    fn drop(&mut self) {
        let state = self.archetype.get_state::<T>().unwrap();
        self.archetype.release::<T>(state);
    }
}

impl<T: Component> Clone for ArchetypeColumn<'_, T> {
    fn clone(&self) -> Self {
        let state = self.archetype.get_state::<T>().unwrap();
        self.archetype.borrow::<T>(state);
        Self {
            archetype: self.archetype,
            column: self.column,
        }
    }
}

impl<T: Component + fmt::Debug> fmt::Debug for ArchetypeColumn<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.column.fmt(f)
    }
}

/// Unique reference to a single column of component data in an [`Archetype`]
pub struct ArchetypeColumnMut<'a, T: Component> {
    archetype: &'a Archetype,
    column: &'a mut [T],
}

impl<'a, T: Component> ArchetypeColumnMut<'a, T> {
    pub(crate) fn new(archetype: &'a Archetype) -> Option<Self> {
        let state = archetype.get_state::<T>()?;
        let ptr = archetype.get_base::<T>(state);
        let column =
            unsafe { core::slice::from_raw_parts_mut(ptr.as_ptr(), archetype.len() as usize) };
        archetype.borrow_mut::<T>(state);
        Some(Self { archetype, column })
    }
}

impl<T: Component> Deref for ArchetypeColumnMut<'_, T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        self.column
    }
}

impl<T: Component> DerefMut for ArchetypeColumnMut<'_, T> {
    fn deref_mut(&mut self) -> &mut [T] {
        self.column
    }
}

impl<T: Component> Drop for ArchetypeColumnMut<'_, T> {
    fn drop(&mut self) {
        let state = self.archetype.get_state::<T>().unwrap();
        self.archetype.release_mut::<T>(state);
    }
}

impl<T: Component + fmt::Debug> fmt::Debug for ArchetypeColumnMut<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.column.fmt(f)
    }
}
