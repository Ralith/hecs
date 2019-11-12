use simplecs::*;

#[test]
fn spawn_and_get() {
    let mut world = World::new();
    let e = world.spawn(("abc", 123));
    let f = world.spawn(("def", 456));
    assert_eq!(world.get::<&'static str>(e), Some(&"abc"));
    assert_eq!(world.get::<i32>(e), Some(&123));
    assert_eq!(world.get::<&'static str>(f), Some(&"def"));
    assert_eq!(world.get::<i32>(f), Some(&456));
}

