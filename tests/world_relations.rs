//! Metamorphic properties for `World`: relations between two executions that
//! must hold by hecs's documented semantics, checked without a reference model.
//!
//! `World` is not `Clone`, so "two executions from the same state" is set up
//! with twin worlds replayed from one drawn history, and compared through the
//! observational fingerprint. `D` is drop-tracked throughout, so a relation
//! that holds observationally but leaks a component still fails.

mod common;

use common::*;
use hecs::{Entity, World};
use hegel::generators as gs;

/// A single-entity operation. Spawning operations are excluded: allocation
/// order is observable through the handles they return, so they do not commute.
#[derive(Clone, Copy, Debug, hegel::PrettyPrintable)]
enum Op {
    InsertOne(u8, i32),
    RemoveOne(u8),
    InsertBundle(Spec),
    RemoveAB,
    RemoveCD,
    Despawn,
    Take,
    MutateA(i32),
    ExchangeAToB(i32),
    ExchangeDToA(i32),
}

#[hegel::composite]
fn ops(tc: &hegel::TestCase) -> Op {
    fn which() -> impl hegel::generators::PrintableGenerator<u8> {
        gs::integers::<u8>().min_value(0).max_value(3)
    }
    tc.draw(hegel::one_of!(
        hegel::compose!(|tc| { Op::InsertOne(tc.draw(which()), tc.draw(val())) }),
        hegel::compose!(|tc| { Op::RemoveOne(tc.draw(which())) }),
        hegel::compose!(|tc| { Op::InsertBundle(tc.draw(specs())) }),
        gs::just(Op::RemoveAB),
        gs::just(Op::RemoveCD),
        gs::just(Op::Despawn),
        gs::just(Op::Take),
        hegel::compose!(|tc| { Op::MutateA(tc.draw(val())) }),
        hegel::compose!(|tc| { Op::ExchangeAToB(tc.draw(val())) }),
        hegel::compose!(|tc| { Op::ExchangeDToA(tc.draw(val())) }),
    ))
}

/// Apply `op` to `e` and report whether it succeeded.
fn apply(world: &mut World, e: Entity, op: Op) -> bool {
    match op {
        Op::InsertOne(which, v) => match which {
            0 => world.insert_one(e, A(v)).is_ok(),
            1 => world.insert_one(e, B(v)).is_ok(),
            2 => world.insert_one(e, C).is_ok(),
            _ => world.insert_one(e, D::new(v)).is_ok(),
        },
        Op::RemoveOne(which) => match which {
            0 => world.remove_one::<A>(e).is_ok(),
            1 => world.remove_one::<B>(e).is_ok(),
            2 => world.remove_one::<C>(e).is_ok(),
            _ => world.remove_one::<D>(e).is_ok(),
        },
        Op::InsertBundle(s) => world.insert(e, make_builder(s).build()).is_ok(),
        Op::RemoveAB => world.remove::<(A, B)>(e).is_ok(),
        Op::RemoveCD => world.remove::<(C, D)>(e).is_ok(),
        Op::Despawn => world.despawn(e).is_ok(),
        Op::Take => world.take(e).is_ok(),
        Op::MutateA(v) => match world.get::<&mut A>(e) {
            Ok(mut a) => {
                a.0 = v;
                true
            }
            Err(_) => false,
        },
        Op::ExchangeAToB(v) => world.exchange_one::<A, B>(e, B(v)).is_ok(),
        Op::ExchangeDToA(v) => world.exchange_one::<D, A>(e, A(v)).is_ok(),
    }
}

/// Operations on two different entities commute: neither their results nor the
/// resulting world depend on the order they run in. This is the observable form
/// of hecs storing each entity's components independently of every other's.
#[hegel::test(settings())]
fn operations_on_distinct_entities_commute(tc: hegel::TestCase) {
    assert_d_balanced_at_start();
    let (mut worlds, pool) = build_twins(&tc, 2, MAX_ENTITIES);
    let Some(e1) = pick(&tc, &pool) else { return };
    let e2 = pick(&tc, &pool).unwrap();
    tc.assume(e1 != e2);
    let x = tc.draw(ops());
    let y = tc.draw(ops());

    let (rx0, ry0, rx1, ry1);
    {
        let (first, second) = worlds.split_at_mut(1);
        rx0 = apply(&mut first[0], e1, x);
        ry0 = apply(&mut first[0], e2, y);
        ry1 = apply(&mut second[0], e2, y);
        rx1 = apply(&mut second[0], e1, x);
    }
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
        d_live(),
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
    assert_d_balanced_at_start();
    let (mut worlds, pool) = build_twins(&tc, 1, MAX_ENTITIES);
    let world = &mut worlds[0];
    let Some(e) = pick(&tc, &pool) else { return };
    let before = fingerprint(world);
    let live = before.contains_key(&e);
    let v = tc.draw(val());
    let which = tc.draw(gs::integers::<u8>().min_value(0).max_value(3));

    let inserted = match which {
        0 => {
            let ins = world.insert_one(e, A(v)).is_ok();
            assert_eq!(
                world.remove_one::<A>(e).ok(),
                ins.then_some(A(v)),
                "removed A"
            );
            ins
        }
        1 => {
            let ins = world.insert_one(e, B(v)).is_ok();
            assert_eq!(
                world.remove_one::<B>(e).ok(),
                ins.then_some(B(v)),
                "removed B"
            );
            ins
        }
        2 => {
            let ins = world.insert_one(e, C).is_ok();
            assert_eq!(world.remove_one::<C>(e).is_ok(), ins, "removed C");
            ins
        }
        _ => {
            let ins = world.insert_one(e, D::new(v)).is_ok();
            let got = world.remove_one::<D>(e).ok();
            assert_eq!(got.as_ref().map(|d| d.0), ins.then_some(v), "removed D");
            ins
        }
    };
    assert_eq!(
        inserted, live,
        "insert_one succeeded on a dead handle {e:?}"
    );

    let mut expected = before;
    if let Some(obs) = expected.get_mut(&e) {
        match which {
            0 => obs.a = None,
            1 => obs.b = None,
            2 => obs.c = false,
            _ => obs.d = None,
        }
    }
    assert_eq!(
        fingerprint(world),
        expected,
        "insert then remove left a residue on {e:?}"
    );
    assert_eq!(
        d_live(),
        total_d(&worlds),
        "drop imbalance after insert then remove"
    );
}

/// `exchange_one::<A, B>` returns the old `A` and installs the new `B`, so
/// exchanging back with the returned value restores the world — except that any
/// pre-existing `B` was overwritten by the first exchange and then taken by the
/// second.
#[hegel::test(settings())]
fn exchange_roundtrip_restores_the_world(tc: hegel::TestCase) {
    assert_d_balanced_at_start();
    let (mut worlds, pool) = build_twins(&tc, 1, MAX_ENTITIES);
    let world = &mut worlds[0];
    let Some(e) = pick(&tc, &pool) else { return };
    let before = fingerprint(world);
    let had_a = before.get(&e).and_then(|o| o.a);
    let x = tc.draw(val());

    match world.exchange_one::<A, B>(e, B(x)) {
        Ok(old_a) => {
            assert_eq!(
                Some(old_a.0),
                had_a,
                "exchange returned the wrong old A for {e:?}"
            );
            let back = world
                .exchange_one::<B, A>(e, A(old_a.0))
                .expect("B was just inserted, so the reverse exchange must succeed");
            assert_eq!(back.0, x, "reverse exchange returned the wrong B for {e:?}");
            let mut expected = before;
            expected.get_mut(&e).unwrap().b = None;
            assert_eq!(
                fingerprint(world),
                expected,
                "exchange roundtrip residue on {e:?}"
            );
        }
        Err(_) => {
            assert!(
                had_a.is_none(),
                "exchange_one::<A, B> failed but {e:?} has an A"
            );
            assert_eq!(
                fingerprint(world),
                before,
                "a failed exchange mutated the world"
            );
        }
    }
    assert_eq!(
        d_live(),
        total_d(&worlds),
        "drop imbalance after exchange roundtrip"
    );
}

/// `spawn_batch` is indistinguishable from spawning the same bundles one at a
/// time, including the handles it hands out. Dropping the iterator early must
/// still spawn the remainder: `SpawnBatchIter`'s `Drop` is documented to drain
/// it.
#[hegel::test(settings())]
fn spawn_batch_matches_individual_spawns(tc: hegel::TestCase) {
    assert_d_balanced_at_start();
    let (mut worlds, _pool) = build_twins(&tc, 2, MAX_ENTITIES);
    let n = tc.draw(gs::integers::<usize>().min_value(0).max_value(5));
    let rows: Vec<(i32, i32)> = (0..n).map(|_| (tc.draw(val()), tc.draw(val()))).collect();
    let consumed = tc.draw(gs::integers::<usize>().min_value(0).max_value(n));

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

/// After `clear`, a world is indistinguishable from `World::new()` under any
/// subsequent operations — including handing out the same `Entity` values,
/// which `clear` documents ("clears metadata so that `Entity` values will
/// repeat").
#[hegel::test(settings())]
fn cleared_world_behaves_like_a_fresh_one(tc: hegel::TestCase) {
    assert_d_balanced_at_start();
    let (mut worlds, _pool) = build_twins(&tc, 1, MAX_ENTITIES);
    let mut cleared = worlds.pop().unwrap();
    cleared.clear();
    assert_eq!(cleared.len(), 0, "clear left live entities");
    assert_eq!(d_live(), 0, "clear did not drop every component");

    let mut fresh = World::new();
    let mut pool: Vec<Entity> = Vec::new();
    let steps = tc.draw(
        gs::integers::<u32>()
            .min_value(0)
            .max_value(MAX_ENTITIES * 2),
    );
    for _ in 0..steps {
        match tc.draw(gs::integers::<u8>().min_value(0).max_value(3)) {
            0 | 1 => {
                let s = tc.draw(specs());
                let in_cleared = cleared.spawn(make_builder(s).build());
                let in_fresh = fresh.spawn(make_builder(s).build());
                assert_eq!(
                    in_cleared, in_fresh,
                    "cleared world allocated a different handle"
                );
                pool.push(in_cleared);
            }
            2 => {
                if let Some(e) = pick(&tc, &pool) {
                    assert_eq!(
                        cleared.despawn(e).is_ok(),
                        fresh.despawn(e).is_ok(),
                        "despawn of {e:?} disagreed"
                    );
                }
            }
            _ => {
                if let Some(e) = pick(&tc, &pool) {
                    let v = tc.draw(val());
                    assert_eq!(
                        cleared.insert_one(e, A(v)).is_ok(),
                        fresh.insert_one(e, A(v)).is_ok(),
                        "insert_one on {e:?} disagreed"
                    );
                }
            }
        }
        assert_eq!(
            fingerprint(&cleared),
            fingerprint(&fresh),
            "cleared world diverged from a fresh one"
        );
    }
    check_archetypes(&cleared, "cleared");
}

/// Inserting two different components on one entity is order-independent, even
/// though the two orders route through different intermediate archetypes.
#[hegel::test(settings())]
fn insert_order_on_one_entity_is_unobservable(tc: hegel::TestCase) {
    assert_d_balanced_at_start();
    let (mut worlds, pool) = build_twins(&tc, 2, MAX_ENTITIES);
    let Some(e) = pick(&tc, &pool) else { return };
    let which = gs::integers::<u8>().min_value(0).max_value(3);
    let c1 = tc.draw(which);
    let c2 = tc.draw(gs::integers::<u8>().min_value(0).max_value(3));
    tc.assume(c1 != c2);
    let (v1, v2) = (tc.draw(val()), tc.draw(val()));

    let insert = |w: &mut World, which: u8, v: i32| apply(w, e, Op::InsertOne(which, v));
    let first1 = insert(&mut worlds[0], c1, v1);
    let first2 = insert(&mut worlds[0], c2, v2);
    let second2 = insert(&mut worlds[1], c2, v2);
    let second1 = insert(&mut worlds[1], c1, v1);
    assert_eq!(
        first1, second1,
        "insert of component {c1} was order-dependent on {e:?}"
    );
    assert_eq!(
        first2, second2,
        "insert of component {c2} was order-dependent on {e:?}"
    );
    assert_eq!(
        fingerprint(&worlds[0]),
        fingerprint(&worlds[1]),
        "insert order of components {c1} and {c2} was observable on {e:?}"
    );
    assert_eq!(
        d_live(),
        total_d(&worlds),
        "drop imbalance after ordered inserts"
    );
}

/// A world reconstructed with `spawn_at` over the same live entities and then
/// given the original's `freelist` allocates handles identically from then on.
/// That is exactly what `World::freelist` documents it is for, and it is the
/// generalization of the `deterministic_ids` test in src/world.rs.
#[hegel::test(settings())]
fn replaying_a_saved_freelist_reproduces_allocation(tc: hegel::TestCase) {
    assert_d_balanced_at_start();
    let (mut worlds, _pool) = build_twins(&tc, 1, MAX_ENTITIES);
    let mut source = worlds.pop().unwrap();

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

    let mut pool: Vec<Entity> = Vec::new();
    let steps = tc.draw(
        gs::integers::<u32>()
            .min_value(0)
            .max_value(MAX_ENTITIES * 2),
    );
    for _ in 0..steps {
        if tc.draw(gs::weighted_booleans(0.7)) {
            let in_source = source.spawn(());
            let in_replica = replica.spawn(());
            assert_eq!(
                in_source, in_replica,
                "replica allocated a different handle"
            );
            pool.push(in_source);
        } else if let Some(e) = pick(&tc, &pool) {
            assert_eq!(
                source.despawn(e).is_ok(),
                replica.despawn(e).is_ok(),
                "despawn of {e:?} disagreed between the world and its replica"
            );
        }
    }
    assert_eq!(
        source.freelist().collect::<Vec<_>>(),
        replica.freelist().collect::<Vec<_>>(),
        "freelists diverged"
    );
}
