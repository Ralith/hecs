// Copyright 2019 Google LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

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
fn build_entity() {
    let mut world = World::new();
    let mut entity = EntityBuilder::new();
    entity.add("abc");
    entity.add(123);
    let e = world.spawn(entity.build());
    entity.add("def");
    entity.add([0u8; 1024]);
    entity.add(456);
    let f = world.spawn(entity.build());
    assert_eq!(*world.get::<&str>(e).unwrap(), "abc");
    assert_eq!(*world.get::<i32>(e).unwrap(), 123);
    assert_eq!(*world.get::<&str>(f).unwrap(), "def");
    assert_eq!(*world.get::<i32>(f).unwrap(), 456);
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
        y: f64,
    }

    let mut world = World::new();
    let e = world.spawn(Foo { x: 42, y: 1.0 });
    assert_eq!(*world.get::<i32>(e).unwrap(), 42);
    assert_eq!(*world.get::<f64>(e).unwrap(), 1.0);
}

#[test]
#[cfg(feature = "macros")]
#[should_panic(expected = "each type must occur at most once")]
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
#[should_panic(expected = "twice on the same borrow")]
fn alias() {
    let mut world = World::new();
    world.spawn(("abc", 123));
    world.spawn(("def", 456, true));
    let mut q = world.query::<&mut i32>();
    let _a = q.iter().collect::<Vec<_>>();
    let _b = q.iter().collect::<Vec<_>>();
}

#[test]
fn remove_missing() {
    let mut world = World::new();
    let e = world.spawn(("abc", 123));
    assert!(world.remove_one::<bool>(e).is_err());
}
