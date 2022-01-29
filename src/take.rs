use alloc::vec::Vec;

use crate::{entities::Entities, Archetype, DynamicBundle, Entity, TypeInfo};

/// An entity removed from a `World`
pub struct TakenEntity<'a> {
    entities: &'a mut Entities,
    entity: Entity,
    archetype: &'a mut Archetype,
    index: u32,
    drop: bool,
}

impl<'a> TakenEntity<'a> {
    /// # Safety
    /// `index` must be in bounds in `archetype`
    pub(crate) unsafe fn new(
        entities: &'a mut Entities,
        entity: Entity,
        archetype: &'a mut Archetype,
        index: u32,
    ) -> Self {
        Self {
            entities,
            entity,
            archetype,
            index,
            drop: true,
        }
    }
}

unsafe impl<'a> DynamicBundle for TakenEntity<'a> {
    fn with_ids<T>(&self, f: impl FnOnce(&[core::any::TypeId]) -> T) -> T {
        f(self.archetype.type_ids())
    }

    fn type_info(&self) -> Vec<crate::TypeInfo> {
        self.archetype.types().to_vec()
    }

    unsafe fn put(mut self, mut f: impl FnMut(*mut u8, TypeInfo)) {
        // Suppress dropping of moved components
        self.drop = false;
        for &ty in self.archetype.types() {
            let ptr = self
                .archetype
                .get_dynamic(ty.id(), ty.layout().size(), self.index)
                .unwrap();
            f(ptr.as_ptr(), ty)
        }
    }
}

impl Drop for TakenEntity<'_> {
    fn drop(&mut self) {
        unsafe {
            if let Some(moved) = self.archetype.remove(self.index, self.drop) {
                self.entities.meta[moved as usize].location.index = self.index;
            }
            self.entities.free(self.entity).unwrap();
        }
    }
}
