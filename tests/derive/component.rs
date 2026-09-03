use hecs::Component;

#[derive(Component)]
struct Unit;

#[derive(Component, Debug, PartialEq)]
struct Tuple(i32);

#[derive(Component)]
struct Named {
    x: u64,
}

#[derive(Component)]
enum Either {
    A,
    B(String),
}

#[derive(Component)]
struct Bounded<T>(T);

#[derive(Component)]
struct WhereClause<T>(T)
where
    T: Send + Sync + 'static;

fn main() {
    let mut world = hecs::World::new();
    let e = world.spawn((
        Unit,
        Tuple(1),
        Named { x: 2 },
        Either::B(String::from("hi")),
        Bounded(3u8),
        WhereClause(4.0f64),
    ));
    assert_eq!(*world.get::<&Tuple>(e).unwrap(), Tuple(1));
    assert_eq!(world.query_one_mut::<&Named>(e).unwrap().x, 2);
}
