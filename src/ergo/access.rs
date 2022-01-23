use alloc::{boxed::Box, vec::Vec};
use core::{
    any::TypeId,
    cell::{Cell, RefCell},
    ops::{Deref, DerefMut},
    ptr::{null_mut, NonNull},
};

use crate::{Component, Entity, TypeInfo, World};

pub(super) type BitsetChunk = usize;
#[derive(Default)]
pub(super) struct BitVec {
    pub(crate) storage: Vec<BitsetChunk>,
}
impl BitVec {
    pub fn chunk_bit_size() -> usize {
        BitsetChunk::BITS as usize
    }
}

pub(super) struct ComponentAccess {
    /// entity ID
    entity: Cell<Entity>,
    // component type ID
    component_type: Cell<TypeInfo>,
    /// address to the component data
    data_addr: Cell<*mut u8>,
    /// number of references to this [ComponentAccess]. max 255 refs
    refs: Cell<u8>,
    /// first bit is mutable borrow, rest immutable. max 127 active shared borrows
    borrow_counter: Cell<u8>,
}
impl Default for ComponentAccess {
    fn default() -> Self {
        Self {
            entity: Cell::new(Entity::from_bits(u64::MAX).unwrap()),
            component_type: Cell::new(TypeInfo::of::<()>()),
            data_addr: Cell::new(core::ptr::null_mut()),
            refs: Default::default(),
            borrow_counter: Default::default(),
        }
    }
}

impl ComponentAccess {
    fn increment_refs(&self) {
        self.refs
            .set(self.refs.get().checked_add(1).expect("too many refs"));
    }
    fn decrement_refs(&self) {
        self.refs.set(
            self.refs
                .get()
                .checked_sub(1)
                .expect("increment/decrement refs mismatch"),
        );
    }

    fn increment_read(&self) {
        let mut borrows = self.borrow_counter.get();
        if borrows & 0x1 != 0 {
            panic!("attempt to borrow a component immutably while a mutable borrow was active");
        }
        borrows = borrows.checked_add(2).expect("too many borrows");
        self.borrow_counter.set(borrows);
    }
    fn decrement_read(&self) {
        let mut borrows = self.borrow_counter.get();
        assert!(
            borrows & 0x1 == 0,
            "decrement read with active mutable borrow"
        );
        assert!(borrows >= 2, "increment/decrement read mismatch");
        borrows -= 2;
        self.borrow_counter.set(borrows);
    }

    fn take_write_lock(&self) {
        let borrows = self.borrow_counter.get();
        if borrows & 0x1 == 1 {
            panic!("attempt to borrow a component mutably while a mutable borrow was active");
        }
        if borrows > 1 {
            panic!("attempt to borrow a component mutably while an immutable borrow was active");
        }
        self.borrow_counter.set(1);
    }
    fn release_write_lock(&self) {
        let borrows = self.borrow_counter.get();
        assert!(
            borrows & 0x1 == 1,
            "release write lock without active write lock"
        );
        assert!(
            borrows == 1,
            "release write lock with active immutable borrow"
        );
        self.borrow_counter.set(0);
    }
    fn has_borrow(&self) -> bool {
        self.borrow_counter.get() != 0
    }
}

const COMPONENT_ACCESS_CHUNK_SIZE: usize = 32;
#[derive(Default)]
pub(super) struct AccessControl {
    /// bitset for whether an entity's metadata is defined by the World or in the scope wrapper
    entity_overrides: RefCell<BitVec>,
    /// allocated chunks of [ComponentAccess]s
    borrow_counter_chunks: RefCell<Vec<Box<[ComponentAccess; COMPONENT_ACCESS_CHUNK_SIZE]>>>,
}

impl AccessControl {
    pub fn prepare(&mut self, world: &World) {
        let capacity = (world.len() + 1).next_power_of_two();
        let mut overrides = self.entity_overrides.borrow_mut();
        overrides
            .storage
            .reserve(capacity as usize / BitVec::chunk_bit_size());
        overrides
            .storage
            .resize(world.len() as usize / BitVec::chunk_bit_size() + 1, 0);
    }

    pub(super) unsafe fn update_data_ptr(
        &self,
        entity: Entity,
        component_type: &TypeInfo,
        new_ptr: *mut u8,
    ) {
        let chunk_list = self.borrow_counter_chunks.borrow_mut();
        for chunk in chunk_list.iter() {
            for comp_access in chunk.iter() {
                if comp_access.entity.get() == entity
                    && comp_access.component_type.get() == *component_type
                {
                    comp_access.data_addr.set(new_ptr);
                }
            }
        }
    }

    pub unsafe fn get_typed_component_ref<T: Component>(
        &self,
        entity: Entity,
        comp_type: &TypeInfo,
        addr: NonNull<u8>,
    ) -> ComponentRef<T> {
        let component_ref = self.get_component_ref(entity, comp_type, addr);
        ComponentRef {
            component_ref,
            phantom: Default::default(),
        }
    }

    pub fn get_component_ref(
        &self,
        entity: Entity,
        component_type: &TypeInfo,
        addr: NonNull<u8>,
    ) -> GenericComponentRef {
        let mut chunk_list = self.borrow_counter_chunks.borrow_mut();
        // see if there are any active access items
        for chunk in chunk_list.iter() {
            for comp_access in chunk.iter() {
                if comp_access.data_addr.get() == addr.as_ptr() {
                    comp_access.increment_refs();
                    return GenericComponentRef::new(comp_access as *const ComponentAccess);
                }
            }
        }
        // reuse an existing access item if possible
        for chunk in chunk_list.iter() {
            for comp_access in chunk.iter() {
                if comp_access.refs.get() == 0 {
                    comp_access.data_addr.set(addr.as_ptr());
                    comp_access.entity.set(entity);
                    comp_access.component_type.set(*component_type);
                    comp_access.increment_refs();
                    return GenericComponentRef::new(comp_access as *const ComponentAccess);
                }
            }
        }

        // allocate new chunk
        chunk_list.push(Box::new(Default::default()));
        let new_item = &chunk_list.last_mut().unwrap()[0];
        new_item.data_addr.set(addr.as_ptr());
        new_item.entity.set(entity);
        new_item.component_type.set(*component_type);
        new_item.increment_refs();
        GenericComponentRef::new(new_item as *const ComponentAccess)
    }

    pub(super) fn expect_zero_refs(&self) {
        for chunk in self.borrow_counter_chunks.borrow().iter() {
            for item in chunk.iter() {
                if item.refs.get() > 0 {
                    panic!("active references when dropping ErgoScope");
                }
            }
        }
    }

    pub(super) fn has_active_borrows(&self, entity: Entity, comp_type: TypeId) -> bool {
        for chunk in self.borrow_counter_chunks.borrow().iter() {
            for item in chunk.iter() {
                if item.entity.get() == entity && item.component_type.get().id() == comp_type {
                    return item.has_borrow();
                }
            }
        }
        false
    }

    pub(super) fn is_entity_overridden(&self, entity: Entity) -> bool {
        let overrides = self.entity_overrides.borrow();
        let idx = entity.id;
        let override_bitchunk = idx * BitsetChunk::BITS;
        let bit_mask = 1 << (idx % BitsetChunk::BITS);
        (overrides.storage[override_bitchunk as usize] & bit_mask) != 0
    }

    pub(super) fn set_entity_overridden(&self, entity: Entity) {
        let mut overrides = self.entity_overrides.borrow_mut();
        let idx = entity.id;
        let override_bitchunk = idx * BitsetChunk::BITS;
        let bit_mask = 1 << (idx % BitsetChunk::BITS);
        overrides.storage[override_bitchunk as usize] |= bit_mask;
    }
}

pub struct GenericComponentRef {
    access_ptr: *const ComponentAccess,
}
impl Drop for GenericComponentRef {
    fn drop(&mut self) {
        let access = unsafe { &*self.access_ptr };
        access.decrement_refs();
    }
}
impl GenericComponentRef {
    fn new(access: *const ComponentAccess) -> Self {
        Self { access_ptr: access }
    }
    pub unsafe fn read(&self) -> GenericRef<'_> {
        let access = &*self.access_ptr;
        access.increment_read();
        GenericRef {
            access_ptr: self.access_ptr,
            phantom: Default::default(),
        }
    }
    pub unsafe fn write(&self) -> GenericRefMut<'_> {
        let access = &*self.access_ptr;
        access.take_write_lock();
        GenericRefMut {
            access_ptr: self.access_ptr,
            phantom: Default::default(),
        }
    }
}

pub struct GenericRef<'a> {
    access_ptr: *const ComponentAccess,
    phantom: core::marker::PhantomData<&'a ()>,
}
impl<'a> Drop for GenericRef<'a> {
    fn drop(&mut self) {
        let access = unsafe { &*self.access_ptr };
        access.decrement_read();
    }
}

impl<'a> GenericRef<'a> {
    pub fn ptr(&self) -> Option<NonNull<u8>> {
        NonNull::new(unsafe { &*self.access_ptr }.data_addr.get())
    }
}

pub struct GenericRefMut<'a> {
    access_ptr: *const ComponentAccess,
    phantom: core::marker::PhantomData<&'a mut ()>,
}
impl<'a> Drop for GenericRefMut<'a> {
    fn drop(&mut self) {
        let access = unsafe { &*self.access_ptr };
        access.release_write_lock();
    }
}

impl<'a> GenericRefMut<'a> {
    pub fn ptr(&self) -> Option<NonNull<u8>> {
        NonNull::new(unsafe { &*self.access_ptr }.data_addr.get())
    }
}

pub struct Ref<'a, T: Component> {
    access_ptr: *const ComponentAccess,
    phantom: core::marker::PhantomData<&'a T>,
}
impl<'a, T: Component> Drop for Ref<'a, T> {
    fn drop(&mut self) {
        let access = unsafe { &*self.access_ptr };
        access.decrement_read();
    }
}
impl<'a, T: Component> Deref for Ref<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe {
            let access = &*self.access_ptr;
            &*(access.data_addr.get() as *const T)
        }
    }
}

pub struct RefMut<'a, T: Component> {
    access_ptr: *const ComponentAccess,
    phantom: core::marker::PhantomData<&'a T>,
}
impl<'a, T: Component> Drop for RefMut<'a, T> {
    fn drop(&mut self) {
        let access = unsafe { &*self.access_ptr };
        access.release_write_lock();
    }
}
impl<'a, T: Component> Deref for RefMut<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe {
            let access = &*self.access_ptr;
            &*(access.data_addr.get() as *const T)
        }
    }
}

impl<'a, T: Component> DerefMut for RefMut<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe {
            let access = &*self.access_ptr;
            &mut *(access.data_addr.get() as *mut T)
        }
    }
}

pub struct ComponentRef<T: Component> {
    component_ref: GenericComponentRef,
    phantom: core::marker::PhantomData<T>,
}

impl<T: Component> ComponentRef<T> {
    pub fn read(&self) -> Ref<T> {
        let access = unsafe { &*self.component_ref.access_ptr };
        if access.data_addr.get() == null_mut() {
            panic!(
                "Component read attempted on removed component {} for entity {:?}",
                access.component_type.get().name().unwrap_or(""),
                access.entity.get()
            );
        }
        access.increment_read();

        Ref {
            access_ptr: self.component_ref.access_ptr,
            phantom: Default::default(),
        }
    }
    pub fn write(&mut self) -> RefMut<T> {
        let access = unsafe { &*self.component_ref.access_ptr };
        if access.data_addr.get() == null_mut() {
            panic!(
                "Component write attempted on removed component {} for entity {:?}",
                access.component_type.get().name().unwrap_or(""),
                access.entity.get()
            );
        }
        access.take_write_lock();
        RefMut {
            access_ptr: self.component_ref.access_ptr,
            phantom: Default::default(),
        }
    }
}
