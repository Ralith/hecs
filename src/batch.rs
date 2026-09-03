use crate::alloc::{
    alloc::{alloc, dealloc},
    collections::BinaryHeap,
};
use core::{
    any::{type_name, TypeId},
    fmt,
    mem::MaybeUninit,
    slice,
    sync::atomic::{AtomicU32, Ordering},
};

use crate::{
    archetype::{TypeIdMap, TypeInfo},
    Archetype, Bundle, Component,
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

    /// [Self::add()] but using type information determined at runtime via [TypeInfo::of()]
    pub fn add_dynamic(&mut self, id: TypeInfo) -> &mut Self {
        self.types.push(id);
        self
    }

    /// Update to include all the components in bundle `T`
    pub fn add_bundle<T: Bundle>(&mut self) -> &mut Self {
        T::with_static_type_info(|type_infos| {
            type_infos.iter().for_each(|info| self.types.push(*info))
        });
        self
    }

    /// Construct a [`ColumnBatchBuilder`] for *exactly* `size` entities with these components
    pub fn into_batch(self, size: u32) -> ColumnBatchBuilder {
        assert!(size < u32::MAX);
        let mut types = self.types.into_sorted_vec();
        types.dedup();
        let fill = types
            .iter()
            .map(|ty| (ty.id(), AtomicU32::new(0)))
            .collect();
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
    fill: TypeIdMap<AtomicU32>,
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
    pub fn writer<T: Component>(&self) -> Option<BatchWriter<'_, T>> {
        let archetype = self.archetype.as_ref().unwrap();
        let state = archetype.get_state::<T>()?;
        let base = unsafe { archetype.get_base::<T>(state) };
        let fill_storage = self.fill.get(&TypeId::of::<T>()).unwrap();
        let fill = fill_storage.swap(u32::MAX, Ordering::Acquire);
        if fill == u32::MAX {
            panic!("another {} writer still exists", type_name::<T>());
        }
        Some(BatchWriter {
            fill_storage,
            fill,
            storage: unsafe {
                &mut slice::from_raw_parts_mut(base.as_ptr().cast(), self.target_fill as usize)
                    [fill as usize..]
            }
            .iter_mut(),
        })
    }

    /// Finish the batch, failing if any components are missing
    pub fn build(mut self) -> Result<ColumnBatch, BatchIncomplete> {
        let mut archetype = self.archetype.take().unwrap();
        if archetype
            .types()
            .iter()
            .any(|ty| *self.fill.get_mut(&ty.id()).unwrap().get_mut() != self.target_fill)
        {
            self.archetype = Some(archetype); // Let Drop do its thing
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
                let fill = *self.fill.get_mut(&ty.id()).unwrap().get_mut();
                unsafe {
                    let scratch = alloc(ty.layout());
                    let base = archetype.get_dynamic(ty.id(), 0, 0).unwrap();
                    for i in 0..fill {
                        scratch.copy_from_nonoverlapping(
                            base.as_ptr().add(i as usize * ty.layout().size()),
                            ty.layout().size(),
                        );
                        ty.drop(scratch);
                    }
                    dealloc(scratch, ty.layout());
                }
            }
        }
    }
}

/// A collection of component data for entities with the same component types
pub struct ColumnBatch(pub(crate) Archetype);

/// Handle for appending components
pub struct BatchWriter<'a, T> {
    fill_storage: &'a AtomicU32,
    fill: u32,
    storage: core::slice::IterMut<'a, MaybeUninit<T>>,
}

impl<T> BatchWriter<'_, T> {
    /// Add a component if there's space remaining
    pub fn push(&mut self, x: T) -> Result<(), T> {
        match self.storage.next() {
            None => Err(x),
            Some(slot) => {
                *slot = MaybeUninit::new(x);
                self.fill += 1;
                Ok(())
            }
        }
    }

    /// How many components have been added so far
    pub fn fill(&self) -> u32 {
        self.fill
    }
}

impl<T> Drop for BatchWriter<'_, T> {
    fn drop(&mut self) {
        // Release any reference to component storage before permitting another writer to be built
        // for this type
        self.storage = core::slice::IterMut::default();
        self.fill_storage.store(self.fill, Ordering::Release);
    }
}

/// Error indicating that a [`ColumnBatchBuilder`] was missing components
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct BatchIncomplete {
    _opaque: (),
}

impl core::error::Error for BatchIncomplete {}

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
        let builder = types.into_batch(0);
        let mut writer = builder.writer::<usize>().unwrap();
        assert!(writer.push(42).is_err());
    }

    #[test]
    fn writer_continues_from_last_fill() {
        let mut types = ColumnBatchType::new();
        types.add::<usize>();
        let builder = types.into_batch(2);
        {
            let mut writer = builder.writer::<usize>().unwrap();
            writer.push(42).unwrap();
        }

        let mut writer = builder.writer::<usize>().unwrap();

        assert_eq!(writer.push(42), Ok(()));
        assert_eq!(writer.push(42), Err(42));
    }

    #[test]
    fn concurrent_writers() {
        let mut types = ColumnBatchType::new();
        types.add::<usize>();
        types.add::<u32>();
        let builder = types.into_batch(2);
        {
            let mut a = builder.writer::<usize>().unwrap();
            let mut b = builder.writer::<u32>().unwrap();
            for i in 0..2 {
                a.push(i as usize).unwrap();
                b.push(i).unwrap();
            }
        }
        builder.build().unwrap();
    }

    #[test]
    #[should_panic(expected = "writer still exists")]
    fn aliasing_writers() {
        let mut types = ColumnBatchType::new();
        types.add::<usize>();
        let builder = types.into_batch(2);
        let _a = builder.writer::<usize>().unwrap();
        let _b = builder.writer::<usize>().unwrap();
    }

    #[test]
    #[cfg(feature = "std")]
    fn dropping_builder_drops_elements() {
        use std::sync::Arc;

        let mut types = ColumnBatchType::new();
        types.add::<Arc<()>>();
        let builder = types.into_batch(1);
        let value = Arc::new(());
        builder
            .writer::<Arc<()>>()
            .unwrap()
            .push(value.clone())
            .unwrap();
        assert_eq!(Arc::strong_count(&value), 2);
        drop(builder);
        assert_eq!(Arc::strong_count(&value), 1);
    }

    #[test]
    #[cfg(feature = "std")]
    fn build_error_drops_elements() {
        use std::sync::Arc;
        let mut ty = ColumnBatchType::new();
        ty.add::<Arc<()>>();
        ty.add::<u32>();

        let builder = ty.into_batch(2);
        let value = Arc::new(());
        {
            let mut writer = builder.writer::<Arc<()>>().unwrap();
            writer.push(value.clone()).unwrap();
            writer.push(value.clone()).unwrap();
        }
        assert!(builder.build().is_err());
        assert_eq!(Arc::strong_count(&value), 1);
    }
}
