//! Properties of the bundle-construction surface: `EntityBuilder`,
//! `EntityBuilderClone`, the bundle query-satisfaction predicates, the `Ref`
//! wrappers, and whole-column archetype access.
//!
//! The oracles are the drawn components, the world's own `satisfies` (which must
//! agree with the predicates that answer the same question without a world),
//! and per-entity reads (which must agree with whole-column reads). `D` is
//! drop-tracked, so a builder that leaks or double-drops what it holds fails
//! even when the components it spawns look right.

use std::collections::BTreeMap;

use fixtures::*;
use hecs::{
    bundle_satisfies_query, dynamic_bundle_satisfies_query, DynamicBundle, Entity,
    EntityBuilderClone, Ref, RefMut, World,
};

/// The bundle predicates answer "would an entity with these components match
/// `Q`?" without a world. They must agree with `World::satisfies` on an entity
/// spawned from that very bundle.
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
/// once — `BuiltEntity::drop` documents that it clears the builder so nothing
/// leaks when the bundle goes unused.
#[hegel::test(settings())]
fn an_unspawned_bundle_drops_its_components(tc: hegel::TestCase) {
    let ds = DropTracker::new();
    let cs = tc.draw(components());
    {
        let mut builder = cs.builder(&ds);
        let built = builder.build();
        assert_eq!(built.has::<D>(), cs.d.is_some(), "BuiltEntity::has::<D>");
    }
    assert_eq!(
        ds.live(),
        0,
        "an unspawned bundle leaked or double-dropped its components"
    );
}

/// `clear` drops what the builder holds and leaves it empty, so a subsequent
/// build spawns nothing.
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

/// Add the `{A, B, C}` part of `cs` one component at a time. `D` is not
/// `Clone`, so it cannot go into an `EntityBuilderClone`.
fn add_individually(builder: &mut EntityBuilderClone, cs: Components) {
    if let Some(v) = cs.a {
        builder.add(A(v));
    }
    if let Some(v) = cs.b {
        builder.add(B(v));
    }
    if cs.c {
        builder.add(C);
    }
}

/// Add the same components with one `add_bundle` of a concrete tuple, which
/// goes through the tuple `DynamicBundleClone` impls instead.
fn add_as_tuple(builder: &mut EntityBuilderClone, cs: Components) {
    match (cs.a, cs.b, cs.c) {
        (None, None, false) => builder.add_bundle(()),
        (Some(a), None, false) => builder.add_bundle((A(a),)),
        (None, Some(b), false) => builder.add_bundle((B(b),)),
        (None, None, true) => builder.add_bundle((C,)),
        (Some(a), Some(b), false) => builder.add_bundle((A(a), B(b))),
        (Some(a), None, true) => builder.add_bundle((A(a), C)),
        (None, Some(b), true) => builder.add_bundle((B(b), C)),
        (Some(a), Some(b), true) => builder.add_bundle((A(a), B(b), C)),
    };
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
    let expected = cs;
    let mut world = World::new();

    let mut individually = EntityBuilderClone::new();
    add_individually(&mut individually, cs);
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
    assert_eq!(observe(&world, e), expected, "individual adds");

    let mut as_tuple = EntityBuilderClone::new();
    add_as_tuple(&mut as_tuple, cs);
    let e = world.spawn(&as_tuple.build());
    assert_eq!(
        observe(&world, e),
        expected,
        "add_bundle of a tuple lost components"
    );

    let mut renested = EntityBuilderClone::new();
    renested.add_bundle(&built);
    let e = world.spawn(&renested.build());
    assert_eq!(
        observe(&world, e),
        expected,
        "add_bundle of a built bundle lost components"
    );
}

/// A cloned `EntityBuilderClone` spawns the same entity as the original, and
/// the original still spawns correctly afterwards.
#[hegel::test(settings())]
fn a_cloned_builder_spawns_the_same_entity(tc: hegel::TestCase) {
    let cs = tc.draw(components_without_d());
    let mut world = World::new();

    let mut original = EntityBuilderClone::new();
    add_individually(&mut original, cs);
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

/// `Ref` and `RefMut` are transparent handles on a component: projecting with
/// `map` reads the same value, and a write through a projected `RefMut` lands
/// in the world.
#[hegel::test(settings())]
fn ref_projections_read_and_write_the_component(tc: hegel::TestCase) {
    let ds = DropTracker::new();
    let v = tc.draw(val());
    let w = tc.draw(val());
    let mut world = World::new();
    let e = world.spawn((A(v), D::new(v, &ds)));

    {
        let shared: Ref<'_, A> = world.get::<&A>(e).expect("A was just spawned");
        assert_eq!(
            format!("{shared:?}"),
            format!("{:?}", A(v)),
            "Ref forwards Debug"
        );
        let copy = shared.clone();
        let projected: Ref<'_, i32> = Ref::map(shared, |a| &a.0);
        assert_eq!(*projected, v, "Ref::map read a different value");
        assert_eq!(
            copy.0, v,
            "a cloned Ref was disturbed by mapping the original"
        );
    }
    {
        let unique: RefMut<'_, A> = world.get::<&mut A>(e).expect("A was just spawned");
        let mut projected: RefMut<'_, i32> = RefMut::map(unique, |a| &mut a.0);
        *projected = w;
    }
    assert_eq!(
        world.get::<&A>(e).unwrap().0,
        w,
        "a write through RefMut::map was lost"
    );

    world.despawn(e).unwrap();
    assert_eq!(ds.live(), 0, "drop imbalance in the ref-projection test");
}

/// Whole-column archetype access presents the same values as per-entity reads,
/// and a write through a unique column is visible to ordinary reads. This is
/// the API serialization and the cloning example are built on, so the two
/// views of the same storage must not drift.
#[hegel::test(settings())]
fn archetype_columns_agree_with_per_entity_reads(tc: hegel::TestCase) {
    let ds = DropTracker::new();
    let history = tc.draw(histories(0, MAX_ENTITIES));
    let world = build_world(&history, &ds);
    let before = fingerprint(&world);

    let mut by_id: BTreeMap<u32, i32> = BTreeMap::new();
    for arch in world.archetypes() {
        if let Some(column) = arch.get::<&A>() {
            assert_eq!(
                column.len(),
                arch.len() as usize,
                "column length != archetype length"
            );
            for (&id, a) in arch.ids().iter().zip(column.iter()) {
                assert!(
                    by_id.insert(id, a.0).is_none(),
                    "entity id {id} in two A columns"
                );
            }
        }
    }
    let want: BTreeMap<u32, i32> = before
        .iter()
        .filter_map(|(e, o)| o.a.map(|v| (e.id(), v)))
        .collect();
    assert_eq!(by_id, want, "A columns disagree with per-entity reads");

    let v = tc.draw(val());
    for arch in world.archetypes() {
        if let Some(mut column) = arch.get::<&mut B>() {
            for b in column.iter_mut() {
                b.0 = v;
            }
        }
    }
    for (e, o) in fingerprint(&world) {
        assert_eq!(
            o.b,
            before[&e].b.map(|_| v),
            "the column write to B missed {e:?}"
        );
        assert_eq!(
            o.a, before[&e].a,
            "the column write to B disturbed A on {e:?}"
        );
        assert_eq!(
            o.d, before[&e].d,
            "the column write to B disturbed D on {e:?}"
        );
    }
}

/// `build()` used to sort the component info by alignment without rebuilding
/// the `TypeId -> slot` map, so after the documented `BuiltEntityClone ->
/// EntityBuilderClone` round trip `get` read the wrong slot. Reproduces on
/// hecs 0.11.1 (issue #460).
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
/// observes it. Reproduces on hecs 0.11.1 (issue #461).
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
