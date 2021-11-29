use crate::alloc::collections::BinaryHeap;
use core::{any::TypeId, fmt, mem::MaybeUninit, slice};

use crate::{
    archetype::{TypeIdMap, TypeInfo},
    Archetype, Component,
};

/// A collection of component types
#[derive(Debug, Clone, Default)]
pub struct ColumnBatchType {
    types: BinaryHeap<TypeInfo>,
}

impl ColumnBatchType {
    /// Create an empty type
    pub fn new() -> Self {
        Self::default()
    }

    /// Update to include `T` components
    pub fn add<T: Component>(&mut self) -> &mut Self {
        self.types.push(TypeInfo::of::<T>());
        self
    }

    /// Construct a [`ColumnBatchBuilder`] for *exactly* `size` entities with these components
    pub fn into_batch(self, size: u32) -> ColumnBatchBuilder {
        let mut types = self.types.into_sorted_vec();
        types.dedup();
        let fill = TypeIdMap::with_capacity_and_hasher(types.len(), Default::default());
        let mut arch = Archetype::new(types);
        arch.reserve(size);
        ColumnBatchBuilder {
            fill,
            target_fill: size,
            archetype: Some(arch),
        }
    }
}

/// An incomplete collection of component data for entities with the same component types
pub struct ColumnBatchBuilder {
    /// Number of components written so far for each component type
    fill: TypeIdMap<u32>,
    target_fill: u32,
    pub(crate) archetype: Option<Archetype>,
}

unsafe impl Send for ColumnBatchBuilder {}
unsafe impl Sync for ColumnBatchBuilder {}

impl ColumnBatchBuilder {
    /// Create a batch for *exactly* `size` entities with certain component types
    pub fn new(ty: ColumnBatchType, size: u32) -> Self {
        ty.into_batch(size)
    }

    /// Get a handle for inserting `T` components if `T` was in the [`ColumnBatchType`]
    pub fn writer<T: Component>(&mut self) -> Option<BatchWriter<'_, T>> {
        let archetype = self.archetype.as_mut().unwrap();
        let state = archetype.get_state::<T>()?;
        let base = archetype.get_base::<T>(state);
        Some(BatchWriter {
            fill: self.fill.entry(TypeId::of::<T>()).or_insert(0),
            storage: unsafe {
                slice::from_raw_parts_mut(base.as_ptr().cast(), self.target_fill as usize)
                    .iter_mut()
            },
        })
    }

    /// Finish the batch, failing if any components are missing
    pub fn build(mut self) -> Result<ColumnBatch, BatchIncomplete> {
        let mut archetype = self.archetype.take().unwrap();
        if archetype
            .types()
            .iter()
            .any(|ty| self.fill.get(&ty.id()).copied().unwrap_or(0) != self.target_fill)
        {
            return Err(BatchIncomplete { _opaque: () });
        }
        unsafe {
            archetype.set_len(self.target_fill);
        }
        Ok(ColumnBatch(archetype))
    }
}

impl Drop for ColumnBatchBuilder {
    fn drop(&mut self) {
        if let Some(archetype) = self.archetype.take() {
            for ty in archetype.types() {
                let fill = self.fill.get(&ty.id()).copied().unwrap_or(0);
                unsafe {
                    let base = archetype.get_dynamic(ty.id(), 0, 0).unwrap();
                    for i in 0..fill {
                        base.as_ptr().add(i as usize).drop_in_place()
                    }
                }
            }
        }
    }
}

/// A collection of component data for entities with the same component types
pub struct ColumnBatch(pub(crate) Archetype);

/// Handle for appending components
pub struct BatchWriter<'a, T> {
    fill: &'a mut u32,
    storage: core::slice::IterMut<'a, MaybeUninit<T>>,
}

impl<T> BatchWriter<'_, T> {
    /// Add a component if there's space remaining
    pub fn push(&mut self, x: T) -> Result<(), T> {
        match self.storage.next() {
            None => Err(x),
            Some(slot) => {
                *slot = MaybeUninit::new(x);
                *self.fill += 1;
                Ok(())
            }
        }
    }

    /// How many components have been added so far
    pub fn fill(&self) -> u32 {
        *self.fill
    }
}

/// Error indicating that a [`ColumnBatchBuilder`] was missing components
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct BatchIncomplete {
    _opaque: (),
}

#[cfg(feature = "std")]
impl std::error::Error for BatchIncomplete {}

impl fmt::Display for BatchIncomplete {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("batch incomplete")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_batch() {
        let mut types = ColumnBatchType::new();
        types.add::<usize>();
        let mut builder = types.into_batch(0);
        let mut writer = builder.writer::<usize>().unwrap();
        assert!(writer.push(42).is_err());
    }
}
