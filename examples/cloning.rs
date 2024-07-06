use hecs::{ColumnBatchBuilder, ColumnBatchType, Component, World};

fn maybe_add_type_to_batch<T: Component>(
    archetype: &hecs::Archetype,
    batch_type: &mut ColumnBatchType,
) {
    if archetype.has::<T>() {
        batch_type.add::<T>();
    }
}

fn maybe_clone_column<T: Component + Clone>(
    archetype: &hecs::Archetype,
    batch: &mut ColumnBatchBuilder,
) {
    if let Some((column, mut writer)) = archetype.get::<&T>().zip(batch.writer::<T>()) {
        for item in column.iter() {
            if let Err(_) = writer.push(item.clone()) {
                unreachable!("push should always succeed since batch was sized to match archetype");
            }
        }
    }
}

/// Clones world entities along with a hardcoded list of components.
///
/// Components not included are omitted from the cloned world.
///
/// Note that entity allocator state may differ in the cloned world - so for example a new entity
/// spawned in each world may end up with different entity ids, entity iteration order may be
/// different, etc.
fn clone_world(world: &World) -> World {
    let mut cloned = World::new();

    for archetype in world.archetypes() {
        let mut batch = ColumnBatchType::new();
        // types have to be listed one by one here
        maybe_add_type_to_batch::<String>(archetype, &mut batch);
        maybe_add_type_to_batch::<i32>(archetype, &mut batch);

        let mut batch = batch.into_batch(archetype.ids().len() as u32);
        // and types need to be listed again here
        maybe_clone_column::<String>(archetype, &mut batch);
        maybe_clone_column::<i32>(archetype, &mut batch);

        let batch = batch.build().expect("batch should be complete");

        let handles = &cloned
            .reserve_entities(archetype.ids().len() as u32)
            .collect::<Vec<_>>();
        cloned.flush();
        cloned.spawn_column_batch_at(handles, batch);
    }

    cloned
}

pub fn main() {
    let int0 = 0;
    let int1 = 1;
    let str0 = "Ada".to_owned();
    let str1 = "Bob".to_owned();

    let mut world0 = World::new();
    let entity0 = world0.spawn((int0, str0));
    let entity1 = world0.spawn((int1, str1));

    let world1 = clone_world(&world0);

    assert_eq!(
        world0.len(),
        world1.len(),
        "cloned world should have same entity count as original world"
    );

    type AllComponentsQuery = (&'static i32, &'static String);

    for entity in [entity0, entity1] {
        let w0_e = world0.entity(entity).expect("w0 entity should exist");
        let w1_e = world1.entity(entity).expect("w1 entity should exist");
        assert!(w0_e.satisfies::<AllComponentsQuery>());
        assert!(w1_e.satisfies::<AllComponentsQuery>());

        assert_eq!(
            w0_e.query::<AllComponentsQuery>().get().unwrap(),
            w1_e.query::<AllComponentsQuery>().get().unwrap()
        );
    }
}
