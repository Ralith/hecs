//! Human-friendly row-major serialization
//!
//! Stores each entity's components together. Preferred for data that will be read/written by
//! humans. Less efficient than column-major serialization.
//!
//! This module builds on the public [`World::iter()`] and [`World::spawn_at()`] APIs, and are
//! somewhat opinionated. For some applications, a custom approach may be preferable.
//!
//! In terms of the serde data model, we treat a [`World`] as a map of entity IDs to user-controlled
//! maps of component IDs to data.

use core::{cell::RefCell, fmt};

use serde::{
    de::{DeserializeSeed, MapAccess, Visitor},
    ser::SerializeMap,
    Deserializer, Serialize, Serializer,
};

use crate::{Component, EntityBuilder, EntityRef, World};

/// Implements serialization of individual entities
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
/// use hecs::{*, serialize::row::*};
///
/// #[derive(Serialize, Deserialize)]
/// enum ComponentId { Position, Velocity }
///
/// // Could include references to external state for use by `serialize_entity`
/// struct Context;
///
/// impl SerializeContext for Context {
///     fn serialize_entity<S>(
///         &mut self,
///         entity: EntityRef<'_>,
///         mut map: S,
///     ) -> Result<S::Ok, S::Error>
///     where
///         S: serde::ser::SerializeMap,
///     {
///         // Call `try_serialize` for every serializable component we want to save
///         try_serialize::<Position, _, _>(&entity, &ComponentId::Position, &mut map)?;
///         try_serialize::<Velocity, _, _>(&entity, &ComponentId::Velocity, &mut map)?;
///         // Or do something custom for more complex cases.
///         map.end()
///     }
/// }
/// ```
pub trait SerializeContext {
    /// Serialize a single entity into a map
    fn serialize_entity<S>(&mut self, entity: EntityRef<'_>, map: S) -> Result<S::Ok, S::Error>
    where
        S: SerializeMap;

    /// Number of entries that [`serialize_entry`](Self::serialize_entity) will produce for
    /// `entity`, if known
    ///
    /// Defaults to `None`. Must be overridden to return `Some` to support certain serializers, e.g.
    /// bincode.
    fn component_count(&self, entity: EntityRef<'_>) -> Option<usize> {
        let _ = entity;
        None
    }
}

/// If `entity` has component `T`, serialize it under `key` in `map`
///
/// Convenience method for [`SerializeContext`] implementations.
pub fn try_serialize<T: Component + Serialize, K: Serialize + ?Sized, S: SerializeMap>(
    entity: &EntityRef<'_>,
    key: &K,
    map: &mut S,
) -> Result<(), S::Error> {
    if let Some(x) = entity.get::<&T>() {
        map.serialize_key(key)?;
        map.serialize_value(&*x)?;
    }
    Ok(())
}

/// Serialize a [`World`] through a [`SerializeContext`] to a [`Serializer`]
pub fn serialize<C, S>(world: &World, context: &mut C, serializer: S) -> Result<S::Ok, S::Error>
where
    C: SerializeContext,
    S: Serializer,
{
    let mut seq = serializer.serialize_map(Some(world.len() as usize))?;
    for entity in world {
        seq.serialize_key(&entity.entity())?;
        seq.serialize_value(&SerializeComponents(RefCell::new((context, Some(entity)))))?;
    }
    seq.end()
}

struct SerializeComponents<'a, C>(RefCell<(&'a mut C, Option<EntityRef<'a>>)>);

impl<'a, C: SerializeContext> Serialize for SerializeComponents<'a, C> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut this = self.0.borrow_mut();
        let entity = this.1.take().unwrap();
        let map = serializer.serialize_map(this.0.component_count(entity))?;
        this.0.serialize_entity(entity, map)
    }
}

/// Deserialize a [`World`] with a [`DeserializeContext`] and a [`Deserializer`]
pub fn deserialize<'de, C, D>(context: &mut C, deserializer: D) -> Result<World, D::Error>
where
    C: DeserializeContext,
    D: Deserializer<'de>,
{
    deserializer.deserialize_map(WorldVisitor(context))
}

/// Implements deserialization of entities from a serde [`MapAccess`] into an [`EntityBuilder`]
///
/// Data external to the [`World`] can be populated during deserialization by storing mutable
/// references inside the struct implementing this trait.
///
/// # Example
/// ```
/// # use serde::{Serialize, Deserialize};
/// # #[derive(Deserialize)]
/// # struct Position([f32; 3]);
/// # #[derive(Deserialize)]
/// # struct Velocity([f32; 3]);
/// use hecs::{*, serialize::row::*};
///
/// #[derive(Serialize, Deserialize)]
/// enum ComponentId { Position, Velocity }
///
/// // Could include references to external state for use by `deserialize_entity`
/// struct Context;
///
/// impl DeserializeContext for Context {
///     fn deserialize_entity<'de, M>(
///         &mut self,
///         mut map: M,
///         entity: &mut EntityBuilder,
///     ) -> Result<(), M::Error>
///     where
///         M: serde::de::MapAccess<'de>,
///     {
///         while let Some(key) = map.next_key()? {
///             match key {
///                 ComponentId::Position => {
///                     entity.add::<Position>(map.next_value()?);
///                 }
///                 ComponentId::Velocity => {
///                     entity.add::<Velocity>(map.next_value()?);
///                 }
///             }
///         }
///         Ok(())
///     }
/// }
/// ```
pub trait DeserializeContext {
    /// Deserialize a single entity
    fn deserialize_entity<'de, M>(
        &mut self,
        map: M,
        entity: &mut EntityBuilder,
    ) -> Result<(), M::Error>
    where
        M: MapAccess<'de>;
}

struct WorldVisitor<'a, C>(&'a mut C);

impl<'de, 'a, C> Visitor<'de> for WorldVisitor<'a, C>
where
    C: DeserializeContext,
{
    type Value = World;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a world")
    }

    fn visit_map<A>(self, mut map: A) -> Result<World, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut world = World::new();
        let mut builder = EntityBuilder::new();
        while let Some(id) = map.next_key()? {
            map.next_value_seed(DeserializeComponents(self.0, &mut builder))?;
            world.spawn_at(id, builder.build());
        }
        Ok(world)
    }
}

struct DeserializeComponents<'a, C>(&'a mut C, &'a mut EntityBuilder);

impl<'de, 'a, C> DeserializeSeed<'de> for DeserializeComponents<'a, C>
where
    C: DeserializeContext,
{
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<(), D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(ComponentsVisitor(self.0, self.1))
    }
}

struct ComponentsVisitor<'a, C>(&'a mut C, &'a mut EntityBuilder);

impl<'de, 'a, C> Visitor<'de> for ComponentsVisitor<'a, C>
where
    C: DeserializeContext,
{
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("an entity's components")
    }

    fn visit_map<A>(self, map: A) -> Result<(), A::Error>
    where
        A: MapAccess<'de>,
    {
        self.0.deserialize_entity(map, self.1)
    }
}

#[cfg(test)]
mod tests {
    use std::fmt;

    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::*;

    #[derive(Serialize, Deserialize, PartialEq, Debug, Copy, Clone)]
    struct Position([f32; 3]);
    #[derive(Serialize, Deserialize, PartialEq, Debug, Copy, Clone)]
    struct Velocity([f32; 3]);

    struct Context;
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
                x.get::<&T>().as_ref().map(|x| &**x) == y.get::<&T>().as_ref().map(|x| &**x)
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
                            e.get::<&Position>().map(|x| *x),
                            e.get::<&Velocity>().map(|x| *x),
                        ),
                    )
                }))
                .finish()
        }
    }

    mod helpers {
        use super::*;
        pub fn serialize<S: Serializer>(x: &World, s: S) -> Result<S::Ok, S::Error> {
            crate::serialize::row::serialize(x, &mut Context, s)
        }
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<World, D::Error> {
            crate::serialize::row::deserialize(&mut Context, d)
        }
    }

    impl DeserializeContext for Context {
        fn deserialize_entity<'de, M>(
            &mut self,
            mut map: M,
            entity: &mut EntityBuilder,
        ) -> Result<(), M::Error>
        where
            M: serde::de::MapAccess<'de>,
        {
            while let Some(key) = map.next_key()? {
                match key {
                    ComponentId::Position => {
                        entity.add::<Position>(map.next_value()?);
                    }
                    ComponentId::Velocity => {
                        entity.add::<Velocity>(map.next_value()?);
                    }
                }
            }
            Ok(())
        }
    }

    impl SerializeContext for Context {
        fn serialize_entity<S>(
            &mut self,
            entity: EntityRef<'_>,
            mut map: S,
        ) -> Result<S::Ok, S::Error>
        where
            S: serde::ser::SerializeMap,
        {
            try_serialize::<Position, _, _>(&entity, &ComponentId::Position, &mut map)?;
            try_serialize::<Velocity, _, _>(&entity, &ComponentId::Velocity, &mut map)?;
            map.end()
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

        assert_tokens(&SerWorld(world), &[
            Token::NewtypeStruct { name: "SerWorld" },
            Token::Map { len: Some(2) },

            Token::U64(e0.to_bits().into()),
            Token::Map { len: None },

            Token::UnitVariant { name: "ComponentId", variant: "Position" },
            Token::NewtypeStruct { name: "Position" },
            Token::Tuple { len: 3 },
            Token::F32(0.0),
            Token::F32(0.0),
            Token::F32(0.0),
            Token::TupleEnd,

            Token::UnitVariant { name: "ComponentId", variant: "Velocity" },
            Token::NewtypeStruct { name: "Velocity" },
            Token::Tuple { len: 3 },
            Token::F32(1.0),
            Token::F32(1.0),
            Token::F32(1.0),
            Token::TupleEnd,

            Token::MapEnd,

            Token::U64(e1.to_bits().into()),
            Token::Map { len: None },

            Token::UnitVariant { name: "ComponentId", variant: "Position" },
            Token::NewtypeStruct { name: "Position" },
            Token::Tuple { len: 3 },
            Token::F32(2.0),
            Token::F32(2.0),
            Token::F32(2.0),
            Token::TupleEnd,

            Token::MapEnd,

            Token::MapEnd,
        ])
    }
}
