//! Properties of the bundle-construction surface: `EntityBuilder`,
//! `EntityBuilderClone`, and the bundle query-satisfaction predicates.
//!
//! The oracles are the drawn components and the world's own `satisfies`, which
//! must agree with the predicates that answer the same question without a
//! world. `D` is drop-tracked, so a builder that leaks or double-drops what it
//! holds fails even when the components it spawns look right.

use fixtures::*;
use hecs::{
    bundle_satisfies_query, dynamic_bundle_satisfies_query, DynamicBundle, DynamicBundleClone,
    Entity, EntityBuilder, EntityBuilderClone, World,
};

/// The bundle predicates answer "would an entity with these components match
/// `Q`?" without a world. They must agree with `World::satisfies` on an entity
/// spawned from that very bundle. Along the way, `BuiltEntity::has` must
/// agree with the drawn components.
#[hegel::test(settings())]
fn bundle_satisfaction_agrees_with_world_satisfies(tc: hegel::TestCase) {
    let ds = DropTracker::new();
    let cs = tc.draw(components());
    let mut world = World::new();

    let mut builder = cs.builder(&ds);
    let built = builder.build();
    assert_eq!(built.has::<A>(), cs.a.is_some(), "BuiltEntity::has::<A>");
    assert_eq!(built.has::<B>(), cs.b.is_some(), "BuiltEntity::has::<B>");
    assert_eq!(built.has::<C>(), cs.c, "BuiltEntity::has::<C>");
    assert_eq!(built.has::<D>(), cs.d.is_some(), "BuiltEntity::has::<D>");

    let predicted = (
        dynamic_bundle_satisfies_query::<_, &A>(&built),
        dynamic_bundle_satisfies_query::<_, &D>(&built),
        dynamic_bundle_satisfies_query::<_, (&A, &B)>(&built),
        dynamic_bundle_satisfies_query::<_, (&A, &C)>(&built),
    );
    let e = world.spawn(built);
    let actual = (
        world.satisfies::<&A>(e),
        world.satisfies::<&D>(e),
        world.satisfies::<(&A, &B)>(e),
        world.satisfies::<(&A, &C)>(e),
    );
    assert_eq!(
        predicted, actual,
        "dynamic_bundle_satisfies_query disagreed with satisfies"
    );
}

/// The static form answers the same question from the bundle's type alone.
#[test]
fn static_bundle_satisfaction_agrees_with_world_satisfies() {
    let mut world = World::new();
    let tuple = world.spawn((A(1), B(2)));
    assert_eq!(
        (
            bundle_satisfies_query::<(A, B), &A>(),
            bundle_satisfies_query::<(A, B), (&A, &B)>(),
            bundle_satisfies_query::<(A, B), &C>(),
            bundle_satisfies_query::<(A, B), (&A, &C)>(),
        ),
        (
            world.satisfies::<&A>(tuple),
            world.satisfies::<(&A, &B)>(tuple),
            world.satisfies::<&C>(tuple),
            world.satisfies::<(&A, &C)>(tuple),
        ),
        "bundle_satisfies_query disagreed with satisfies"
    );
    let pair = (A(1), B(2));
    assert!(
        pair.has::<A>() && pair.has::<B>() && !pair.has::<C>(),
        "tuple DynamicBundle::has"
    );
}

/// A bundle that is built but never spawned still drops its components exactly
/// once. hecs does not document that (the `EntityBuilder` doc example only
/// shows reuse after a spawn), so this pins de-facto behavior of
/// `BuiltEntity`'s drop.
#[hegel::test(settings())]
fn an_unspawned_bundle_drops_its_components(tc: hegel::TestCase) {
    let ds = DropTracker::new();
    let cs = tc.draw(components());
    {
        let mut builder = cs.builder(&ds);
        let _built = builder.build();
    }
    assert_eq!(
        ds.live(),
        0,
        "an unspawned bundle leaked or double-dropped its components"
    );
}

/// `clear` drops what the builder holds and leaves it empty, so a subsequent
/// build spawns an entity with no components.
#[hegel::test(settings())]
fn clearing_a_builder_drops_its_components(tc: hegel::TestCase) {
    let ds = DropTracker::new();
    let cs = tc.draw(components());
    let mut world = World::new();
    let mut builder = cs.builder(&ds);
    builder.clear();
    assert_eq!(
        builder.component_types().count(),
        0,
        "clear left component types"
    );
    assert!(
        !builder.has::<A>() && !builder.has::<D>(),
        "clear left components"
    );
    assert_eq!(
        ds.live(),
        0,
        "clear leaked or double-dropped the builder's components"
    );

    let e = world.spawn(builder.build());
    assert_eq!(
        fingerprint(&world).get(&e),
        Some(&Components::default()),
        "a cleared builder spawned components"
    );
}

/// An edit made through `get_mut` before building is the value that gets
/// spawned.
#[hegel::test(settings())]
fn builder_edits_through_get_mut_are_spawned(tc: hegel::TestCase) {
    let ds = DropTracker::new();
    let cs = tc.draw(components());
    let v = tc.draw(val());
    let mut world = World::new();
    let mut builder = cs.builder(&ds);

    let mut expected = cs;
    if let Some(a) = builder.get_mut::<&mut A>() {
        a.0 = v;
        expected.a = Some(v);
    }
    if let Some(d) = builder.get_mut::<&mut D>() {
        d.value = v;
        expected.d = Some(v);
    }
    let e = world.spawn(builder.build());
    assert_eq!(
        fingerprint(&world).get(&e),
        Some(&expected),
        "get_mut edits were not spawned"
    );
}

/// Add the same components with one `add_bundle` of a concrete tuple, which
/// goes through the tuple `DynamicBundleClone` impls instead.
fn add_as_tuple(builder: &mut EntityBuilderClone, cs: Components) {
    assert!(cs.d.is_none(), "D is not Clone");
    struct Add<'a>(&'a mut EntityBuilderClone);
    impl TupleSink for Add<'_> {
        fn put(&mut self, bundle: impl DynamicBundleClone) {
            self.0.add_bundle(bundle);
        }
        fn put_with_d(&mut self, _: impl DynamicBundle) {
            unreachable!("checked D-free above");
        }
    }
    put_as_tuple(&mut Add(builder), cs, &DropTracker::new());
}

fn observe(world: &World, e: Entity) -> Components {
    *fingerprint(world)
        .get(&e)
        .unwrap_or_else(|| panic!("{e:?} is missing from the world"))
}

/// `add_bundle` of a tuple is equivalent to adding the same components one at
/// a time, and a `BuiltEntityClone` fed back in through `add_bundle` carries
/// them all across.
#[hegel::test(settings())]
fn add_bundle_matches_individual_adds(tc: hegel::TestCase) {
    let cs = tc.draw(components_without_d());
    let mut world = World::new();

    let individually = cs.clone_builder();
    assert_eq!(
        individually.get::<&A>().map(|r| r.0),
        cs.a,
        "EntityBuilderClone::get::<&A>"
    );
    assert_eq!(
        individually.component_types().count(),
        cs.component_count(),
        "EntityBuilderClone::component_types"
    );
    let built = individually.build();
    let e = world.spawn(&built);
    assert_eq!(observe(&world, e), cs, "individual adds");

    let mut as_tuple = EntityBuilderClone::new();
    add_as_tuple(&mut as_tuple, cs);
    let e = world.spawn(&as_tuple.build());
    assert_eq!(
        observe(&world, e),
        cs,
        "add_bundle of a tuple lost components"
    );

    let mut renested = EntityBuilderClone::new();
    renested.add_bundle(&built);
    let e = world.spawn(&renested.build());
    assert_eq!(
        observe(&world, e),
        cs,
        "add_bundle of a built bundle lost components"
    );
}

/// `EntityBuilder::add_bundle` (a separate code path from the `Clone`
/// variant's) is equivalent to adding the same components one at a time, and
/// a second bundle replaces the components it shares with the first, dropping
/// the replaced values.
#[hegel::test(settings())]
fn entity_builder_add_bundle_matches_individual_adds(tc: hegel::TestCase) {
    let ds = DropTracker::new();
    let first = tc.draw(components());
    let second = tc.draw(components());
    let mut world = World::new();

    struct Add<'a>(&'a mut EntityBuilder);
    impl TupleSink for Add<'_> {
        fn put(&mut self, bundle: impl DynamicBundleClone) {
            self.0.add_bundle(bundle);
        }
        fn put_with_d(&mut self, bundle: impl DynamicBundle) {
            self.0.add_bundle(bundle);
        }
    }

    let mut via_adds = first.builder(&ds);
    let e = world.spawn(via_adds.build());
    let mut via_bundle = EntityBuilder::new();
    put_as_tuple(&mut Add(&mut via_bundle), first, &ds);
    let f = world.spawn(via_bundle.build());
    assert_eq!(
        observe(&world, e),
        observe(&world, f),
        "add_bundle of a tuple disagrees with individual adds"
    );

    let mut overwritten = EntityBuilder::new();
    put_as_tuple(&mut Add(&mut overwritten), first, &ds);
    put_as_tuple(&mut Add(&mut overwritten), second, &ds);
    let g = world.spawn(overwritten.build());
    let expected = Components {
        a: second.a.or(first.a),
        b: second.b.or(first.b),
        c: first.c || second.c,
        d: second.d.or(first.d),
    };
    assert_eq!(observe(&world, g), expected, "the second bundle must win");
    assert_eq!(
        ds.live(),
        d_in(&world),
        "a replaced D was leaked or double-dropped"
    );
}

/// A cloned `EntityBuilderClone` spawns the same entity as the original, and
/// the original still spawns correctly afterwards.
#[hegel::test(settings())]
fn a_cloned_builder_spawns_the_same_entity(tc: hegel::TestCase) {
    let cs = tc.draw(components_without_d());
    let mut world = World::new();

    let original = cs.clone_builder();
    let copy = original.clone();
    assert_eq!(
        copy.has::<A>(),
        cs.a.is_some(),
        "a clone lost a component type"
    );

    let from_original = world.spawn(&original.build());
    let from_copy = world.spawn(&copy.build());
    assert_eq!(
        observe(&world, from_original),
        cs,
        "the original builder spawned the wrong entity"
    );
    assert_eq!(
        observe(&world, from_copy),
        cs,
        "a cloned builder spawned a different entity"
    );
}

/// `build()` used to sort the component info by alignment without rebuilding
/// the `TypeId -> slot` map, so after the documented `BuiltEntityClone ->
/// EntityBuilderClone` round trip `get` read the wrong slot.
// https://github.com/Ralith/hecs/issues/460
#[test]
fn builder_clone_roundtrip_preserves_component_lookup() {
    #[derive(Clone)]
    struct Small(u8);
    #[derive(Clone)]
    struct Big(u64);

    let mut builder = EntityBuilderClone::new();
    // Added lowest-alignment first, so the sort in build() is guaranteed to
    // permute the info vector.
    builder.add(Small(7));
    builder.add(Big(0x4242_4242_4242_4242));
    let built = builder.build();

    // Spawning is unaffected: it iterates the info vector directly.
    let mut world = World::new();
    let e = world.spawn(&built);
    assert_eq!(world.get::<&Small>(e).unwrap().0, 7, "spawned Small");
    assert_eq!(
        world.get::<&Big>(e).unwrap().0,
        0x4242_4242_4242_4242,
        "spawned Big"
    );

    let roundtripped: EntityBuilderClone = built.into();
    assert_eq!(
        roundtripped.get::<&Small>().map(|s| s.0),
        Some(7),
        "Small after a round trip"
    );
    assert_eq!(
        roundtripped.get::<&Big>().map(|b| b.0),
        Some(0x4242_4242_4242_4242),
        "Big after a round trip"
    );
}

/// `Clone for EntityBuilderClone` used to call `alloc` with a zero-size layout
/// for a builder holding nothing, or only zero-sized components. Only Miri
/// observes it.
// https://github.com/Ralith/hecs/issues/461
#[test]
fn cloning_an_empty_clone_builder_is_sound() {
    #[derive(Clone)]
    struct Marker;

    let empty = EntityBuilderClone::new();
    drop(empty.clone());

    let mut zero_sized = EntityBuilderClone::new();
    zero_sized.add(Marker);
    drop(zero_sized.clone());
}
