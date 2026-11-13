//! Metamorphic properties for `World`: relations between two executions that
//! must hold by hecs's documented semantics, checked without a reference model.
//!
//! `World` is not `Clone`, so "two executions from the same state" is set up
//! with twin worlds replayed from one drawn history, and compared through the
//! observational fingerprint. `D` is drop-tracked throughout, so a relation
//! that holds observationally but leaks a component still fails.

use fixtures::*;
use hecs::{Entity, World};
use hegel::generators::{self as gs, Generator};
use hegel::stateful::{pool, Pool};
use hegel::TestCase;


/// Operations on two different entities commute: neither their results nor the
/// fingerprint of the resulting world depend on the order they run in. The
/// fingerprint excludes allocator state, which is order-dependent (two
/// despawns enter the freelist in run order), just as `ops()` excludes
/// spawns. This is the observable form of hecs storing each entity's
/// components independently of every other's.
#[hegel::test(settings())]
fn operations_on_distinct_entities_commute(tc: hegel::TestCase) {
    let ds = DropTracker::new();
    let history = tc.draw(histories(1, MAX_ENTITIES));
    let (mut worlds, mut pool) = build_twins(&tc, &history, 2, &ds);
    // Consuming `e1` takes it out of the pool, so the second draw cannot
    // collide with it (and rejects the case when the pool held only one).
    let e1 = tc.draw(pool.values_consumed().print_as_debug());
    let e2 = tc.draw(handle_from(&pool));
    let x = tc.draw(ops());
    let y = tc.draw(ops());

    let rx0 = apply(&mut worlds[0], e1, x, &ds);
    let ry0 = apply(&mut worlds[0], e2, y, &ds);
    let ry1 = apply(&mut worlds[1], e2, y, &ds);
    let rx1 = apply(&mut worlds[1], e1, x, &ds);
    assert_eq!(
        rx0, rx1,
        "result of {x:?} on {e1:?} depended on the order of {y:?} on {e2:?}"
    );
    assert_eq!(
        ry0, ry1,
        "result of {y:?} on {e2:?} depended on the order of {x:?} on {e1:?}"
    );
    assert_eq!(
        fingerprint(&worlds[0]),
        fingerprint(&worlds[1]),
        "[{x:?} on {e1:?}; {y:?} on {e2:?}] and the reverse order left different worlds"
    );
    assert_eq!(
        ds.live(),
        total_d(&worlds),
        "drop imbalance after commuting operations"
    );
    check_archetypes(&worlds[0], "x-then-y");
    check_archetypes(&worlds[1], "y-then-x");
}

/// `insert_one::<T>` followed by `remove_one::<T>` leaves the world as it was
/// except that `T` is gone from that entity, and returns the value just
/// inserted. `insert_one` documents that an existing component "is dropped and
/// replaced", which is what makes the residue exactly "T absent".
#[hegel::test(settings())]
fn insert_then_remove_leaves_the_component_absent(tc: hegel::TestCase) {
    let ds = DropTracker::new();
    let history = tc.draw(histories(1, MAX_ENTITIES));
    let (mut world, pool) = build_world_with_handles(&tc, &history, &ds);
    let e = tc.draw(handle_from(&pool));
    let before = fingerprint(&world);
    let live = before.contains_key(&e);
    let v = tc.draw(val());
    let kind = tc.draw(kinds());

    let inserted = match kind {
        Kind::A => {
            let ins = world.insert_one(e, A(v)).is_ok();
            assert_eq!(
                world.remove_one::<A>(e).ok(),
                ins.then_some(A(v)),
                "removed A"
            );
            ins
        }
        Kind::B => {
            let ins = world.insert_one(e, B(v)).is_ok();
            assert_eq!(
                world.remove_one::<B>(e).ok(),
                ins.then_some(B(v)),
                "removed B"
            );
            ins
        }
        Kind::C => {
            let ins = world.insert_one(e, C).is_ok();
            assert_eq!(world.remove_one::<C>(e).is_ok(), ins, "removed C");
            ins
        }
        Kind::D => {
            let ins = world.insert_one(e, D::new(v, &ds)).is_ok();
            let got = world.remove_one::<D>(e).ok();
            assert_eq!(got.as_ref().map(|d| d.value), ins.then_some(v), "removed D");
            ins
        }
    };
    assert_eq!(
        inserted, live,
        "insert_one success disagreed with liveness for {e:?}"
    );

    let mut expected = before;
    if let Some(obs) = expected.get_mut(&e) {
        *obs = obs.without(kind);
    }
    assert_eq!(
        fingerprint(&world),
        expected,
        "insert then remove left a residue on {e:?}"
    );
    assert_eq!(
        ds.live(),
        d_in(&world),
        "drop imbalance after insert then remove"
    );
}

/// `exchange_one::<A, B>` returns the old `A` and installs the new `B`, so
/// exchanging back with the returned value restores the world — except that any
/// pre-existing `B` was overwritten by the first exchange and then taken by the
/// second.
#[hegel::test(settings())]
fn exchange_roundtrip_restores_the_world(tc: hegel::TestCase) {
    let ds = DropTracker::new();
    let history = tc.draw(histories(1, MAX_ENTITIES));
    let (mut world, pool) = build_world_with_handles(&tc, &history, &ds);
    let e = tc.draw(handle_from(&pool));
    let before = fingerprint(&world);
    let a_before = before.get(&e).and_then(|o| o.a);
    let x = tc.draw(val());

    match world.exchange_one::<A, B>(e, B(x)) {
        Ok(old_a) => {
            assert_eq!(
                Some(old_a.0),
                a_before,
                "exchange returned the wrong old A for {e:?}"
            );
            let back = world
                .exchange_one::<B, A>(e, A(old_a.0))
                .expect("B was just inserted, so the reverse exchange must succeed");
            assert_eq!(back.0, x, "reverse exchange returned the wrong B for {e:?}");
            let mut expected = before;
            // The first exchange overwrote any pre-existing B and the second
            // took the replacement out, so no B survives either way.
            expected.get_mut(&e).unwrap().b = None;
            assert_eq!(
                fingerprint(&world),
                expected,
                "exchange roundtrip residue on {e:?}"
            );
        }
        Err(_) => {
            assert!(
                a_before.is_none(),
                "exchange_one::<A, B> failed but {e:?} has an A"
            );
            assert_eq!(
                fingerprint(&world),
                before,
                "a failed exchange mutated the world"
            );
        }
    }
    assert_eq!(
        ds.live(),
        d_in(&world),
        "drop imbalance after exchange roundtrip"
    );
}

/// `spawn_batch` is indistinguishable from spawning the same bundles one at a
/// time, including the handles it hands out. Dropping the iterator early must
/// still spawn the remainder. hecs does not document that, so this pins
/// de-facto behavior of `SpawnBatchIter`'s drop.
#[hegel::test(settings())]
fn spawn_batch_matches_individual_spawns(tc: hegel::TestCase) {
    let ds = DropTracker::new();
    // The history puts both allocators into the same non-trivial state, so
    // the batch also exercises freelist reuse.
    let history = tc.draw(histories(0, MAX_ENTITIES));
    let (mut worlds, _pool) = build_twins(&tc, &history, 2, &ds);
    // The `(A, B)` payload pairs to spawn, and how many to pull from the
    // batch iterator before dropping it.
    let rows: Vec<(i32, i32)> = tc.draw(gs::vecs(hegel::tuples!(val(), val())).max_size(5));
    let n = rows.len();
    let consumed = tc.draw(gs::integers::<usize>().max_value(n));

    let batched: Vec<Entity> = {
        let mut iter = worlds[0].spawn_batch(rows.iter().map(|&(a, b)| (A(a), B(b))));
        assert_eq!(iter.len(), n, "SpawnBatchIter::len");
        let taken: Vec<Entity> = iter.by_ref().take(consumed).collect();
        taken
        // dropping `iter` here spawns whatever was not consumed
    };
    let individually: Vec<Entity> = rows
        .iter()
        .map(|&(a, b)| worlds[1].spawn((A(a), B(b))))
        .collect();

    assert_eq!(
        batched,
        individually[..consumed],
        "spawn_batch handed out different handles than individual spawns"
    );
    assert_eq!(
        fingerprint(&worlds[0]),
        fingerprint(&worlds[1]),
        "spawn_batch of {n} rows with {consumed} consumed differs from individual spawns"
    );
    check_archetypes(&worlds[0], "batch");
}

/// The cleared world and a fresh one, driven in lockstep.
struct ClearedVsFresh {
    cleared: World,
    fresh: World,
    /// Every handle the lockstep spawns handed out, despawned ones included.
    handles: Pool<Entity>,
    ds: DropTracker,
}

// Each rule runs the same operation against both worlds and asserts the two
// results agree. The invariant requires the worlds to stay observationally
// identical.
#[hegel::state_machine]
impl ClearedVsFresh {
    #[rule]
    fn spawn(&mut self, tc: TestCase) {
        let cs = tc.draw(components());
        let in_cleared = self.cleared.spawn(cs.builder(&self.ds).build());
        let in_fresh = self.fresh.spawn(cs.builder(&self.ds).build());
        assert_eq!(
            in_cleared, in_fresh,
            "cleared world allocated a different handle"
        );
        self.handles.add(in_cleared);
    }

    #[rule]
    fn op(&mut self, tc: TestCase) {
        let e = tc.draw(handle_from(&self.handles));
        let op = tc.draw(ops());
        assert_eq!(
            apply(&mut self.cleared, e, op, &self.ds),
            apply(&mut self.fresh, e, op, &self.ds),
            "{op:?} on {e:?} disagreed"
        );
    }

    #[invariant]
    fn indistinguishable(&self, _: TestCase) {
        assert_eq!(
            fingerprint(&self.cleared),
            fingerprint(&self.fresh),
            "cleared world diverged from a fresh one"
        );
        assert_eq!(
            self.ds.live(),
            d_in(&self.cleared) + d_in(&self.fresh),
            "drop imbalance between the twins"
        );
        check_archetypes(&self.cleared, "cleared");
    }
}

/// After `clear`, a world is indistinguishable from `World::new()` under any
/// subsequent operations — including handing out the same `Entity` values,
/// which `clear` documents ("clears metadata so that `Entity` values will
/// repeat").
#[hegel::test(settings().stateful_step_count(STEPS))]
fn cleared_world_behaves_like_a_fresh_one(tc: TestCase) {
    let ds = DropTracker::new();
    let history = tc.draw(histories(0, MAX_ENTITIES));
    let mut cleared = build_world(&history, &ds);
    cleared.clear();
    assert_eq!(cleared.len(), 0, "clear left live entities");
    assert_eq!(ds.live(), 0, "clear did not drop every component");

    let machine = ClearedVsFresh {
        cleared,
        fresh: World::new(),
        handles: pool(&tc),
        ds,
    };
    hegel::stateful::run(machine, tc);
}

/// Inserting two different components on one entity is order-independent, even
/// though the two orders route through different intermediate archetypes.
#[hegel::test(settings())]
fn insert_order_on_one_entity_is_unobservable(tc: hegel::TestCase) {
    let ds = DropTracker::new();
    let history = tc.draw(histories(1, MAX_ENTITIES));
    let (mut worlds, pool) = build_twins(&tc, &history, 2, &ds);
    let e = tc.draw(handle_from(&pool));
    let pair = tc.draw(
        gs::samples(&KINDS[..])
            .without_replacement()
            .min_size(2)
            .max_size(2),
    );
    let (k1, k2) = (pair[0], pair[1]);
    let v1 = tc.draw(val());
    let v2 = tc.draw(val());

    let insert = |w: &mut World, k: Kind, v: i32| k.insert_one(w, e, v, &ds).is_ok();
    let k1_before_k2 = insert(&mut worlds[0], k1, v1);
    let k2_after_k1 = insert(&mut worlds[0], k2, v2);
    let k2_before_k1 = insert(&mut worlds[1], k2, v2);
    let k1_after_k2 = insert(&mut worlds[1], k1, v1);
    assert_eq!(
        k1_before_k2, k1_after_k2,
        "insert of {k1:?} was order-dependent on {e:?}"
    );
    assert_eq!(
        k2_after_k1, k2_before_k1,
        "insert of {k2:?} was order-dependent on {e:?}"
    );
    assert_eq!(
        fingerprint(&worlds[0]),
        fingerprint(&worlds[1]),
        "insert order of {k1:?} and {k2:?} was observable on {e:?}"
    );
    assert_eq!(
        ds.live(),
        total_d(&worlds),
        "drop imbalance after ordered inserts"
    );
}

/// A world and its reconstruction, driven in lockstep.
struct FreelistReplica {
    source: World,
    replica: World,
    /// Every handle the lockstep spawns handed out, despawned ones included.
    handles: Pool<Entity>,
}

// Each rule runs the same allocator operation against both worlds and asserts
// they hand out the same result. The invariant requires the freelists to
// match, so allocation stays deterministic from any reachable state.
#[hegel::state_machine]
impl FreelistReplica {
    #[rule]
    fn spawn(&mut self, _: TestCase) {
        let in_source = self.source.spawn(());
        let in_replica = self.replica.spawn(());
        assert_eq!(
            in_source, in_replica,
            "replica allocated a different handle"
        );
        self.handles.add(in_source);
    }

    #[rule]
    fn despawn(&mut self, tc: TestCase) {
        let e = tc.draw(handle_from(&self.handles));
        assert_eq!(
            self.source.despawn(e).is_ok(),
            self.replica.despawn(e).is_ok(),
            "despawn of {e:?} disagreed between the world and its replica"
        );
    }

    #[invariant]
    fn freelists_agree(&self, _: TestCase) {
        assert_eq!(
            self.source.freelist().collect::<Vec<_>>(),
            self.replica.freelist().collect::<Vec<_>>(),
            "freelists diverged"
        );
    }
}

/// A world reconstructed with `spawn_at` over the same live entities and then
/// given the original's `freelist` allocates handles identically from then on.
/// That is exactly what `World::freelist` documents it is for, and it is the
/// generalization of the `deterministic_ids` test in src/world.rs.
#[hegel::test(settings().stateful_step_count(STEPS))]
fn replaying_a_saved_freelist_reproduces_allocation(tc: TestCase) {
    let ds = DropTracker::new();
    let history = tc.draw(histories(0, MAX_ENTITIES));
    let source = build_world(&history, &ds);

    let mut replica = World::new();
    for eref in source.iter() {
        replica.spawn_at(eref.entity(), ());
    }
    replica.set_freelist(&source.freelist().collect::<Vec<_>>());
    assert_eq!(
        replica.len(),
        source.len(),
        "replica has a different number of entities"
    );

    let machine = FreelistReplica {
        source,
        replica,
        handles: pool(&tc),
    };
    hegel::stateful::run(machine, tc);
}
