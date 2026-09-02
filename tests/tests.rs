#![allow(deprecated)]

use std::borrow::Cow;

use hecs::*;

// Component types used throughout these tests
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Int(i32);
impl Component for Int {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Count(usize);
impl Component for Count {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Bool(bool);
impl Component for Bool {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Char(char);
impl Component for Char {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Str(&'static str);
impl Component for Str {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Bytes([u8; 1024]);
impl Component for Bytes {}

#[derive(Clone, Debug, PartialEq)]
struct Label(String);
impl Component for Label {}

#[derive(Clone, Debug, PartialEq)]
struct Text(Cow<'static, str>);
impl Component for Text {}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Float(f64);
impl Component for Float {}

#[derive(Clone, Copy, Debug, PartialEq)]
struct F32(f32);
impl Component for F32 {}

#[test]
fn random_access() {
    let mut world = World::new();
    let e = world.spawn((Str("abc"), Int(123)));
    let f = world.spawn((Str("def"), Int(456), Bool(true)));
    assert_eq!(world.get::<&Str>(e).unwrap().0, "abc");
    assert_eq!(world.get::<&Int>(e).unwrap().0, 123);
    assert_eq!(world.get::<&Str>(f).unwrap().0, "def");
    assert_eq!(world.get::<&Int>(f).unwrap().0, 456);
    world.get::<&mut Int>(f).unwrap().0 = 42;
    assert_eq!(world.get::<&Int>(f).unwrap().0, 42);
}

#[test]
fn despawn() {
    let mut world = World::new();
    let e = world.spawn((Str("abc"), Int(123)));
    let f = world.spawn((Str("def"), Int(456)));
    assert_eq!(world.query::<()>().iter().count(), 2);
    world.despawn(e).unwrap();
    assert_eq!(world.query::<()>().iter().count(), 1);
    assert!(world.get::<&Str>(e).is_err());
    assert!(world.get::<&Int>(e).is_err());
    assert_eq!(world.get::<&Str>(f).unwrap().0, "def");
    assert_eq!(world.get::<&Int>(f).unwrap().0, 456);
}

#[test]
fn query_all() {
    let mut world = World::new();
    let e = world.spawn((Str("abc"), Int(123)));
    let f = world.spawn((Str("def"), Int(456)));

    let ents = world
        .query::<(Entity, &Int, &Str)>()
        .iter()
        .map(|(e, i, s)| (e, *i, *s))
        .collect::<Vec<_>>();
    assert_eq!(ents.len(), 2);
    assert!(ents.contains(&(e, Int(123), Str("abc"))));
    assert!(ents.contains(&(f, Int(456), Str("def"))));

    let ents = world.query::<Entity>().iter().collect::<Vec<_>>();
    assert_eq!(ents.len(), 2);
    assert!(ents.contains(&e));
    assert!(ents.contains(&f));
}

#[test]
#[cfg(feature = "macros")]
fn derived_query() {
    #[derive(Query, Debug, PartialEq)]
    struct Foo<'a> {
        x: &'a Int,
        y: &'a mut Bool,
    }

    let mut world = World::new();
    let e = world.spawn((Int(42), Bool(false)));
    assert_eq!(
        world.query_one_mut::<Foo>(e).unwrap(),
        Foo {
            x: &Int(42),
            y: &mut Bool(false)
        }
    );
}

#[test]
#[cfg(feature = "macros")]
fn derived_enum_query() {
    #[derive(Query, Debug, PartialEq)]
    enum Foo<'a> {
        NumberAndString(&'a Int, &'a Label),
        Number(&'a Int),
        Boolean(&'a mut Bool),
    }

    let mut world = World::new();
    let e1 = world.spawn((Int(42), Bool(false)));

    assert_eq!(
        world.query_one_mut::<Foo>(e1).unwrap(),
        Foo::Number(&Int(42))
    );

    let e2 = world.spawn((Label(String::from("Hello")), Bool(false)));

    assert_eq!(
        world.query_one_mut::<Foo>(e2).unwrap(),
        Foo::Boolean(&mut Bool(false))
    );

    let e3 = world.spawn((Label(String::from("Hello")), Int(42)));

    assert_eq!(
        world.query_one_mut::<Foo>(e3).unwrap(),
        Foo::NumberAndString(&Int(42), &Label(String::from("Hello")))
    );

    let e4 = world.spawn((Label(String::from("Hello")), Count(0)));

    assert_eq!(
        world.query_one_mut::<Foo>(e4),
        Err(QueryOneError::Unsatisfied)
    );
}

#[test]
#[cfg(feature = "macros")]
fn derived_enum_query_with_empty() {
    #[derive(Query, Debug, PartialEq)]
    enum Foo<'a> {
        Number(&'a Int),
        Empty,
        Impossible(&'a Label),
    }

    let mut world = World::new();
    let e1 = world.spawn((Int(42), Bool(false)));

    assert_eq!(
        world.query_one_mut::<Foo>(e1).unwrap(),
        Foo::Number(&Int(42))
    );

    let e2 = world.spawn((Bool(false), Count(0)));

    assert_eq!(world.query_one_mut::<Foo>(e2).unwrap(), Foo::Empty);

    let e3 = world.spawn((Label(String::from("Hello")), Bool(false)));

    assert_eq!(world.query_one_mut::<Foo>(e3).unwrap(), Foo::Empty);
}

#[test]
#[cfg(feature = "macros")]
fn derived_bundle_clone() {
    #[derive(Bundle, DynamicBundleClone)]
    struct Foo<T: Clone + Component> {
        x: Int,
        y: Bool,
        z: T,
    }

    #[derive(PartialEq, Debug, Query)]
    struct FooQuery<'a> {
        x: &'a Int,
        y: &'a Bool,
        z: &'a Label,
    }

    let mut world = World::new();
    let mut builder = EntityBuilderClone::new();
    builder.add_bundle(Foo {
        x: Int(42),
        y: Bool(false),
        z: Label(String::from("Foo")),
    });

    let entity = builder.build();
    let e = world.spawn(&entity);
    assert_eq!(
        world.query_one_mut::<FooQuery>(e).unwrap(),
        FooQuery {
            x: &Int(42),
            y: &Bool(false),
            z: &Label(String::from("Foo")),
        }
    );
}

#[test]
fn query_single_component() {
    let mut world = World::new();
    let e = world.spawn((Str("abc"), Int(123)));
    let f = world.spawn((Str("def"), Int(456), Bool(true)));
    let ents = world
        .query::<(Entity, &Int)>()
        .iter()
        .map(|(e, i)| (e, *i))
        .collect::<Vec<_>>();
    assert_eq!(ents.len(), 2);
    assert!(ents.contains(&(e, Int(123))));
    assert!(ents.contains(&(f, Int(456))));
}

#[test]
fn query_missing_component() {
    let mut world = World::new();
    world.spawn((Str("abc"), Int(123)));
    world.spawn((Str("def"), Int(456)));
    assert!(world.query::<(&Bool, &Int)>().iter().next().is_none());
}

#[test]
fn query_sparse_component() {
    let mut world = World::new();
    world.spawn((Str("abc"), Int(123)));
    let f = world.spawn((Str("def"), Int(456), Bool(true)));
    let ents = world
        .query::<(Entity, &Bool)>()
        .iter()
        .map(|(e, b)| (e, *b))
        .collect::<Vec<_>>();
    assert_eq!(ents, &[(f, Bool(true))]);
}

#[test]
fn query_optional_component() {
    let mut world = World::new();
    let e = world.spawn((Str("abc"), Int(123)));
    let f = world.spawn((Str("def"), Int(456), Bool(true)));
    let ents = world
        .query::<(Entity, Option<&Bool>, &Int)>()
        .iter()
        .map(|(e, b, i)| (e, b.copied(), *i))
        .collect::<Vec<_>>();
    assert_eq!(ents.len(), 2);
    assert!(ents.contains(&(e, None, Int(123))));
    assert!(ents.contains(&(f, Some(Bool(true)), Int(456))));
}

#[test]
fn prepare_query() {
    let mut world = World::new();
    let e = world.spawn((Str("abc"), Int(123)));
    let f = world.spawn((Str("def"), Int(456)));

    let mut query = PreparedQuery::<(Entity, &Int, &Str)>::default();

    let ents = query
        .query(&world)
        .iter()
        .map(|(e, i, s)| (e, *i, *s))
        .collect::<Vec<_>>();
    assert_eq!(ents.len(), 2);
    assert!(ents.contains(&(e, Int(123), Str("abc"))));
    assert!(ents.contains(&(f, Int(456), Str("def"))));

    let ents = query
        .query_mut(&mut world)
        .map(|(e, i, s)| (e, *i, *s))
        .collect::<Vec<_>>();
    assert_eq!(ents.len(), 2);
    assert!(ents.contains(&(e, Int(123), Str("abc"))));
    assert!(ents.contains(&(f, Int(456), Str("def"))));
}

#[test]
fn invalidate_prepared_query() {
    let mut world = World::new();
    let e = world.spawn((Str("abc"), Int(123)));
    let f = world.spawn((Str("def"), Int(456)));

    let mut query = PreparedQuery::<(Entity, &Int, &Str)>::default();

    let ents = query
        .query(&world)
        .iter()
        .map(|(e, i, s)| (e, *i, *s))
        .collect::<Vec<_>>();
    assert_eq!(ents.len(), 2);
    assert!(ents.contains(&(e, Int(123), Str("abc"))));
    assert!(ents.contains(&(f, Int(456), Str("def"))));

    world.spawn((Bool(true),));
    let g = world.spawn((Str("ghi"), Int(789)));

    let ents = query
        .query_mut(&mut world)
        .map(|(e, i, s)| (e, *i, *s))
        .collect::<Vec<_>>();
    assert_eq!(ents.len(), 3);
    assert!(ents.contains(&(e, Int(123), Str("abc"))));
    assert!(ents.contains(&(f, Int(456), Str("def"))));
    assert!(ents.contains(&(g, Int(789), Str("ghi"))));
}

#[test]
fn random_access_via_view() {
    let mut world = World::new();
    let e = world.spawn((Str("abc"), Int(123)));
    let f = world.spawn((Str("def"),));

    let mut query = PreparedQuery::<(&Int, &Str)>::default();
    let mut query = query.query(&world);
    let mut view = query.view();

    let (i, s) = view.get(e).unwrap();
    assert_eq!(*i, Int(123));
    assert_eq!(*s, Str("abc"));

    assert!(view.get_mut(f).is_none());
}

#[test]
fn random_access_via_view_mut() {
    let mut world = World::new();
    let e = world.spawn((Str("abc"), Int(123)));
    let f = world.spawn((Str("def"),));

    let mut query = PreparedQuery::<(&Int, &Str)>::default();
    let mut view = query.view_mut(&mut world);

    let (i, s) = view.get(e).unwrap();
    assert_eq!(*i, Int(123));
    assert_eq!(*s, Str("abc"));

    assert!(view.get_mut(f).is_none());

    assert!(view.contains(e));
    assert!(!view.contains(f));
}

#[test]
fn view_borrow_on_world() {
    let mut world = World::new();
    let e0 = world.spawn((Int(3), Str("hello")));
    let e1 = world.spawn((Float(6.0), Str("world")));
    let e2 = world.spawn((Int(12),));

    {
        let str_view = world.view::<&Str>();

        assert_eq!(str_view.get(e0).unwrap().0, "hello");
        assert_eq!(str_view.get(e1).unwrap().0, "world");
        assert_eq!(str_view.get(e2), None);
    }

    {
        let mut int_view = world.view::<&mut Int>();
        assert_eq!(int_view.get_mut(e0).unwrap().0, 3);
        assert_eq!(int_view.get_mut(e1), None);
        assert_eq!(int_view.get_mut(e2).unwrap().0, 12);

        // edit some value
        int_view.get_mut(e0).unwrap().0 = 100;
    }

    {
        let mut int_str_view = world.view::<(&Str, &mut Int)>();
        let (s, i) = int_str_view.get_mut(e0).unwrap();
        assert_eq!(s.0, "hello");
        assert_eq!(i.0, 100);
        assert_eq!(int_str_view.get_mut(e1), None);
        assert_eq!(int_str_view.get_mut(e2), None);
    }
}

#[test]
fn view_mut_on_world() {
    let mut world = World::new();
    let e0 = world.spawn((Int(3), Str("hello")));
    let e1 = world.spawn((Float(6.0), Str("world")));
    let e2 = world.spawn((Int(12),));

    let str_view = world.view_mut::<&Str>();
    assert_eq!(str_view.get(e0).unwrap().0, "hello");
    assert_eq!(str_view.get(e1).unwrap().0, "world");
    assert_eq!(str_view.get(e2), None);

    let mut int_view = world.view_mut::<&mut Int>();
    assert_eq!(int_view.get_mut(e0).unwrap().0, 3);
    assert_eq!(int_view.get_mut(e1), None);
    assert_eq!(int_view.get_mut(e2).unwrap().0, 12);

    // edit some value
    int_view.get_mut(e0).unwrap().0 = 100;

    let mut int_str_view = world.view_mut::<(&Str, &mut Int)>();
    let (s, i) = int_str_view.get_mut(e0).unwrap();
    assert_eq!(s.0, "hello");
    assert_eq!(i.0, 100);
    assert_eq!(int_str_view.get_mut(e1), None);
    assert_eq!(int_str_view.get_mut(e2), None);
}

#[should_panic]
#[test]
fn view_mut_panic() {
    let mut world = World::new();
    let e = world.spawn((Char('a'),));

    // we should panic since we have two overlapping views:
    let mut first_view = world.view::<&mut Char>();
    let mut second_view = world.view::<&mut Char>();

    first_view.get_mut(e).unwrap();
    second_view.get_mut(e).unwrap();
}

#[test]
#[should_panic]
fn simultaneous_access_must_be_non_overlapping() {
    let mut world = World::new();
    let a = world.spawn((Int(1),));
    let b = world.spawn((Int(2),));
    let c = world.spawn((Int(3),));
    let d = world.spawn((Int(4),));

    let mut query = world.query_mut::<&mut Int>();
    let mut view = query.view();

    view.get_disjoint_mut([a, d, c, b, a]);
}

#[test]
fn build_entity() {
    let mut world = World::new();
    let mut entity = EntityBuilder::new();
    entity.add(Str("abc"));
    entity.add(Int(123));
    let e = world.spawn(entity.build());
    entity.add(Str("def"));
    entity.add(Bytes([0u8; 1024]));
    entity.add(Int(456));
    entity.add(Int(789));
    let f = world.spawn(entity.build());
    assert_eq!(world.get::<&Str>(e).unwrap().0, "abc");
    assert_eq!(world.get::<&Int>(e).unwrap().0, 123);
    assert_eq!(world.get::<&Str>(f).unwrap().0, "def");
    assert_eq!(world.get::<&Int>(f).unwrap().0, 789);
}

#[test]
fn build_entity_clone() {
    let mut world = World::new();
    let mut entity = EntityBuilderClone::new();
    entity.add(Str("def"));
    entity.add(Bytes([0u8; 1024]));
    entity.add(Int(456));
    entity.add(Int(789));
    entity.add_bundle((Str("yup"), Count(67)));
    entity.add_bundle((F32(5.0), Label(String::from("Foo"))));
    entity.add_bundle((F32(7.0), Label(String::from("Bar")), Count(42)));
    let entity = entity.build();
    let e = world.spawn(&entity);
    let f = world.spawn(&entity);
    let g = world.spawn(&entity);
    world
        .insert_one(g, Text(Cow::<'static, str>::from("after")))
        .unwrap();

    for e in [e, f, g] {
        assert_eq!(world.get::<&Str>(e).unwrap().0, "yup");
        assert_eq!(world.get::<&Int>(e).unwrap().0, 789);
        assert_eq!(world.get::<&Count>(e).unwrap().0, 42);
        assert_eq!(world.get::<&F32>(e).unwrap().0, 7.0);
        assert_eq!(world.get::<&Label>(e).unwrap().0, "Bar");
    }

    assert_eq!(world.get::<&Text>(g).unwrap().0, "after");
}

#[test]
fn build_builder_clone() {
    let mut a = EntityBuilderClone::new();
    a.add(Label(String::from("abc")));
    a.add(Int(123));
    let mut b = EntityBuilderClone::new();
    b.add(Label(String::from("def")));
    b.add_bundle(&a.build());
    assert_eq!(b.get::<&Label>(), Some(&Label(String::from("abc"))));
    assert_eq!(b.get::<&Int>(), Some(&Int(123)));
}

#[test]
#[allow(clippy::redundant_clone)]
fn cloned_builder() {
    let mut builder = EntityBuilderClone::new();
    builder.add(Label(String::from("abc"))).add(Int(123));

    let mut world = World::new();
    let e = world.spawn(&builder.build().clone());
    assert_eq!(world.get::<&Label>(e).unwrap().0, "abc");
    assert_eq!(world.get::<&Int>(e).unwrap().0, 123);
}

#[test]
#[cfg(feature = "macros")]
fn build_dynamic_bundle() {
    #[derive(Bundle, DynamicBundleClone)]
    struct Foo {
        x: Int,
        y: Char,
    }

    let mut world = World::new();
    let mut entity = EntityBuilderClone::new();
    entity.add_bundle(Foo {
        x: Int(5),
        y: Char('c'),
    });
    entity.add_bundle((Label(String::from("Bar")), F32(6.0)));
    entity.add(Char('a'));
    let entity = entity.build();
    let e = world.spawn(&entity);
    let f = world.spawn(&entity);
    let g = world.spawn(&entity);

    world
        .insert_one(g, Text(Cow::<'static, str>::from("after")))
        .unwrap();

    for e in [e, f, g] {
        assert_eq!(world.get::<&Int>(e).unwrap().0, 5);
        assert_eq!(world.get::<&Char>(e).unwrap().0, 'a');
        assert_eq!(world.get::<&Label>(e).unwrap().0, "Bar");
        assert_eq!(world.get::<&F32>(e).unwrap().0, 6.0);
    }

    assert_eq!(world.get::<&Text>(g).unwrap().0, "after");
}

#[test]
fn access_builder_components() {
    let mut world = World::new();
    let mut entity = EntityBuilder::new();

    entity.add(Str("abc"));
    entity.add(Int(123));

    assert!(entity.has::<Str>());
    assert!(entity.has::<Int>());
    assert!(!entity.has::<Count>());

    assert_eq!(entity.get::<&Str>().unwrap().0, "abc");
    assert_eq!(entity.get::<&Int>().unwrap().0, 123);
    assert_eq!(entity.get::<&Count>(), None);

    entity.get_mut::<&mut Int>().unwrap().0 = 456;
    assert_eq!(entity.get::<&Int>().unwrap().0, 456);

    let g = world.spawn(entity.build());

    assert_eq!(world.get::<&Str>(g).unwrap().0, "abc");
    assert_eq!(world.get::<&Int>(g).unwrap().0, 456);
}

#[test]
fn build_entity_bundle() {
    let mut world = World::new();
    let mut entity = EntityBuilder::new();
    entity.add_bundle((Str("abc"), Int(123)));
    let e = world.spawn(entity.build());
    entity.add(Int(456));
    entity.add_bundle((Str("def"), Bytes([0u8; 1024]), Int(789)));
    let f = world.spawn(entity.build());
    assert_eq!(world.get::<&Str>(e).unwrap().0, "abc");
    assert_eq!(world.get::<&Int>(e).unwrap().0, 123);
    assert_eq!(world.get::<&Str>(f).unwrap().0, "def");
    assert_eq!(world.get::<&Int>(f).unwrap().0, 789);
}

#[test]
fn dynamic_components() {
    let mut world = World::new();
    let e = world.spawn((Int(42),));
    world.insert(e, (Bool(true), Str("abc"))).unwrap();
    assert_eq!(
        world
            .query::<(Entity, &Int, &Bool)>()
            .iter()
            .map(|(e, i, b)| (e, *i, *b))
            .collect::<Vec<_>>(),
        &[(e, Int(42), Bool(true))]
    );
    assert_eq!(world.remove_one::<Int>(e), Ok(Int(42)));
    assert_eq!(
        world
            .query::<(Entity, &Int, &Bool)>()
            .iter()
            .map(|(e, i, b)| (e, *i, *b))
            .collect::<Vec<_>>(),
        &[]
    );
    assert_eq!(
        world
            .query::<(Entity, &Bool, &Str)>()
            .iter()
            .map(|(e, b, s)| (e, *b, *s))
            .collect::<Vec<_>>(),
        &[(e, Bool(true), Str("abc"))]
    );
}

#[test]
fn spawn_buffered_entity() {
    let mut world = World::new();
    let mut buffer = CommandBuffer::new();
    let ent = world.reserve_entity();
    let ent1 = world.reserve_entity();
    let ent2 = world.reserve_entity();
    let ent3 = world.reserve_entity();

    buffer.insert(ent, (Int(1), Bool(true)));
    buffer.insert(ent1, (Int(13), Float(7.11), Str("hecs")));
    buffer.insert(ent2, (Int(17), Bool(false), Char('o')));
    buffer.insert(ent3, (Int(2), Str("qwe"), Float(101.103), Bool(false)));

    buffer.run_on(&mut world);

    assert!(world.get::<&Bool>(ent).unwrap().0);
    assert!(!world.get::<&Bool>(ent2).unwrap().0);

    assert_eq!(world.get::<&Str>(ent1).unwrap().0, "hecs");
    assert_eq!(world.get::<&Int>(ent1).unwrap().0, 13);
    assert_eq!(world.get::<&Int>(ent3).unwrap().0, 2);
}

#[test]
fn despawn_buffered_entity() {
    let mut world = World::new();
    let mut buffer = CommandBuffer::new();
    let ent = world.spawn((Int(1), Bool(true)));
    buffer.despawn(ent);

    buffer.run_on(&mut world);
    assert!(!world.contains(ent));
}

#[test]
fn remove_buffered_component() {
    let mut world = World::new();
    let mut buffer = CommandBuffer::new();
    let ent = world.spawn((Int(7), Bool(true), Str("hecs")));

    buffer.remove::<(Int, Str)>(ent);
    buffer.run_on(&mut world);

    assert!(world.get::<&Str>(ent).is_err());
    assert!(world.get::<&Int>(ent).is_err());
}

#[test]
#[should_panic(expected = "already borrowed")]
fn illegal_borrow() {
    let mut world = World::new();
    world.spawn((Str("abc"), Int(123)));
    world.spawn((Str("def"), Int(456)));

    world.query::<(&mut Int, &Int)>().iter();
}

#[test]
#[should_panic(expected = "already borrowed")]
fn illegal_borrow_2() {
    let mut world = World::new();
    world.spawn((Str("abc"), Int(123)));
    world.spawn((Str("def"), Int(456)));

    world.query::<(&mut Int, &mut Int)>().iter();
}

#[test]
#[should_panic(expected = "query violates a unique borrow")]
fn illegal_query_mut_borrow() {
    let mut world = World::new();
    world.spawn((Str("abc"), Int(123)));
    world.spawn((Str("def"), Int(456)));

    world.query_mut::<(&Int, &mut Int)>();
}

#[test]
#[should_panic(expected = "query violates a unique borrow")]
fn illegal_query_one_borrow() {
    let mut world = World::new();
    let entity = world.spawn((Str("abc"), Int(123)));

    world.query_one::<(&mut Int, &Int)>(entity).get().unwrap();
}

#[test]
#[should_panic(expected = "query violates a unique borrow")]
fn illegal_query_one_borrow_2() {
    let mut world = World::new();
    let entity = world.spawn((Str("abc"), Int(123)));

    world
        .query_one::<(&mut Int, &mut Int)>(entity)
        .get()
        .unwrap();
}

#[test]
#[should_panic(expected = "query violates a unique borrow")]
fn illegal_query_one_mut_borrow() {
    let mut world = World::new();
    let entity = world.spawn((Str("abc"), Int(123)));

    world.query_one_mut::<(&mut Int, &Int)>(entity).unwrap();
}

#[test]
#[should_panic(expected = "query violates a unique borrow")]
fn illegal_query_one_mut_borrow_2() {
    let mut world = World::new();
    let entity = world.spawn((Str("abc"), Int(123)));

    world.query_one_mut::<(&mut Int, &mut Int)>(entity).unwrap();
}

#[test]
fn disjoint_queries() {
    let mut world = World::new();
    world.spawn((Str("abc"), Bool(true)));
    world.spawn((Str("def"), Int(456)));

    let _a = world.query::<(&mut Str, &Bool)>();
    let _b = world.query::<(&mut Str, &Int)>();
}

#[test]
fn shared_borrow() {
    let mut world = World::new();
    world.spawn((Str("abc"), Int(123)));
    world.spawn((Str("def"), Int(456)));

    world.query::<(&Int, &Int)>();
}

#[test]
#[should_panic(expected = "already borrowed")]
fn illegal_random_access() {
    let mut world = World::new();
    let e = world.spawn((Str("abc"), Int(123)));
    let _borrow = world.get::<&mut Int>(e).unwrap();
    world.get::<&Int>(e).unwrap();
}

#[test]
#[cfg(feature = "macros")]
fn derived_bundle() {
    #[derive(Bundle)]
    struct Foo {
        x: Int,
        y: Char,
    }

    let mut world = World::new();
    let e = world.spawn(Foo {
        x: Int(42),
        y: Char('a'),
    });
    assert_eq!(world.get::<&Int>(e).unwrap().0, 42);
    assert_eq!(world.get::<&Char>(e).unwrap().0, 'a');
}

#[test]
#[cfg(feature = "macros")]
#[cfg_attr(
    debug_assertions,
    should_panic(
        expected = "attempted to allocate entity with duplicate tests::Int components; \
                    each type must occur at most once!"
    )
)]
#[cfg_attr(
    not(debug_assertions),
    should_panic(expected = "attempted to allocate entity with duplicate components; \
                    each type must occur at most once!")
)]
fn bad_bundle_derive() {
    #[derive(Bundle)]
    struct Foo {
        x: Int,
        y: Int,
    }

    let mut world = World::new();
    world.spawn(Foo {
        x: Int(42),
        y: Int(42),
    });
}

#[test]
#[cfg_attr(miri, ignore)]
fn spawn_many() {
    let mut world = World::new();
    const N: usize = 100_000;
    for _ in 0..N {
        world.spawn((Int(42),));
    }
    assert_eq!(world.iter().count(), N);
}

#[test]
fn clear() {
    let mut world = World::new();
    world.spawn((Str("abc"), Int(123)));
    world.spawn((Str("def"), Int(456), Bool(true)));
    world.clear();
    assert_eq!(world.iter().count(), 0);
}

#[test]
fn remove_missing() {
    let mut world = World::new();
    let e = world.spawn((Str("abc"), Int(123)));
    assert!(world.remove_one::<Bool>(e).is_err());
}

#[test]
fn exchange_components() {
    let mut world = World::new();

    let entity = world.spawn((Label("abc".to_owned()), Int(123)));
    assert!(world.get::<&Label>(entity).is_ok());
    assert!(world.get::<&Int>(entity).is_ok());
    assert!(world.get::<&Bool>(entity).is_err());

    world.exchange_one::<Label, _>(entity, Bool(true)).unwrap();
    assert!(world.get::<&Label>(entity).is_err());
    assert!(world.get::<&Int>(entity).is_ok());
    assert!(world.get::<&Bool>(entity).is_ok());
}

#[test]
fn reserve() {
    let mut world = World::new();
    let a = world.reserve_entity();
    let b = world.reserve_entity();

    assert_eq!(world.query::<()>().iter().count(), 0);

    world.flush();

    let entities = world.query::<Entity>().iter().collect::<Vec<_>>();

    assert_eq!(entities.len(), 2);
    assert!(entities.contains(&a));
    assert!(entities.contains(&b));
}

#[test]
fn query_batched() {
    let mut world = World::new();
    let a = world.spawn(());
    let b = world.spawn(());
    let c = world.spawn((Int(42),));
    assert_eq!(world.query::<()>().iter_batched(1).count(), 3);
    assert_eq!(world.query::<()>().iter_batched(2).count(), 2);
    assert_eq!(world.query::<()>().iter_batched(2).flatten().count(), 3);
    // different archetypes are always in different batches
    assert_eq!(world.query::<()>().iter_batched(3).count(), 2);
    assert_eq!(world.query::<()>().iter_batched(3).flatten().count(), 3);
    assert_eq!(world.query::<()>().iter_batched(4).count(), 2);
    let entities = world
        .query::<Entity>()
        .iter_batched(1)
        .flatten()
        .collect::<Vec<_>>();
    assert_eq!(entities.len(), 3);
    assert!(entities.contains(&a));
    assert!(entities.contains(&b));
    assert!(entities.contains(&c));

    // Batched queries filter like usual
    assert_eq!(
        world
            .query::<(Entity, &Int)>()
            .iter_batched(1)
            .flatten()
            .collect::<Vec<_>>(),
        &[(c, &Int(42))]
    );
}

#[test]
fn query_mut_batched() {
    let mut world = World::new();
    let a = world.spawn(());
    let b = world.spawn(());
    let c = world.spawn((Int(42),));
    assert_eq!(world.query_mut::<()>().into_iter_batched(1).count(), 3);
    assert_eq!(world.query_mut::<()>().into_iter_batched(2).count(), 2);
    assert_eq!(
        world
            .query_mut::<()>()
            .into_iter_batched(2)
            .flatten()
            .count(),
        3
    );
    // different archetypes are always in different batches
    assert_eq!(world.query_mut::<()>().into_iter_batched(3).count(), 2);
    assert_eq!(
        world
            .query_mut::<()>()
            .into_iter_batched(3)
            .flatten()
            .count(),
        3
    );
    assert_eq!(world.query_mut::<()>().into_iter_batched(4).count(), 2);
    let entities = world
        .query_mut::<Entity>()
        .into_iter_batched(1)
        .flatten()
        .collect::<Vec<_>>();
    assert_eq!(entities.len(), 3);
    assert!(entities.contains(&a));
    assert!(entities.contains(&b));
    assert!(entities.contains(&c));
}

#[test]
fn spawn_batch() {
    let mut world = World::new();
    world.spawn_batch((0..10).map(|x| (Int(x), Str("abc"))));
    let entity_count = world.query::<&Int>().iter().count();
    assert_eq!(entity_count, 10);
}

#[test]
fn query_one() {
    let mut world = World::new();
    let a = world.spawn((Str("abc"), Int(123)));
    let b = world.spawn((Str("def"), Int(456)));
    let c = world.spawn((Str("ghi"), Int(789), Bool(true)));
    assert_eq!(world.query_one::<&Int>(a).get(), Ok(&Int(123)));
    assert_eq!(world.query_one::<&Int>(b).get(), Ok(&Int(456)));
    assert_eq!(
        world.query_one::<(&Int, &Bool)>(a).get(),
        Err(QueryOneError::Unsatisfied)
    );
    assert_eq!(
        world.query_one::<(&Int, &Bool)>(c).get(),
        Ok((&Int(789), &Bool(true)))
    );
    world.despawn(a).unwrap();
    assert_eq!(
        world.query_one::<&Int>(a).get(),
        Err(QueryOneError::NoSuchEntity)
    );
}

#[test]
#[cfg_attr(
    debug_assertions,
    should_panic(
        expected = "attempted to allocate entity with duplicate tests::F32 components; \
                    each type must occur at most once!"
    )
)]
#[cfg_attr(
    not(debug_assertions),
    should_panic(expected = "attempted to allocate entity with duplicate components; \
                    each type must occur at most once!")
)]
fn duplicate_components_panic() {
    let mut world = World::new();
    world.reserve::<(F32, Int, F32)>(1);
}

#[test]
fn spawn_column_batch() {
    let mut world = World::new();
    let mut batch_ty = ColumnBatchType::new();
    batch_ty.add::<Int>().add::<Bool>();

    // Unique archetype
    let b;
    {
        let batch = batch_ty.clone().into_batch(2);
        {
            let mut bs = batch.writer::<Bool>().unwrap();
            bs.push(Bool(true)).unwrap();
            bs.push(Bool(false)).unwrap();
            let mut is = batch.writer::<Int>().unwrap();
            is.push(Int(42)).unwrap();
            is.push(Int(43)).unwrap();
        }
        let entities = world
            .spawn_column_batch(batch.build().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(entities.len(), 2);
        assert_eq!(
            world.query_one_mut::<(&Int, &Bool)>(entities[0]).unwrap(),
            (&Int(42), &Bool(true))
        );
        assert_eq!(
            world.query_one_mut::<(&Int, &Bool)>(entities[1]).unwrap(),
            (&Int(43), &Bool(false))
        );
        world.despawn(entities[0]).unwrap();
        b = entities[1];
    }

    // Duplicate archetype
    {
        let batch = batch_ty.clone().into_batch(2);
        {
            let mut bs = batch.writer::<Bool>().unwrap();
            bs.push(Bool(true)).unwrap();
            bs.push(Bool(false)).unwrap();
            let mut is = batch.writer::<Int>().unwrap();
            is.push(Int(44)).unwrap();
            is.push(Int(45)).unwrap();
        }
        let entities = world
            .spawn_column_batch(batch.build().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(entities.len(), 2);
        assert_eq!(world.get::<&Int>(b).unwrap().0, 43);
        assert_eq!(world.get::<&Int>(entities[0]).unwrap().0, 44);
        assert_eq!(world.get::<&Int>(entities[1]).unwrap().0, 45);
    }
}

#[test]
fn columnar_access() {
    let mut world = World::new();
    let e = world.spawn((Str("abc"), Int(123)));
    let f = world.spawn((Str("def"), Int(456), Bool(true)));
    let g = world.spawn((Str("ghi"), Int(789), Bool(false)));
    let mut archetypes = world.archetypes();
    let _empty = archetypes.next().unwrap();
    let a = archetypes.next().unwrap();
    assert_eq!(a.ids(), &[e.id()]);
    assert_eq!(*a.get::<&Int>().unwrap(), [Int(123)]);
    assert!(a.get::<&Bool>().is_none());
    let b = archetypes.next().unwrap();
    assert_eq!(b.ids(), &[f.id(), g.id()]);
    assert_eq!(*b.get::<&Int>().unwrap(), [Int(456), Int(789)]);
}

#[test]
fn empty_entity_ref() {
    let mut world = World::new();
    let e = world.spawn(());
    let r = world.entity(e).unwrap();
    assert_eq!(r.entity(), e);
}

#[test]
fn query_or() {
    let mut world = World::new();
    let e = world.spawn((Str("abc"), Int(123)));
    let _ = world.spawn((Str("def"),));
    let f = world.spawn((Str("ghi"), Bool(true)));
    let g = world.spawn((Str("jkl"), Int(456), Bool(false)));
    let results = world
        .query::<(Entity, &Str, Or<&Int, &Bool>)>()
        .iter()
        .map(|(handle, s, value)| (handle, *s, value.cloned()))
        .collect::<Vec<_>>();
    assert_eq!(results.len(), 3);
    assert!(results.contains(&(e, Str("abc"), Or::Left(Int(123)))));
    assert!(results.contains(&(f, Str("ghi"), Or::Right(Bool(true)))));
    assert!(results.contains(&(g, Str("jkl"), Or::Both(Int(456), Bool(false)))));
}

#[test]
fn len() {
    let mut world = World::new();
    let ent = world.spawn(());
    world.spawn(());
    world.spawn(());
    assert_eq!(world.len(), 3);
    world.despawn(ent).unwrap();
    assert_eq!(world.len(), 2);
    world.clear();
    assert_eq!(world.len(), 0);
}

#[test]
fn take() {
    let mut world_a = World::new();
    let e = world_a.spawn((Label("abc".to_string()), Int(42)));
    let f = world_a.spawn((Label("def".to_string()), Int(17)));
    let mut world_b = World::new();
    let e2 = world_b.spawn(world_a.take(e).unwrap());
    assert!(!world_a.contains(e));
    assert_eq!(world_b.get::<&Label>(e2).unwrap().0, "abc");
    assert_eq!(world_b.get::<&Int>(e2).unwrap().0, 42);
    assert_eq!(world_a.get::<&Label>(f).unwrap().0, "def");
    assert_eq!(world_a.get::<&Int>(f).unwrap().0, 17);
    world_b.take(e2).unwrap();
    assert!(!world_b.contains(e2));
}

#[test]
fn empty_archetype_conflict() {
    let mut world = World::new();
    let _ = world.spawn((Int(42), Bool(true)));
    let _ = world.spawn((Int(17), Str("abc")));
    let e = world.spawn((Int(12), Bool(false), Str("def")));
    world.despawn(e).unwrap();
    for _ in world.query::<(&mut Int, &Str)>().iter() {
        for _ in world.query::<(&mut Int, &Bool)>().iter() {}
    }
}

#[test]
fn component_ref_map() {
    struct TestComponent {
        id: i32,
    }
    impl Component for TestComponent {}

    let mut world = World::new();
    let e = world.spawn((TestComponent { id: 21 },));

    let e_ref = world.entity(e).unwrap();
    {
        let comp = e_ref.get::<&'_ TestComponent>().unwrap();
        // Test that no unbalanced releases occur when cloning refs.
        let _comp2 = comp.clone();
        let id = Ref::map(comp, |c| &c.id);
        assert_eq!(*id, 21);
    }

    {
        let comp = e_ref.get::<&'_ mut TestComponent>().unwrap();
        let mut id = RefMut::map(comp, |c| &mut c.id);
        *id = 31;
    }

    {
        let comp = e_ref.get::<&'_ TestComponent>().unwrap();
        let id = Ref::map(comp, |c| &c.id);
        assert_eq!(*id, 31);
    }
}

#[test]
fn query_many() {
    let mut world = World::new();
    let a = world.spawn((Int(42), Bool(true)));
    let b = world.spawn((Int(17),));
    assert_eq!(
        world.query_many_mut::<&Int, 2>([a, b]),
        [Ok(&Int(42)), Ok(&Int(17))]
    );
}

#[test]
#[should_panic]
fn query_many_duplicate() {
    let mut world = World::new();
    let e = world.spawn(());
    _ = world.query_many_mut::<(), 2>([e, e]);
}

#[test]
fn cache_invalidation() {
    let mut world = World::new();
    assert_eq!(
        world.query::<(Entity, &Int)>().iter().collect::<Vec<_>>(),
        []
    );
    let a = world.spawn((Int(42), Bool(true)));
    let b = world.spawn((Int(17),));
    assert_eq!(
        world.query::<(Entity, &Int)>().iter().collect::<Vec<_>>(),
        &[(a, &Int(42)), (b, &Int(17))]
    );
}

// https://github.com/Ralith/hecs/issues/417
#[test]
fn entity_generation_regression() {
    struct C;
    impl Component for C {}

    let mut world = World::new();

    let a = world.spawn((C,));
    let b = world.spawn((C,));
    world.despawn(a).unwrap();
    world.despawn(b).unwrap();
    let c = world.spawn((C,));
    world.despawn(c).unwrap();
    let d = world.spawn((C,));
    let d2 = world.query::<Entity>().iter().next().unwrap();
    assert_eq!(d, d2);
}
