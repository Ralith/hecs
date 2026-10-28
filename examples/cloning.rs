//! This example demonstrates using the [ColumnBatch][hecs::ColumnBatch] API to efficiently clone
//! the entities in a [World] along with some or all components.
//!
//! Note that the cloned world may have different iteration order and/or newly created entity ids
//! may diverge between the original and newly created worlds. If that is a dealbreaker for you,
//! see https://github.com/Ralith/hecs/issues/332 for some pointers on preserving entity allocator
//! state; as of time of writing, you'll need to patch `hecs`.

use std::any::TypeId;

use hecs::{Archetype, ColumnBatchBuilder, ColumnBatchType, Component, TypeIdMap, TypeInfo, World};

#[derive(Clone, Debug, PartialEq)]
struct Count(i32);
impl Component for Count {}

#[derive(Clone, Debug, PartialEq)]
struct Name(String);
impl Component for Name {}

/// A component that will not be registered with the [`WorldCloner`]
#[derive(Clone, Debug, PartialEq)]
struct Marker(u8);
impl Component for Marker {}

struct ComponentCloneMetadata {
    type_info: TypeInfo,
    insert_into_batch_func: &'static dyn Fn(&Archetype, &mut ColumnBatchBuilder),
}

/// Clones world entities along with registered components when [Self::clone_world()] is called.
///
/// Unregistered components are omitted from the cloned world. Entities containing unregistered
/// components will still be cloned.
///
/// Note that entity allocator state may differ in the cloned world - so for example a new entity
/// spawned in each world may end up with different entity ids, entity iteration order may be
/// different, etc.
#[derive(Default)]
struct WorldCloner {
    registry: TypeIdMap<ComponentCloneMetadata>,
}

impl WorldCloner {
    pub fn register<T: Component + Clone>(&mut self) {
        self.registry.insert(
            TypeId::of::<T>(),
            ComponentCloneMetadata {
                type_info: TypeInfo::of::<T>(),
                insert_into_batch_func: &|src, dest| {
                    let mut column = dest.writer::<T>().unwrap();
                    for component in &*src.get::<&T>().unwrap() {
                        _ = column.push(component.clone());
                    }
                },
            },
        );
    }

    fn clone_world(&self, world: &World) -> World {
        let mut cloned = World::new();

        for archetype in world.archetypes() {
            let mut batch_type = ColumnBatchType::new();
            for (&type_id, clone_metadata) in self.registry.iter() {
                if archetype.has_dynamic(type_id) {
                    batch_type.add_dynamic(clone_metadata.type_info);
                }
            }

            let mut batch_builder = batch_type.into_batch(archetype.ids().len() as u32);
            for (&type_id, clone_metadata) in self.registry.iter() {
                if archetype.has_dynamic(type_id) {
                    (clone_metadata.insert_into_batch_func)(archetype, &mut batch_builder)
                }
            }

            let batch = batch_builder.build().expect("batch should be complete");
            let handles = &cloned
                .reserve_entities(archetype.ids().len() as u32)
                .collect::<Vec<_>>();
            cloned.flush();
            cloned.spawn_column_batch_at(handles, batch);
        }

        cloned
    }
}

pub fn main() {
    let mut world0 = World::new();
    let entity0 = world0.spawn((Count(0), Name("Ada".to_owned())));
    let entity1 = world0.spawn((Count(1), Name("Bob".to_owned())));
    let entity2 = world0.spawn((Name("Cal".to_owned()),));
    let entity3 = world0.spawn((Marker(0),)); // unregistered component

    let mut cloner = WorldCloner::default();
    cloner.register::<Count>();
    cloner.register::<Name>();

    let world1 = cloner.clone_world(&world0);

    assert_eq!(
        world0.len(),
        world1.len(),
        "cloned world should have same entity count as original world"
    );

    // NB: unregistered components don't get cloned
    assert!(
        world0
            .entity(entity3)
            .expect("w0 entity3 should exist")
            .has::<Marker>(),
        "original world entity has Marker component"
    );
    assert!(
        !world1
            .entity(entity3)
            .expect("w1 entity3 should exist")
            .has::<Marker>(),
        "cloned world entity does not have Marker component because it was not registered"
    );

    type AllRegisteredComponentsQuery = (&'static Count, &'static Name);
    for entity in [entity0, entity1] {
        let w0_e = world0.entity(entity).expect("w0 entity should exist");
        let w1_e = world1.entity(entity).expect("w1 entity should exist");
        assert!(w0_e.satisfies::<AllRegisteredComponentsQuery>());
        assert!(w1_e.satisfies::<AllRegisteredComponentsQuery>());

        assert_eq!(
            w0_e.query::<AllRegisteredComponentsQuery>().get().unwrap(),
            w1_e.query::<AllRegisteredComponentsQuery>().get().unwrap()
        );
    }

    type SomeRegisteredComponentsQuery = (&'static Name,);
    let w0_e = world0.entity(entity2).expect("w0 entity2 should exist");
    let w1_e = world1.entity(entity2).expect("w1 entity2 should exist");
    assert!(w0_e.satisfies::<SomeRegisteredComponentsQuery>());
    assert!(w1_e.satisfies::<SomeRegisteredComponentsQuery>());

    assert_eq!(
        w0_e.query::<SomeRegisteredComponentsQuery>().get().unwrap(),
        w1_e.query::<SomeRegisteredComponentsQuery>().get().unwrap()
    );
}
