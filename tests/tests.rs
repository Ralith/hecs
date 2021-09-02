// Copyright 2019 Google LLC
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use hecs::*;

#[test]
fn random_access() {
    let mut world = World::new();
    let e = world.spawn(("abc", 123));
    let f = world.spawn(("def", 456, true));
    assert_eq!(*world.get::<&str>(e).unwrap(), "abc");
    assert_eq!(*world.get::<i32>(e).unwrap(), 123);
    assert_eq!(*world.get::<&str>(f).unwrap(), "def");
    assert_eq!(*world.get::<i32>(f).unwrap(), 456);
    *world.get_mut::<i32>(f).unwrap() = 42;
    assert_eq!(*world.get::<i32>(f).unwrap(), 42);
}

#[test]
fn despawn() {
    let mut world = World::new();
    let e = world.spawn(("abc", 123));
    let f = world.spawn(("def", 456));
    assert_eq!(world.query::<()>().iter().count(), 2);
    world.despawn(e).unwrap();
    assert_eq!(world.query::<()>().iter().count(), 1);
    assert!(world.get::<&str>(e).is_err());
    assert!(world.get::<i32>(e).is_err());
    assert_eq!(*world.get::<&str>(f).unwrap(), "def");
    assert_eq!(*world.get::<i32>(f).unwrap(), 456);
}

#[test]
fn query_all() {
    let mut world = World::new();
    let e = world.spawn(("abc", 123));
    let f = world.spawn(("def", 456));

    let ents = world
        .query::<(&i32, &&str)>()
        .iter()
        .map(|(e, (&i, &s))| (e, i, s))
        .collect::<Vec<_>>();
    assert_eq!(ents.len(), 2);
    assert!(ents.contains(&(e, 123, "abc")));
    assert!(ents.contains(&(f, 456, "def")));

    let ents = world.query::<()>().iter().collect::<Vec<_>>();
    assert_eq!(ents.len(), 2);
    assert!(ents.contains(&(e, ())));
    assert!(ents.contains(&(f, ())));
}

#[test]
#[cfg(feature = "macros")]
fn derived_query() {
    #[derive(Query, Debug, PartialEq)]
    struct Foo<'a> {
        x: &'a i32,
        y: &'a mut bool,
    }

    let mut world = World::new();
    let e = world.spawn((42, false));
    assert_eq!(
        world.query_one_mut::<Foo>(e).unwrap(),
        Foo {
            x: &42,
            y: &mut false
        }
    );
}

#[test]
fn query_single_component() {
    let mut world = World::new();
    let e = world.spawn(("abc", 123));
    let f = world.spawn(("def", 456, true));
    let ents = world
        .query::<&i32>()
        .iter()
        .map(|(e, &i)| (e, i))
        .collect::<Vec<_>>();
    assert_eq!(ents.len(), 2);
    assert!(ents.contains(&(e, 123)));
    assert!(ents.contains(&(f, 456)));
}

#[test]
fn query_missing_component() {
    let mut world = World::new();
    world.spawn(("abc", 123));
    world.spawn(("def", 456));
    assert!(world.query::<(&bool, &i32)>().iter().next().is_none());
}

#[test]
fn query_sparse_component() {
    let mut world = World::new();
    world.spawn(("abc", 123));
    let f = world.spawn(("def", 456, true));
    let ents = world
        .query::<&bool>()
        .iter()
        .map(|(e, &b)| (e, b))
        .collect::<Vec<_>>();
    assert_eq!(ents, &[(f, true)]);
}

#[test]
fn query_optional_component() {
    let mut world = World::new();
    let e = world.spawn(("abc", 123));
    let f = world.spawn(("def", 456, true));
    let ents = world
        .query::<(Option<&bool>, &i32)>()
        .iter()
        .map(|(e, (b, &i))| (e, b.copied(), i))
        .collect::<Vec<_>>();
    assert_eq!(ents.len(), 2);
    assert!(ents.contains(&(e, None, 123)));
    assert!(ents.contains(&(f, Some(true), 456)));
}

#[test]
fn prepare_query() {
    let mut world = World::new();
    let e = world.spawn(("abc", 123));
    let f = world.spawn(("def", 456));

    let mut query = PreparedQuery::<(&i32, &&str)>::default();

    let ents = query
        .query(&world)
        .iter()
        .map(|(e, (&i, &s))| (e, i, s))
        .collect::<Vec<_>>();
    assert_eq!(ents.len(), 2);
    assert!(ents.contains(&(e, 123, "abc")));
    assert!(ents.contains(&(f, 456, "def")));

    let ents = query
        .query_mut(&mut world)
        .map(|(e, (&i, &s))| (e, i, s))
        .collect::<Vec<_>>();
    assert_eq!(ents.len(), 2);
    assert!(ents.contains(&(e, 123, "abc")));
    assert!(ents.contains(&(f, 456, "def")));
}

#[test]
fn invalidate_prepared_query() {
    let mut world = World::new();
    let e = world.spawn(("abc", 123));
    let f = world.spawn(("def", 456));

    let mut query = PreparedQuery::<(&i32, &&str)>::default();

    let ents = query
        .query(&world)
        .iter()
        .map(|(e, (&i, &s))| (e, i, s))
        .collect::<Vec<_>>();
    assert_eq!(ents.len(), 2);
    assert!(ents.contains(&(e, 123, "abc")));
    assert!(ents.contains(&(f, 456, "def")));

    world.spawn((true,));
    let g = world.spawn(("ghi", 789));

    let ents = query
        .query_mut(&mut world)
        .map(|(e, (&i, &s))| (e, i, s))
        .collect::<Vec<_>>();
    assert_eq!(ents.len(), 3);
    assert!(ents.contains(&(e, 123, "abc")));
    assert!(ents.contains(&(f, 456, "def")));
    assert!(ents.contains(&(g, 789, "ghi")));
}

#[test]
fn build_entity() {
    let mut world = World::new();
    let mut entity = EntityBuilder::new();
    entity.add("abc");
    entity.add(123);
    let e = world.spawn(entity.build());
    entity.add("def");
    entity.add([0u8; 1024]);
    entity.add(456);
    entity.add(789);
    let f = world.spawn(entity.build());
    assert_eq!(*world.get::<&str>(e).unwrap(), "abc");
    assert_eq!(*world.get::<i32>(e).unwrap(), 123);
    assert_eq!(*world.get::<&str>(f).unwrap(), "def");
    assert_eq!(*world.get::<i32>(f).unwrap(), 789);
}

#[test]
fn access_builder_components() {
    let mut world = World::new();
    let mut entity = EntityBuilder::new();

    entity.add("abc");
    entity.add(123);

    assert!(entity.has::<&str>());
    assert!(entity.has::<i32>());
    assert!(!entity.has::<usize>());

    assert_eq!(*entity.get::<&str>().unwrap(), "abc");
    assert_eq!(*entity.get::<i32>().unwrap(), 123);
    assert_eq!(entity.get::<usize>(), None);

    *entity.get_mut::<i32>().unwrap() = 456;
    assert_eq!(*entity.get::<i32>().unwrap(), 456);

    let g = world.spawn(entity.build());

    assert_eq!(*world.get::<&str>(g).unwrap(), "abc");
    assert_eq!(*world.get::<i32>(g).unwrap(), 456);
}

#[test]
fn build_entity_bundle() {
    let mut world = World::new();
    let mut entity = EntityBuilder::new();
    entity.add_bundle(("abc", 123));
    let e = world.spawn(entity.build());
    entity.add(456);
    entity.add_bundle(("def", [0u8; 1024], 789));
    let f = world.spawn(entity.build());
    assert_eq!(*world.get::<&str>(e).unwrap(), "abc");
    assert_eq!(*world.get::<i32>(e).unwrap(), 123);
    assert_eq!(*world.get::<&str>(f).unwrap(), "def");
    assert_eq!(*world.get::<i32>(f).unwrap(), 789);
}

#[test]
fn dynamic_components() {
    let mut world = World::new();
    let e = world.spawn((42,));
    world.insert(e, (true, "abc")).unwrap();
    assert_eq!(
        world
            .query::<(&i32, &bool)>()
            .iter()
            .map(|(e, (&i, &b))| (e, i, b))
            .collect::<Vec<_>>(),
        &[(e, 42, true)]
    );
    assert_eq!(world.remove_one::<i32>(e), Ok(42));
    assert_eq!(
        world
            .query::<(&i32, &bool)>()
            .iter()
            .map(|(e, (&i, &b))| (e, i, b))
            .collect::<Vec<_>>(),
        &[]
    );
    assert_eq!(
        world
            .query::<(&bool, &&str)>()
            .iter()
            .map(|(e, (&b, &s))| (e, b, s))
            .collect::<Vec<_>>(),
        &[(e, true, "abc")]
    );
}

#[test]
#[should_panic(expected = "already borrowed")]
fn illegal_borrow() {
    let mut world = World::new();
    world.spawn(("abc", 123));
    world.spawn(("def", 456));

    world.query::<(&mut i32, &i32)>().iter();
}

#[test]
#[should_panic(expected = "already borrowed")]
fn illegal_borrow_2() {
    let mut world = World::new();
    world.spawn(("abc", 123));
    world.spawn(("def", 456));

    world.query::<(&mut i32, &mut i32)>().iter();
}

#[test]
#[should_panic(expected = "query violates a unique borrow")]
fn illegal_query_mut_borrow() {
    let mut world = World::new();
    world.spawn(("abc", 123));
    world.spawn(("def", 456));

    world.query_mut::<(&i32, &mut i32)>();
}

#[test]
fn disjoint_queries() {
    let mut world = World::new();
    world.spawn(("abc", true));
    world.spawn(("def", 456));

    let _a = world.query::<(&mut &str, &bool)>();
    let _b = world.query::<(&mut &str, &i32)>();
}

#[test]
fn shared_borrow() {
    let mut world = World::new();
    world.spawn(("abc", 123));
    world.spawn(("def", 456));

    world.query::<(&i32, &i32)>();
}

#[test]
#[should_panic(expected = "already borrowed")]
fn illegal_random_access() {
    let mut world = World::new();
    let e = world.spawn(("abc", 123));
    let _borrow = world.get_mut::<i32>(e).unwrap();
    world.get::<i32>(e).unwrap();
}

#[test]
#[cfg(feature = "macros")]
fn derived_bundle() {
    #[derive(Bundle)]
    struct Foo {
        x: i32,
        y: char,
    }

    let mut world = World::new();
    let e = world.spawn(Foo { x: 42, y: 'a' });
    assert_eq!(*world.get::<i32>(e).unwrap(), 42);
    assert_eq!(*world.get::<char>(e).unwrap(), 'a');
}

#[test]
#[cfg(feature = "macros")]
#[cfg_attr(
    debug_assertions,
    should_panic(
        expected = "attempted to allocate entity with duplicate i32 components; each type must occur at most once!"
    )
)]
#[cfg_attr(
    not(debug_assertions),
    should_panic(
        expected = "attempted to allocate entity with duplicate components; each type must occur at most once!"
    )
)]
fn bad_bundle_derive() {
    #[derive(Bundle)]
    struct Foo {
        x: i32,
        y: i32,
    }

    let mut world = World::new();
    world.spawn(Foo { x: 42, y: 42 });
}

#[test]
#[cfg_attr(miri, ignore)]
fn spawn_many() {
    let mut world = World::new();
    const N: usize = 100_000;
    for _ in 0..N {
        world.spawn((42u128,));
    }
    assert_eq!(world.iter().count(), N);
}

#[test]
fn clear() {
    let mut world = World::new();
    world.spawn(("abc", 123));
    world.spawn(("def", 456, true));
    world.clear();
    assert_eq!(world.iter().count(), 0);
}

#[test]
fn remove_missing() {
    let mut world = World::new();
    let e = world.spawn(("abc", 123));
    assert!(world.remove_one::<bool>(e).is_err());
}

#[test]
fn reserve() {
    let mut world = World::new();
    let a = world.reserve_entity();
    let b = world.reserve_entity();

    assert_eq!(world.query::<()>().iter().count(), 0);

    world.flush();

    let entities = world
        .query::<()>()
        .iter()
        .map(|(e, ())| e)
        .collect::<Vec<_>>();

    assert_eq!(entities.len(), 2);
    assert!(entities.contains(&a));
    assert!(entities.contains(&b));
}

#[test]
fn query_batched() {
    let mut world = World::new();
    let a = world.spawn(());
    let b = world.spawn(());
    let c = world.spawn((42,));
    assert_eq!(world.query::<()>().iter_batched(1).count(), 3);
    assert_eq!(world.query::<()>().iter_batched(2).count(), 2);
    assert_eq!(world.query::<()>().iter_batched(2).flatten().count(), 3);
    // different archetypes are always in different batches
    assert_eq!(world.query::<()>().iter_batched(3).count(), 2);
    assert_eq!(world.query::<()>().iter_batched(3).flatten().count(), 3);
    assert_eq!(world.query::<()>().iter_batched(4).count(), 2);
    let entities = world
        .query::<()>()
        .iter_batched(1)
        .flatten()
        .map(|(e, ())| e)
        .collect::<Vec<_>>();
    dbg!(&entities);
    assert_eq!(entities.len(), 3);
    assert!(entities.contains(&a));
    assert!(entities.contains(&b));
    assert!(entities.contains(&c));
}

#[test]
fn spawn_batch() {
    let mut world = World::new();
    world.spawn_batch((0..100).map(|x| (x, "abc")));
    let entities = world
        .query::<&i32>()
        .iter()
        .map(|(_, &x)| x)
        .collect::<Vec<_>>();
    assert_eq!(entities.len(), 100);
}

#[test]
fn query_one() {
    let mut world = World::new();
    let a = world.spawn(("abc", 123));
    let b = world.spawn(("def", 456));
    let c = world.spawn(("ghi", 789, true));
    assert_eq!(world.query_one::<&i32>(a).unwrap().get(), Some(&123));
    assert_eq!(world.query_one::<&i32>(b).unwrap().get(), Some(&456));
    assert!(world.query_one::<(&i32, &bool)>(a).unwrap().get().is_none());
    assert_eq!(
        world.query_one::<(&i32, &bool)>(c).unwrap().get(),
        Some((&789, &true))
    );
    world.despawn(a).unwrap();
    assert!(world.query_one::<&i32>(a).is_err());
}

#[test]
#[cfg_attr(
    debug_assertions,
    should_panic(
        expected = "attempted to allocate entity with duplicate f32 components; each type must occur at most once!"
    )
)]
#[cfg_attr(
    not(debug_assertions),
    should_panic(
        expected = "attempted to allocate entity with duplicate components; each type must occur at most once!"
    )
)]
fn duplicate_components_panic() {
    let mut world = World::new();
    world.reserve::<(f32, i64, f32)>(1);
}

#[test]
fn spawn_column_batch() {
    let mut world = World::new();
    let mut batch_ty = ColumnBatchType::new();
    batch_ty.add::<i32>().add::<bool>();

    // Unique archetype
    let b;
    {
        let mut batch = batch_ty.clone().into_batch(2);
        let mut bs = batch.writer::<bool>().unwrap();
        bs.push(true).unwrap();
        bs.push(false).unwrap();
        let mut is = batch.writer::<i32>().unwrap();
        is.push(42).unwrap();
        is.push(43).unwrap();
        let entities = world
            .spawn_column_batch(batch.build().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(entities.len(), 2);
        assert_eq!(
            world.query_one_mut::<(&i32, &bool)>(entities[0]).unwrap(),
            (&42, &true)
        );
        assert_eq!(
            world.query_one_mut::<(&i32, &bool)>(entities[1]).unwrap(),
            (&43, &false)
        );
        world.despawn(entities[0]).unwrap();
        b = entities[1];
    }

    // Duplicate archetype
    {
        let mut batch = batch_ty.clone().into_batch(2);
        let mut bs = batch.writer::<bool>().unwrap();
        bs.push(true).unwrap();
        bs.push(false).unwrap();
        let mut is = batch.writer::<i32>().unwrap();
        is.push(44).unwrap();
        is.push(45).unwrap();
        let entities = world
            .spawn_column_batch(batch.build().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(entities.len(), 2);
        assert_eq!(*world.get::<i32>(b).unwrap(), 43);
        assert_eq!(*world.get::<i32>(entities[0]).unwrap(), 44);
        assert_eq!(*world.get::<i32>(entities[1]).unwrap(), 45);
    }
}

#[test]
fn columnar_access() {
    let mut world = World::new();
    let e = world.spawn(("abc", 123));
    let f = world.spawn(("def", 456, true));
    let g = world.spawn(("ghi", 789, false));
    let mut archetypes = world.archetypes();
    let _empty = archetypes.next().unwrap();
    let a = archetypes.next().unwrap();
    assert_eq!(a.ids(), &[e.id()]);
    assert_eq!(*a.get::<i32>().unwrap(), [123]);
    assert!(a.get::<bool>().is_none());
    let b = archetypes.next().unwrap();
    assert_eq!(b.ids(), &[f.id(), g.id()]);
    assert_eq!(*b.get::<i32>().unwrap(), [456, 789]);
}

#[test]
fn empty_entity_ref() {
    let mut world = World::new();
    let e = world.spawn(());
    let r = world.entity(e).unwrap();
    assert_eq!(r.entity(), e);
}
