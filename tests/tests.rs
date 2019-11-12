use simplecs::*;

#[test]
fn random_access() {
    let mut world = World::new();
    let e = world.spawn(("abc", 123));
    let f = world.spawn(("def", 456));
    assert_eq!(world.get::<&'static str>(e), Some(&"abc"));
    assert_eq!(world.get::<i32>(e), Some(&123));
    assert_eq!(world.get::<&'static str>(f), Some(&"def"));
    assert_eq!(world.get::<i32>(f), Some(&456));
    *world.get_mut::<i32>(f).unwrap() = 42;
    assert_eq!(world.get::<i32>(f), Some(&42));
}

#[test]
fn query_all() {
    let mut world = World::new();
    world.spawn(("abc", 123));
    world.spawn(("def", 456));
    let ents = world.iter::<(&i32, &&'static str)>().collect::<Vec<_>>();
    assert_eq!(ents.len(), 2);
    assert!(ents.contains(&(&123, &"abc")));
    assert!(ents.contains(&(&456, &"def")));
}

#[test]
fn query_single_component() {
    let mut world = World::new();
    world.spawn(("abc", 123));
    world.spawn(("def", 456));
    let ents = world.iter::<&i32>().collect::<Vec<_>>();
    assert_eq!(ents.len(), 2);
    assert!(ents.contains(&&123));
    assert!(ents.contains(&&456));
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
fn query_optional_component() {
    let mut world = World::new();
    world.spawn(("abc", 123));
    world.spawn(("def", 456, true));
    let ents = world.iter::<(Option<&bool>, &i32)>().collect::<Vec<_>>();
    assert_eq!(ents.len(), 2);
    assert!(ents.contains(&(None, &123)));
    assert!(ents.contains(&(Some(&true), &456)));
}
