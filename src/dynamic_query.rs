use core::{alloc::Layout, any::TypeId, ptr::NonNull};

use crate::{entities::EntityMeta, Archetype, Entity};

/// The component types accessed by a dynamic query.
#[derive(Debug, Copy, Clone)]
pub struct DynamicQueryTypes<'a> {
    pub(crate) read_types: &'a [TypeId],
    pub(crate) write_types: &'a [TypeId],
}

impl<'a> DynamicQueryTypes<'a> {
    /// Creates a dynamic query that reads component types in `read_types`
    /// and writes component types in `write_types`.
    pub fn new(read_types: &'a [TypeId], write_types: &'a [TypeId]) -> Self {
        Self {
            read_types,
            write_types,
        }
    }

    /// Gets the component types to be read by this query.
    pub fn read_types(&self) -> &[TypeId] {
        self.read_types
    }

    /// Gets the component types to be accessed mutably by this query.
    pub fn write_types(&self) -> &[TypeId] {
        self.write_types
    }
}

/// A pointer to a slice of component data from a [`DynamicQuery`].
pub struct ComponentSlice {
    typ: TypeId,
    ptr: NonNull<u8>,
    len: usize,
    component_layout: Layout,
}

impl ComponentSlice {
    /// Returns the number of components in this slice.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns the layout of a single component in this slice.
    pub fn component_layout(&self) -> Layout {
        self.component_layout
    }

    /// Returns the number of bytes in the component slice.
    pub fn len_in_bytes(&self) -> usize {
        self.len() * self.component_layout().size()
    }

    /// Returns a pointer to the raw component data.
    pub fn ptr(&self) -> NonNull<u8> {
        self.ptr
    }

    /// Casts this component slice to a slice of type `T`.
    ///
    /// # Panics
    /// Panics if `T` is not the type of the components in this slice.
    pub fn as_slice<T: 'static>(&self) -> &[T] {
        assert_eq!(TypeId::of::<T>(), self.typ, "component type does not match");
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr() as *const T, self.len) }
    }
}

/// The result of a dynamic query.
pub struct DynamicQuery<'q> {
    types: DynamicQueryTypes<'q>,
    archetypes: &'q [Archetype],
    entity_meta: &'q [EntityMeta],
}

impl<'q> DynamicQuery<'q> {
    pub(crate) fn new(
        types: DynamicQueryTypes<'q>,
        archetypes: &'q [Archetype],
        entity_meta: &'q [EntityMeta],
    ) -> Self {
        Self {
            types,
            archetypes,
            entity_meta,
        }
    }

    /// Returns an iterator over entities yielded by this query.
    pub fn iter_entities<'a>(&'a self) -> impl Iterator<Item = Entity> + 'a {
        self.iter_matching_archetypes()
            .flat_map(|archetype| archetype.entity_slice().iter().copied())
            .map(move |id| Entity {
                generation: unsafe { self.entity_meta.get_unchecked(id as usize).generation },
                id,
            })
    }

    /// Returns an iterator over pointers to components of the given type
    /// yielded by this query.
    pub fn iter_component_slices<'a>(
        &'a self,
        component_type: TypeId,
    ) -> impl Iterator<Item = ComponentSlice> + 'a {
        self.iter_matching_archetypes()
            .map(move |archetype| unsafe {
                let ptr = archetype
                    .get_dynamic(component_type, 0, 0)
                    .expect("component not in query");
                let len = archetype.len() as usize;
                let component_layout = archetype.component_layout(component_type).unwrap();
                ComponentSlice {
                    typ: component_type,
                    ptr,
                    len,
                    component_layout,
                }
            })
    }

    fn iter_matching_archetypes<'a>(&'a self) -> impl Iterator<Item = &'q Archetype> + 'a {
        self.archetypes
            .iter()
            .filter(move |archetype| archetype.access_dynamic(&self.types).is_some())
            .filter(|archetype| archetype.len() > 0)
    }
}
