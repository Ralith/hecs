//! This example demonstrates using the [ColumnBatch][hecs::ColumnBatch] API to efficiently clone
//! the entities in a [World] along with some or all components.
//!
//! By using [`World::set_freelist()`], we ensure that the cloned world will allocate new entities
//! in sync with the original world: newly allocated entities will have the same entity ids as the
//! original world.
//!
//! Iteration order of entities in the cloned world will be the same as the original world, but this
//! is only incidentally the case and not necessarily guaranteed by the API in use here; see
//! comments in the implementation of [`WorldCloner::clone_world()`] for more details.

use hecs::{Archetype, ColumnBatchBuilder, ColumnBatchType, Component, TypeIdMap, TypeInfo, World};
use std::any::TypeId;

struct ComponentCloneMetadata {
    type_info: TypeInfo,
    insert_into_batch_func: &'static dyn Fn(&Archetype, &mut ColumnBatchBuilder),
}

/// Clones world entities along with registered components when [Self::clone_world()] is called.
///
/// Unregistered components are omitted from the cloned world, but entities containing unregistered
/// components will still be cloned (even if they result in "empty" entities).
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
        let freelist = world.freelist().collect::<Vec<_>>();

        let mut cloned = World::new();

        // Copy over archetypes by invoking the stored cloning function for each component type.
        //
        // Iteration order of the cloned world is (incidentally) preserved here because iterating
        // through the world (or iterating through the results of a query on the world) iterates
        // through all archetypes in the order in which they were added to the world - so to keep
        // iteration order the same, we rely on `archetypes()` returning archetypes in the order
        // they were added to the world, and we make sure to add those to the cloned world in that
        // same order again.
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

            let handles = archetype
                .ids()
                .iter()
                .map(|id| unsafe { world.find_entity_from_id(*id) })
                .collect::<Vec<_>>();

            let batch = batch_builder.build().expect("batch should be complete");
            cloned.spawn_column_batch_at(&handles, batch);
        }

        // make sure that the cloned world allocates new entities in sync with the original world
        cloned.set_freelist(&freelist);

        cloned
    }
}

pub fn main() {
    let entity0_data = (0i32, "Ada".to_owned());
    let entity1_data = (1i32,);
    let entity2_data = (2i32, "Cob".to_owned());
    let entity3_data = (3i32,);
    let entity4_data = (4i32, 4u8); // we don't register u8 so it won't be cloned

    let mut world0 = World::new();
    let entity0 = world0.spawn(entity0_data.clone());
    let entity1 = world0.spawn(entity1_data.clone());
    let entity2 = world0.spawn(entity2_data.clone());
    let _entity3 = world0.spawn(entity3_data.clone());
    let entity4 = world0.spawn(entity4_data.clone());

    world0.despawn(entity1).unwrap();
    world0.despawn(_entity3).unwrap();
    let _entity3 = world0.spawn(entity3_data.clone());

    let mut cloner = WorldCloner::default();
    cloner.register::<i32>();
    cloner.register::<String>();

    let mut world1 = cloner.clone_world(&world0);

    assert_eq!(
        world0.len(),
        world1.len(),
        "cloned world should have same entity count as original world"
    );

    // check iteration order is the same via iterating over all entities
    for (original_entity, cloned_entity) in world0.iter().zip(world1.iter()) {
        assert_eq!(
            original_entity.entity(),
            cloned_entity.entity(),
            "entity iteration order should be the same"
        );
        let original_i32 = original_entity.get::<&i32>().unwrap();
        let cloned_i32 = cloned_entity.get::<&i32>().unwrap();
        assert_eq!(*original_i32, *cloned_i32, "i32 component should be cloned");
    }

    // check iteration order is the same via a query
    let mut query = world0.query::<&i32>();
    let mut cloned_query = world1.query::<&i32>();
    for ((original_entity, original_i32), (cloned_entity, cloned_i32)) in
        query.iter().zip(cloned_query.iter())
    {
        assert_eq!(
            original_entity, cloned_entity,
            "query entity iteration order should be the same"
        );
        assert_eq!(
            *original_i32, *cloned_i32,
            "query i32 components should be the same"
        );
    }
    drop(query);
    drop(cloned_query);

    // spawn something to make sure that entity ids allocated are the same
    let entity1 = world0.spawn(entity1_data.clone());
    assert_eq!(
        entity1,
        world1.spawn(entity1_data.clone()),
        "newly spawned entity should have same id in both worlds"
    );

    // despawn and re-spawn to make sure that entity ids allocated are the same
    world0.despawn(entity0).unwrap();
    world1.despawn(entity0).unwrap();
    let entity0 = world0.spawn(entity0_data.clone());
    assert_eq!(
        entity0,
        world1.spawn(entity0_data.clone()),
        "despawned and re-spawned entity should have same id in both worlds"
    );

    // make sure that entity 2 has had its string cloned as expected
    let original_string = world0.get::<&String>(entity2).unwrap();
    let cloned_string = world1.get::<&String>(entity2).unwrap();
    assert_eq!(
        *original_string, *cloned_string,
        "string component should be cloned"
    );

    // but the u8 component shouldn't have been cloned
    assert!(
        world1.contains(entity4),
        "entity 4 should be in the cloned world despite containing only unregistered components"
    );
    assert!(
        world1.get::<&u8>(entity4).is_err(),
        "entity 4's u8 component should not be cloned because it was not registered"
    );
}
