//! Fast column-major serialization
//!
//! Stores each archetype in a block where each type of component is laid out
//! contiguously. Preferred for data that will be read/written programmatically. Efficient, compact,
//! and highly compressible, but difficult to read or edit by hand.
//!
//! This module builds on the public archetype-related APIs and [`World::spawn_column_batch_at()`],
//! and is somewhat opinionated. For some applications, a custom approach may be preferable.
//!
//! In terms of the serde data model, we treat a [`World`] as a sequence of archetypes, where each
//! archetype is a 4-tuple of an entity count `n`, component count `k`, a `k`-tuple of
//! user-controlled component IDs, and a `k+1`-tuple of `n`-tuples of components, such that the
//! first `n`-tuple contains `Entity` values and the remainder each contain components of the type
//! identified by the corresponding component ID.

use crate::alloc::vec::Vec;
use core::{any::type_name, cell::RefCell, fmt, marker::PhantomData};

use serde::{
    de::{self, DeserializeSeed, SeqAccess, Unexpected, Visitor},
    ser::{SerializeSeq, SerializeTuple},
    Deserialize, Deserializer, Serialize, Serializer,
};

use crate::{
    Archetype, ColumnBatch, ColumnBatchBuilder, ColumnBatchType, Component, Entity, World,
};

/// Implements serialization of archetypes
///
/// `serialize_component_ids` and `serialize_components` must serialize exactly the number of
/// elements indicated by `component_count` or return `Err`.
///
/// Data external to the [`World`] can be exposed during serialization by storing references inside
/// the struct implementing this trait.
///
/// # Example
///
/// ```
/// # use serde::{Serialize, Deserialize};
/// # #[derive(Serialize)]
/// # struct Position([f32; 3]);
/// # #[derive(Serialize)]
/// # struct Velocity([f32; 3]);
/// use std::any::TypeId;
/// use hecs::{*, serialize::column::*};
///
/// #[derive(Serialize, Deserialize)]
/// enum ComponentId { Position, Velocity }
///
/// struct Context;
///
/// impl SerializeContext for Context {
///     fn component_count(&self, archetype: &Archetype) -> usize {
///         archetype.component_types()
///             .filter(|&t| t == TypeId::of::<Position>() || t == TypeId::of::<Velocity>())
///             .count()
///     }
///
///     fn serialize_component_ids<S: serde::ser::SerializeTuple>(
///         &mut self,
///         archetype: &Archetype,
///         mut out: S,
///     ) -> Result<S::Ok, S::Error> {
///         try_serialize_id::<Position, _, _>(archetype, &ComponentId::Position, &mut out)?;
///         try_serialize_id::<Velocity, _, _>(archetype, &ComponentId::Velocity, &mut out)?;
///         out.end()
///     }
///
///     fn serialize_components<S: serde::ser::SerializeTuple>(
///         &mut self,
///         archetype: &Archetype,
///         mut out: S,
///     ) -> Result<S::Ok, S::Error> {
///         try_serialize::<Position, _>(archetype, &mut out)?;
///         try_serialize::<Velocity, _>(archetype, &mut out)?;
///         out.end()
///     }
/// }
/// ```
// Serializing the ID tuple separately from component data allows the deserializer to allocate the
// entire output archetype up front, rather than having to allocate storage for each component type
// after processing the previous one and copy into an archetype at the end.
pub trait SerializeContext {
    /// Number of entries that [`serialize_component_ids`](Self::serialize_component_ids) and
    /// [`serialize_components`](Self::serialize_components) will produce for `archetype`
    fn component_count(&self, archetype: &Archetype) -> usize;

    /// Serialize the IDs of the components from `archetype` that will be serialized
    // We use a wrapper here rather than exposing the serde type directly because it's a huge pain
    // to determine how many IDs were written otherwise, and we need that to set the component data
    // tuple length correctly.
    fn serialize_component_ids<S: SerializeTuple>(
        &mut self,
        archetype: &Archetype,
        out: S,
    ) -> Result<S::Ok, S::Error>;

    /// Serialize component data from `archetype` into `out`
    ///
    /// For each component ID written by `serialize_component_ids`, this method must write a tuple
    /// containing one value for each entity, e.g. using [`try_serialize`], in the same order. Each
    /// tuple's length must exactly match the number of entities in `archetype`, and there must be
    /// exactly the same number of tuples as there were IDs.
    fn serialize_components<S: SerializeTuple>(
        &mut self,
        archetype: &Archetype,
        out: S,
    ) -> Result<S::Ok, S::Error>;
}

/// If `archetype` has `T` components, serialize `id` into `S`
pub fn try_serialize_id<T, I, S>(archetype: &Archetype, id: &I, out: &mut S) -> Result<(), S::Error>
where
    T: Component,
    I: Serialize + ?Sized,
    S: SerializeTuple,
{
    if archetype.has::<T>() {
        out.serialize_element(id)?;
    }
    Ok(())
}

/// If `archetype` has `T` components, serialize them into `out`
///
/// Useful for implementing [`SerializeContext::serialize_components()`].
pub fn try_serialize<T, S>(archetype: &Archetype, out: &mut S) -> Result<(), S::Error>
where
    T: Component + Serialize,
    S: SerializeTuple,
{
    if let Some(xs) = archetype.get::<T>() {
        serialize_collection(&*xs, out)?;
    }
    Ok(())
}

/// Serialize components from `collection` into a single element of `out`
fn serialize_collection<I, S>(collection: I, out: &mut S) -> Result<(), S::Error>
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator,
    I::Item: Serialize,
    S: SerializeTuple,
{
    struct SerializeColumn<I>(RefCell<I>);

    impl<I> Serialize for SerializeColumn<I>
    where
        I: ExactSizeIterator,
        I::Item: Serialize,
    {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let mut iter = self.0.borrow_mut();
            let mut tuple = serializer.serialize_tuple(iter.len())?;
            for x in &mut *iter {
                tuple.serialize_element(&x)?;
            }
            tuple.end()
        }
    }

    out.serialize_element(&SerializeColumn(RefCell::new(collection.into_iter())))
}

/// Serialize a [`World`] through a [`SerializeContext`] to a [`Serializer`]
pub fn serialize<C, S>(world: &World, context: &mut C, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    C: SerializeContext,
{
    struct SerializeArchetype<'a, C> {
        world: &'a World,
        archetype: &'a Archetype,
        ctx: RefCell<&'a mut C>,
    }

    impl<C> Serialize for SerializeArchetype<'_, C>
    where
        C: SerializeContext,
    {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let ctx = &mut *self.ctx.borrow_mut();
            let mut tuple = serializer.serialize_tuple(4)?;
            tuple.serialize_element(&self.archetype.len())?;
            tuple.serialize_element(&(self.archetype.types().len() as u32))?;
            let components = ctx.component_count(self.archetype);
            let helper = SerializeComponentIds::<'_, C> {
                archetype: self.archetype,
                ctx: RefCell::new(ctx),
                components,
            };
            tuple.serialize_element(&helper)?;
            tuple.serialize_element(&SerializeComponents::<'_, C> {
                world: self.world,
                archetype: self.archetype,
                ctx: RefCell::new(ctx),
                components,
            })?;
            tuple.end()
        }
    }

    struct SerializeComponentIds<'a, C> {
        archetype: &'a Archetype,
        ctx: RefCell<&'a mut C>,
        components: usize,
    }

    impl<C> Serialize for SerializeComponentIds<'_, C>
    where
        C: SerializeContext,
    {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let tuple = serializer.serialize_tuple(self.components)?;
            self.ctx
                .borrow_mut()
                .serialize_component_ids(self.archetype, tuple)
        }
    }

    struct SerializeComponents<'a, C> {
        world: &'a World,
        archetype: &'a Archetype,
        ctx: RefCell<&'a mut C>,
        components: usize,
    }

    impl<C> Serialize for SerializeComponents<'_, C>
    where
        C: SerializeContext,
    {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let ctx = &mut *self.ctx.borrow_mut();
            let mut tuple = serializer.serialize_tuple(self.components + 1)?;

            // Serialize entity IDs
            tuple.serialize_element(&SerializeEntities {
                world: self.world,
                ids: self.archetype.ids(),
            })?;

            // Serialize component data
            ctx.serialize_components(self.archetype, tuple)
        }
    }

    struct SerializeEntities<'a> {
        world: &'a World,
        ids: &'a [u32],
    }

    impl Serialize for SerializeEntities<'_> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let mut tuple = serializer.serialize_tuple(self.ids.len())?;
            for &id in self.ids {
                let entity = unsafe { self.world.find_entity_from_id(id) };
                tuple.serialize_element(&entity)?;
            }
            tuple.end()
        }
    }

    let mut seq =
        serializer.serialize_seq(Some(world.archetypes().filter(|x| !x.is_empty()).count()))?;
    for archetype in world.archetypes().filter(|x| !x.is_empty()) {
        seq.serialize_element(&SerializeArchetype {
            world,
            archetype,
            ctx: RefCell::new(context),
        })?;
    }
    seq.end()
}

/// Implements deserialization of archetypes
///
/// # Example
///
/// ```
/// # use serde::{Serialize, Deserialize};
/// # #[derive(Deserialize)]
/// # struct Position([f32; 3]);
/// # #[derive(Deserialize)]
/// # struct Velocity([f32; 3]);
/// use hecs::{*, serialize::column::*};
///
/// #[derive(Serialize, Deserialize)]
/// enum ComponentId { Position, Velocity }
///
/// // Could include references to external state for use by serialization methods
/// struct Context {
///     /// Components of the archetype currently being deserialized
///     components: Vec<ComponentId>,
/// }
///
/// impl DeserializeContext for Context {
///     fn deserialize_component_ids<'de, A>(
///         &mut self,
///         mut seq: A,
///     ) -> Result<ColumnBatchType, A::Error>
///     where
///         A: serde::de::SeqAccess<'de>,
///     {
///         self.components.clear(); // Discard data from the previous archetype
///         let mut batch = ColumnBatchType::new();
///         while let Some(id) = seq.next_element()? {
///             match id {
///                 ComponentId::Position => {
///                     batch.add::<Position>();
///                 }
///                 ComponentId::Velocity => {
///                     batch.add::<Velocity>();
///                 }
///             }
///             self.components.push(id);
///         }
///         Ok(batch)
///     }
///
///     fn deserialize_components<'de, A>(
///         &mut self,
///         entity_count: u32,
///         mut seq: A,
///         batch: &mut ColumnBatchBuilder,
///     ) -> Result<(), A::Error>
///     where
///         A: serde::de::SeqAccess<'de>,
///     {
///         // Decode component data in the order that the component IDs appeared
///         for component in &self.components {
///             match *component {
///                 ComponentId::Position => {
///                     deserialize_column::<Position, _>(entity_count, &mut seq, batch)?;
///                 }
///                 ComponentId::Velocity => {
///                     deserialize_column::<Velocity, _>(entity_count, &mut seq, batch)?;
///                 }
///             }
///         }
///         Ok(())
///     }
/// }
pub trait DeserializeContext {
    /// Deserialize a set of component IDs
    ///
    /// Implementers should usually store the deserialized component IDs in `self` to guide the
    /// following `deserialize_components` call.
    fn deserialize_component_ids<'de, A>(&mut self, seq: A) -> Result<ColumnBatchType, A::Error>
    where
        A: SeqAccess<'de>;

    /// Deserialize all component data for an archetype
    ///
    /// `seq` is a sequence of tuples directly corresponding to the IDs read in
    /// `deserialize_component_ids`, each containing `entity_count` elements.
    fn deserialize_components<'de, A>(
        &mut self,
        entity_count: u32,
        seq: A,
        batch: &mut ColumnBatchBuilder,
    ) -> Result<(), A::Error>
    where
        A: SeqAccess<'de>;
}

/// Deserialize a column of `entity_count` `T`s from `seq` into `out`
pub fn deserialize_column<'de, T, A>(
    entity_count: u32,
    seq: &mut A,
    out: &mut ColumnBatchBuilder,
) -> Result<(), A::Error>
where
    T: Component + Deserialize<'de>,
    A: SeqAccess<'de>,
{
    seq.next_element_seed(DeserializeColumn::<T>::new(entity_count, out))?
        .ok_or_else(|| {
            de::Error::invalid_value(
                Unexpected::Other("end of components"),
                &"a column of components",
            )
        })
}

/// Deserializer for a single component type, for use in [`DeserializeContext::deserialize_components()`]
struct DeserializeColumn<'a, T> {
    entity_count: u32,
    out: &'a mut ColumnBatchBuilder,
    marker: PhantomData<fn() -> T>,
}

impl<'de, 'a, T> DeserializeColumn<'a, T>
where
    T: Component + Deserialize<'de>,
{
    /// Construct a deserializer for `entity_count` `T` components, writing into `batch`
    pub fn new(entity_count: u32, batch: &'a mut ColumnBatchBuilder) -> Self {
        Self {
            entity_count,
            out: batch,
            marker: PhantomData,
        }
    }
}

impl<'de, 'a, T> DeserializeSeed<'de> for DeserializeColumn<'a, T>
where
    T: Component + Deserialize<'de>,
{
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<(), D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_tuple(
            self.entity_count as usize,
            ColumnVisitor::<T> {
                entity_count: self.entity_count,
                out: self.out,
                marker: PhantomData,
            },
        )
    }
}

struct ColumnVisitor<'a, T> {
    entity_count: u32,
    out: &'a mut ColumnBatchBuilder,
    marker: PhantomData<fn() -> T>,
}

impl<'de, 'a, T> Visitor<'de> for ColumnVisitor<'a, T>
where
    T: Component + Deserialize<'de>,
{
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        write!(
            formatter,
            "a set of {} {} values",
            self.entity_count,
            type_name::<T>()
        )
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<(), A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut out = self.out.writer::<T>().expect("unexpected component type");
        while let Some(component) = seq.next_element()? {
            if out.push(component).is_err() {
                return Err(de::Error::invalid_value(
                    Unexpected::Other("extra component"),
                    &self,
                ));
            }
        }
        if out.fill() < self.entity_count {
            return Err(de::Error::invalid_length(out.fill() as usize, &self));
        }
        Ok(())
    }
}

/// Deserialize a [`World`] with a [`DeserializeContext`] and a [`Deserializer`]
pub fn deserialize<'de, C, D>(context: &mut C, deserializer: D) -> Result<World, D::Error>
where
    C: DeserializeContext,
    D: Deserializer<'de>,
{
    deserializer.deserialize_seq(WorldVisitor(context))
}

struct WorldVisitor<'a, C>(&'a mut C);

impl<'de, 'a, C> Visitor<'de> for WorldVisitor<'a, C>
where
    C: DeserializeContext,
{
    type Value = World;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a sequence of archetypes")
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<World, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut world = World::new();
        let mut entities = Vec::new();
        while let Some(bundle) =
            seq.next_element_seed(DeserializeArchetype(self.0, &mut entities))?
        {
            world.spawn_column_batch_at(&entities, bundle);
            entities.clear();
        }
        Ok(world)
    }
}

struct DeserializeArchetype<'a, C>(&'a mut C, &'a mut Vec<Entity>);

impl<'de, 'a, C> DeserializeSeed<'de> for DeserializeArchetype<'a, C>
where
    C: DeserializeContext,
{
    type Value = ColumnBatch;

    fn deserialize<D>(self, deserializer: D) -> Result<ColumnBatch, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_tuple(4, ArchetypeVisitor(self.0, self.1))
    }
}

struct ArchetypeVisitor<'a, C>(&'a mut C, &'a mut Vec<Entity>);

impl<'de, 'a, C> Visitor<'de> for ArchetypeVisitor<'a, C>
where
    C: DeserializeContext,
{
    type Value = ColumnBatch;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a 4-tuple of an entity count, a component count, a component ID list, and a component value list")
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<ColumnBatch, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let entity_count = seq
            .next_element::<u32>()?
            .ok_or_else(|| de::Error::invalid_length(0, &self))?;
        let component_count = seq
            .next_element::<u32>()?
            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
        self.1.reserve(entity_count as usize);
        let ty = seq
            .next_element_seed(DeserializeComponentIds(self.0, component_count))?
            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
        let mut batch = ty.into_batch(entity_count);
        seq.next_element_seed(DeserializeComponents {
            ctx: self.0,
            entity_count,
            component_count,
            entities: self.1,
            out: &mut batch,
        })?
        .ok_or_else(|| de::Error::invalid_length(3, &self))?;
        batch.build().map_err(|_| {
            de::Error::invalid_value(
                Unexpected::Other("incomplete archetype"),
                &"a complete archetype",
            )
        })
    }
}

struct DeserializeComponentIds<'a, C>(&'a mut C, u32);

impl<'de, 'a, C> DeserializeSeed<'de> for DeserializeComponentIds<'a, C>
where
    C: DeserializeContext,
{
    type Value = ColumnBatchType;

    fn deserialize<D>(self, deserializer: D) -> Result<ColumnBatchType, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_tuple(self.1 as usize, ComponentIdVisitor(self.0, self.1))
    }
}

struct ComponentIdVisitor<'a, C>(&'a mut C, u32);

impl<'de, 'a, C> Visitor<'de> for ComponentIdVisitor<'a, C>
where
    C: DeserializeContext,
{
    type Value = ColumnBatchType;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "a set of {} component IDs", self.1)
    }

    fn visit_seq<A>(self, seq: A) -> Result<ColumnBatchType, A::Error>
    where
        A: SeqAccess<'de>,
    {
        self.0.deserialize_component_ids(seq)
    }
}

struct DeserializeComponents<'a, C> {
    ctx: &'a mut C,
    component_count: u32,
    entity_count: u32,
    entities: &'a mut Vec<Entity>,
    out: &'a mut ColumnBatchBuilder,
}

impl<'de, 'a, C> DeserializeSeed<'de> for DeserializeComponents<'a, C>
where
    C: DeserializeContext,
{
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<(), D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_tuple(
            self.component_count as usize + 1,
            ComponentsVisitor {
                ctx: self.ctx,
                entity_count: self.entity_count,
                entities: self.entities,
                out: self.out,
            },
        )
    }
}

struct ComponentsVisitor<'a, C> {
    ctx: &'a mut C,
    entity_count: u32,
    entities: &'a mut Vec<Entity>,
    out: &'a mut ColumnBatchBuilder,
}

impl<'de, 'a, C> Visitor<'de> for ComponentsVisitor<'a, C>
where
    C: DeserializeContext,
{
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "a set of {} components", self.entity_count)
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<(), A::Error>
    where
        A: SeqAccess<'de>,
    {
        seq.next_element_seed(DeserializeEntities {
            count: self.entity_count,
            out: self.entities,
        })?;
        self.ctx
            .deserialize_components(self.entity_count, seq, self.out)
    }
}

struct DeserializeEntities<'a> {
    count: u32,
    out: &'a mut Vec<Entity>,
}

impl<'de, 'a> DeserializeSeed<'de> for DeserializeEntities<'a> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<(), D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_tuple(
            self.count as usize,
            EntitiesVisitor {
                count: self.count,
                out: self.out,
            },
        )
    }
}

struct EntitiesVisitor<'a> {
    count: u32,
    out: &'a mut Vec<Entity>,
}

impl<'de, 'a> Visitor<'de> for EntitiesVisitor<'a> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        write!(formatter, "a list of {} entity IDs", self.count)
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<(), A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut n = 0;
        while let Some(id) = seq.next_element()? {
            self.out.push(id);
            n += 1;
        }
        if n != self.count {
            return Err(de::Error::invalid_length(n as usize, &self));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::alloc::vec::Vec;
    use core::fmt;

    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::*;

    #[derive(Serialize, Deserialize, PartialEq, Debug, Copy, Clone)]
    struct Position([f32; 3]);
    #[derive(Serialize, Deserialize, PartialEq, Debug, Copy, Clone)]
    struct Velocity([f32; 3]);

    #[derive(Default)]
    struct Context {
        components: Vec<ComponentId>,
    }
    #[derive(Serialize, Deserialize)]
    enum ComponentId {
        Position,
        Velocity,
    }

    #[derive(Serialize, Deserialize)]
    /// Bodge into serde_test's very strict interface
    struct SerWorld(#[serde(with = "helpers")] World);

    impl PartialEq for SerWorld {
        fn eq(&self, other: &Self) -> bool {
            fn same_components<T: Component + PartialEq>(x: &EntityRef, y: &EntityRef) -> bool {
                x.get::<T>().as_ref().map(|x| &**x) == y.get::<T>().as_ref().map(|x| &**x)
            }

            for (x, y) in self.0.iter().zip(other.0.iter()) {
                if x.entity() != y.entity()
                    || !same_components::<Position>(&x, &y)
                    || !same_components::<Velocity>(&x, &y)
                {
                    return false;
                }
            }
            true
        }
    }

    impl fmt::Debug for SerWorld {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_map()
                .entries(self.0.iter().map(|e| {
                    (
                        e.entity(),
                        (
                            e.get::<Position>().map(|x| *x),
                            e.get::<Velocity>().map(|x| *x),
                        ),
                    )
                }))
                .finish()
        }
    }

    mod helpers {
        use super::*;
        pub fn serialize<S: Serializer>(x: &World, s: S) -> Result<S::Ok, S::Error> {
            crate::serialize::column::serialize(
                x,
                &mut Context {
                    components: Vec::new(),
                },
                s,
            )
        }
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<World, D::Error> {
            crate::serialize::column::deserialize(
                &mut Context {
                    components: Vec::new(),
                },
                d,
            )
        }
    }

    impl DeserializeContext for Context {
        fn deserialize_component_ids<'de, A>(
            &mut self,
            mut seq: A,
        ) -> Result<ColumnBatchType, A::Error>
        where
            A: SeqAccess<'de>,
        {
            self.components.clear();
            let mut batch = ColumnBatchType::new();
            while let Some(id) = seq.next_element()? {
                match id {
                    ComponentId::Position => {
                        batch.add::<Position>();
                    }
                    ComponentId::Velocity => {
                        batch.add::<Velocity>();
                    }
                }
                self.components.push(id);
            }
            Ok(batch)
        }

        fn deserialize_components<'de, A>(
            &mut self,
            entity_count: u32,
            mut seq: A,
            batch: &mut ColumnBatchBuilder,
        ) -> Result<(), A::Error>
        where
            A: SeqAccess<'de>,
        {
            for component in &self.components {
                match *component {
                    ComponentId::Position => {
                        deserialize_column::<Position, _>(entity_count, &mut seq, batch)?;
                    }
                    ComponentId::Velocity => {
                        deserialize_column::<Velocity, _>(entity_count, &mut seq, batch)?;
                    }
                }
            }
            Ok(())
        }
    }

    impl SerializeContext for Context {
        fn component_count(&self, archetype: &Archetype) -> usize {
            archetype.component_types().len()
        }

        fn serialize_component_ids<S: SerializeTuple>(
            &mut self,
            archetype: &Archetype,
            mut out: S,
        ) -> Result<S::Ok, S::Error> {
            try_serialize_id::<Position, _, _>(archetype, &ComponentId::Position, &mut out)?;
            try_serialize_id::<Velocity, _, _>(archetype, &ComponentId::Velocity, &mut out)?;
            out.end()
        }

        fn serialize_components<S: SerializeTuple>(
            &mut self,
            archetype: &Archetype,
            mut out: S,
        ) -> Result<S::Ok, S::Error> {
            try_serialize::<Position, _>(archetype, &mut out)?;
            try_serialize::<Velocity, _>(archetype, &mut out)?;
            out.end()
        }
    }

    #[test]
    #[rustfmt::skip]
    fn roundtrip() {
        use serde_test::{Token, assert_tokens};

        let mut world = World::new();
        let p0 = Position([0.0, 0.0, 0.0]);
        let v0 = Velocity([1.0, 1.0, 1.0]);
        let p1 = Position([2.0, 2.0, 2.0]);
        let e0 = world.spawn((p0, v0));
        let e1 = world.spawn((p1,));
        let e2 = world.spawn(());

        assert_tokens(&SerWorld(world), &[
            Token::NewtypeStruct { name: "SerWorld" },
            Token::Seq { len: Some(3) },

            Token::Tuple { len: 4 },
            Token::U32(1),
            Token::U32(0),
            Token::Tuple { len: 0 },
            Token::TupleEnd,
            Token::Tuple { len: 1 },
            Token::Tuple { len: 1 },
            Token::U64(e2.to_bits().into()),
            Token::TupleEnd,
            Token::TupleEnd,
            Token::TupleEnd,

            Token::Tuple { len: 4 },
            Token::U32(1),
            Token::U32(2),
            Token::Tuple { len: 2 },
            Token::UnitVariant { name: "ComponentId", variant: "Position" },
            Token::UnitVariant { name: "ComponentId", variant: "Velocity" },
            Token::TupleEnd,
            Token::Tuple { len: 3 },
            Token::Tuple { len: 1 },
            Token::U64(e0.to_bits().into()),
            Token::TupleEnd,
            Token::Tuple { len: 1 },
            Token::NewtypeStruct { name: "Position" },
            Token::Tuple { len: 3 },
            Token::F32(0.0),
            Token::F32(0.0),
            Token::F32(0.0),
            Token::TupleEnd,
            Token::TupleEnd,
            Token::Tuple { len: 1 },
            Token::NewtypeStruct { name: "Velocity" },
            Token::Tuple { len: 3 },
            Token::F32(1.0),
            Token::F32(1.0),
            Token::F32(1.0),
            Token::TupleEnd,
            Token::TupleEnd,
            Token::TupleEnd,
            Token::TupleEnd,

            Token::Tuple { len: 4 },
            Token::U32(1),
            Token::U32(1),
            Token::Tuple { len: 1 },
            Token::UnitVariant { name: "ComponentId", variant: "Position" },
            Token::TupleEnd,
            Token::Tuple { len: 2 },
            Token::Tuple { len: 1 },
            Token::U64(e1.to_bits().into()),
            Token::TupleEnd,
            Token::Tuple { len: 1 },
            Token::NewtypeStruct { name: "Position" },
            Token::Tuple { len: 3 },
            Token::F32(2.0),
            Token::F32(2.0),
            Token::F32(2.0),
            Token::TupleEnd,
            Token::TupleEnd,
            Token::TupleEnd,
            Token::TupleEnd,

            Token::SeqEnd,
        ])
    }
}
