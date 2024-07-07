use std::any::TypeId;

use hecs::{Archetype, ColumnBatchBuilder, ColumnBatchType, Component, TypeIdMap, TypeInfo, World};

struct ComponentCloneMetadata {
    type_info: TypeInfo,
    func: &'static dyn Fn(&Archetype, &mut ColumnBatchBuilder),
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
                func: &|src, dest| {
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
            let mut batch = ColumnBatchType::new();

            for (&k, v) in self.registry.iter() {
                if archetype.has_dynamic(k) {
                    batch.add_dynamic(v.type_info);
                }
            }
            let mut batch = batch.into_batch(archetype.ids().len() as u32);

            for (&k, v) in self.registry.iter() {
                if archetype.has_dynamic(k) {
                    (v.func)(archetype, &mut batch)
                }
            }

            let batch = batch.build().expect("batch should be complete");

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
    let int0 = 0;
    let int1 = 1;
    let str0 = "Ada".to_owned();
    let str1 = "Bob".to_owned();
    let str2 = "Cal".to_owned();

    let mut world0 = World::new();
    let entity0 = world0.spawn((int0, str0));
    let entity1 = world0.spawn((int1, str1));
    let entity2 = world0.spawn((str2,));
    let entity3 = world0.spawn((0u8,)); // unregistered component

    let mut cloner = WorldCloner::default();
    cloner.register::<i32>();
    cloner.register::<String>();

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
            .has::<u8>(),
        "original world entity has u8 component"
    );
    assert!(
        !world1
            .entity(entity3)
            .expect("w1 entity3 should exist")
            .has::<u8>(),
        "cloned world entity does not have u8 component because it was not registered"
    );

    type AllRegisteredComponentsQuery = (&'static i32, &'static String);
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

    type SomeRegisteredComponentsQuery = (&'static String,);
    let w0_e = world0.entity(entity2).expect("w0 entity2 should exist");
    let w1_e = world1.entity(entity2).expect("w1 entity2 should exist");
    assert!(w0_e.satisfies::<SomeRegisteredComponentsQuery>());
    assert!(w1_e.satisfies::<SomeRegisteredComponentsQuery>());

    assert_eq!(
        w0_e.query::<SomeRegisteredComponentsQuery>().get().unwrap(),
        w1_e.query::<SomeRegisteredComponentsQuery>().get().unwrap()
    );
}
