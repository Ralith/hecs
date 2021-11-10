use std::any::TypeId;

use hecs::{DynamicQueryTypes, Entity, World};

#[test]
fn dynamic_query() {
    let mut world = World::new();

    let entity1 = world.spawn((123, "abc", 4.0));
    let entity2 = world.spawn((500, "aaa", 4.5));
    let entity3 = world.spawn((124, "abd", 6.0, vec![10]));
    let entity4 = world.spawn(("one",));

    let read_types = [TypeId::of::<i32>(), TypeId::of::<&'static str>()];
    let write_types = [TypeId::of::<f64>()];
    let types = DynamicQueryTypes::new(&read_types, &write_types);

    let query = world.query_dynamic(types);
    let entities: Vec<Entity> = query.iter_entities().collect();
    assert_eq!(entities.len(), 3);
    assert!(entities.contains(&entity1));
    assert!(entities.contains(&entity2));
    assert!(entities.contains(&entity3));
    assert!(!entities.contains(&entity4));

    let i32s: Vec<i32> = query
        .iter_component_slices(TypeId::of::<i32>())
        .flat_map(|components| components.as_slice().to_vec())
        .collect();
    let strings: Vec<&'static str> = query
        .iter_component_slices(TypeId::of::<&'static str>())
        .flat_map(|components| components.as_slice().to_vec())
        .collect();

    for (i, entity) in entities.into_iter().enumerate() {
        let i32 = i32s[i];
        let string = strings[i];
        let (expected_i32, expected_string) = if entity == entity1 {
            (123, "abc")
        } else if entity == entity2 {
            (500, "aaa")
        } else if entity == entity3 {
            (124, "abd")
        } else {
            unreachable!()
        };

        assert_eq!(i32, expected_i32);
        assert_eq!(string, expected_string);
    }
}
