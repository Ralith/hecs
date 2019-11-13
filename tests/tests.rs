use simplecs::*;

#[test]
fn random_access() {
    let mut world = World::new();
    let e = world.spawn(("abc", 123));
    let f = world.spawn(("def", 456, true));
    assert_eq!(*world.get::<&'static str>(e).unwrap(), "abc");
    assert_eq!(*world.get::<i32>(e).unwrap(), 123);
    assert_eq!(*world.get::<&'static str>(f).unwrap(), "def");
    assert_eq!(*world.get::<i32>(f).unwrap(), 456);
    *world.get_mut::<i32>(f).unwrap() = 42;
    assert_eq!(*world.get::<i32>(f).unwrap(), 42);
}

#[test]
fn despawn() {
    let mut world = World::new();
    let e = world.spawn(("abc", 123));
    let f = world.spawn(("def", 456));
    world.despawn(e).unwrap();
    assert!(world.get::<&'static str>(e).is_err());
    assert!(world.get::<i32>(e).is_err());
    assert_eq!(*world.get::<&'static str>(f).unwrap(), "def");
    assert_eq!(*world.get::<i32>(f).unwrap(), 456);
}

#[test]
fn query_all() {
    let mut world = World::new();
    let e = world.spawn(("abc", 123));
    let f = world.spawn(("def", 456));

    let ents = world.iter::<(&i32, &&'static str)>().collect::<Vec<_>>();
    assert_eq!(ents.len(), 2);
    assert!(ents.contains(&(e, (&123, &"abc"))));
    assert!(ents.contains(&(f, (&456, &"def"))));

    let ents = world.iter::<()>().collect::<Vec<_>>();
    assert_eq!(ents.len(), 2);
    assert!(ents.contains(&(e, ())));
    assert!(ents.contains(&(f, ())));
}

#[test]
fn query_single_component() {
    let mut world = World::new();
    let e = world.spawn(("abc", 123));
    let f = world.spawn(("def", 456, true));
    let ents = world.iter::<&i32>().collect::<Vec<_>>();
    assert_eq!(ents.len(), 2);
    assert!(ents.contains(&(e, &123)));
    assert!(ents.contains(&(f, &456)));
}

#[test]
fn query_missing_component() {
    let mut world = World::new();
    world.spawn(("abc", 123));
    world.spawn(("def", 456));
    let ents = world.iter::<(&bool, &i32)>().collect::<Vec<_>>();
    assert_eq!(ents.len(), 0);
}

#[test]
fn query_sparse_component() {
    let mut world = World::new();
    world.spawn(("abc", 123));
    let f = world.spawn(("def", 456, true));
    let ents = world.iter::<(&bool)>().collect::<Vec<_>>();
    assert_eq!(ents, &[(f, &true)]);
}

#[test]
fn query_optional_component() {
    let mut world = World::new();
    let e = world.spawn(("abc", 123));
    let f = world.spawn(("def", 456, true));
    let ents = world.iter::<(Option<&bool>, &i32)>().collect::<Vec<_>>();
    assert_eq!(ents.len(), 2);
    assert!(ents.contains(&(e, (None, &123))));
    assert!(ents.contains(&(f, (Some(&true), &456))));
}

#[test]
fn build_entity() {
    let mut world = World::new();
    let mut entity = EntityBuilder::new();
    entity.with("abc").with(123);
    let e = world.spawn(entity.build());
    assert_eq!(*world.get::<&'static str>(e).unwrap(), "abc");
    assert_eq!(*world.get::<i32>(e).unwrap(), 123);
}

#[test]
fn dynamic_components() {
    let mut world = World::new();
    let e = world.spawn((42,));
    world.insert(e, true).unwrap();
    assert_eq!(
        world.iter::<(&i32, &bool)>().collect::<Vec<_>>(),
        &[(e, (&42, &true))]
    );
    assert_eq!(world.remove::<i32>(e), Ok(42));
    assert_eq!(world.iter::<(&i32, &bool)>().collect::<Vec<_>>(), &[]);
    assert_eq!(world.iter::<&bool>().collect::<Vec<_>>(), &[(e, &true)]);
}

#[test]
#[should_panic(expected = "component type already borrowed")]
fn illegal_borrow() {
    let mut world = World::new();
    world.spawn(("abc", 123));
    world.spawn(("def", 456));

    world.iter::<(&mut i32, &i32)>();
}

#[test]
#[should_panic(expected = "component type already borrowed")]
fn illegal_borrow_2() {
    let mut world = World::new();
    world.spawn(("abc", 123));
    world.spawn(("def", 456));

    world.iter::<(&mut i32, &mut i32)>();
}

#[test]
fn shared_borrow() {
    let mut world = World::new();
    world.spawn(("abc", 123));
    world.spawn(("def", 456));

    world.iter::<(&i32, &i32)>();
}

#[test]
#[should_panic(expected = "component type already borrowed")]
fn illegal_random_access() {
    let mut world = World::new();
    let e = world.spawn(("abc", 123));
    let _borrow = world.get_mut::<i32>(e).unwrap();
    world.get::<i32>(e).unwrap();
}
