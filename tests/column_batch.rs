//! Properties of column-batch spawning: `ColumnBatchType`,
//! `ColumnBatchBuilder`, `BatchWriter`, `World::spawn_column_batch` and
//! `World::spawn_column_batch_at`.
//!
//! The oracle for a batch is the same rows spawned one at a time, plus the
//! push order the writers were given. `D` is drop-tracked, which is what makes
//! the leak properties here meaningful: this machinery moves component data
//! with raw pointer copies and owns partially initialized storage.

mod common;

use std::collections::HashSet;

use common::*;
use hecs::{ColumnBatch, ColumnBatchBuilder, ColumnBatchType, Entity, TypeInfo, World};
use hegel::generators as gs;

/// One entity's worth of batch data: payloads for its `A` and `D` columns.
type Row = (i32, i32);

fn draw_rows(tc: &hegel::TestCase, max: usize) -> Vec<Row> {
    let n = tc.draw(gs::integers::<usize>().min_value(0).max_value(max));
    (0..n).map(|_| (tc.draw(val()), tc.draw(val()))).collect()
}

/// Build a complete `{A, D}` batch from `rows`, exercising the writer surface
/// on the way: `writer` for a type outside the batch, `fill`, and a push past
/// capacity.
fn build_batch(rows: &[Row]) -> ColumnBatch {
    let mut types = ColumnBatchType::new();
    types.add::<A>();
    types.add::<D>();
    let builder = types.into_batch(rows.len() as u32);

    assert!(
        builder.writer::<B>().is_none(),
        "writer::<B> exists but B is not in the batch type"
    );
    {
        let mut writer = builder.writer::<A>().expect("A is in the batch type");
        for &(a, _) in rows {
            writer.push(A(a)).expect("push within capacity");
        }
        assert_eq!(writer.fill(), rows.len() as u32, "A column fill");
        assert_eq!(
            writer.push(A(0)),
            Err(A(0)),
            "a push past capacity must return the value"
        );
    }
    {
        let mut writer = builder.writer::<D>().expect("D is in the batch type");
        for &(_, d) in rows {
            writer.push(D::new(d)).expect("push within capacity");
        }
        assert_eq!(writer.fill(), rows.len() as u32, "D column fill");
    }
    builder.build().expect("a fully filled batch must build")
}

/// A world's observable contents as an order-independent multiset.
fn multiset(world: &World) -> Vec<Spec> {
    let mut v: Vec<Spec> = fingerprint(world).into_values().collect();
    v.sort();
    v
}

/// `spawn_column_batch` produces the same entities as spawning the same rows
/// one at a time, and hands out one distinct handle per row carrying that row's
/// components in push order.
///
/// Both worlds are seeded with unrelated archetypes and a non-empty freelist,
/// the state that made `spawn_column_batch` panic before the fix in
/// "Fix panic after spawn_column_batch on a world with a nonempty freelist".
#[hegel::test(settings())]
fn column_batch_matches_individual_spawns(tc: hegel::TestCase) {
    assert_d_balanced_at_start();
    let mut batched = World::new();
    let mut individually = World::new();

    let seeded = tc.draw(gs::integers::<u32>().min_value(0).max_value(6));
    let mut seed_handles = Vec::new();
    for _ in 0..seeded {
        let s = tc.draw(specs());
        let e = batched.spawn(make_builder(s).build());
        assert_eq!(
            e,
            individually.spawn(make_builder(s).build()),
            "seeding diverged"
        );
        seed_handles.push(e);
    }
    for &e in &seed_handles {
        if tc.draw(gs::booleans()) {
            batched.despawn(e).expect("despawn of a live seed");
            individually.despawn(e).expect("despawn of a live seed");
        }
    }

    // Two successive batches, so the second one merges into the archetype the
    // first one created.
    for _ in 0..tc.draw(gs::integers::<u8>().min_value(1).max_value(2)) {
        let rows = draw_rows(&tc, 12);
        let len_before = batched.len();
        let mut seen: HashSet<Entity> = batched.iter().map(|eref| eref.entity()).collect();

        let iter = batched.spawn_column_batch(build_batch(&rows));
        assert_eq!(iter.len(), rows.len(), "SpawnColumnBatchIter::len");
        let handles: Vec<Entity> = iter.collect();
        assert_eq!(
            handles.len(),
            rows.len(),
            "SpawnColumnBatchIter yielded the wrong count"
        );
        assert_eq!(
            batched.len(),
            len_before + rows.len() as u32,
            "world.len() after spawn_column_batch"
        );

        for (i, (&e, &(a, d))) in handles.iter().zip(&rows).enumerate() {
            assert!(seen.insert(e), "row {i} reused handle {e:?}");
            assert!(batched.contains(e), "row {i} handle {e:?} is not contained");
            assert_eq!(batched.get::<&A>(e).unwrap().0, a, "A of row {i}");
            assert_eq!(batched.get::<&D>(e).unwrap().0, d, "D of row {i}");
        }

        for &(a, d) in &rows {
            individually.spawn((A(a), D::new(d)));
        }
        assert_eq!(
            multiset(&batched),
            multiset(&individually),
            "the batch world and the one-at-a-time world hold different entities"
        );
        check_archetypes(&batched, "batch world");
        assert_eq!(
            d_live(),
            d_in(&batched) + d_in(&individually),
            "drop imbalance after a batch"
        );
    }
}

/// How many `D` components a world holds.
fn d_in(world: &World) -> i64 {
    world.query::<&D>().iter().count() as i64
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    /// Live, with components the batch does not supply; they must not survive.
    Live,
    /// Spawned and then despawned; the batch resurrects the exact handle.
    Despawned,
}

/// `spawn_column_batch_at` places row `i` on handle `i`, replacing whatever
/// occupied that id — including the components the batch does not supply — and
/// leaves every other entity alone.
///
/// The handle list may repeat a handle. `spawn_column_batch_at_redundant` in
/// src/world.rs pins the semantics: the last row for an id wins and the earlier
/// duplicates are discarded. Repeated handles used to write out of bounds
/// (issue #449) and then to leave zombie entities behind, so this is the
/// regression witness for both fixes.
#[hegel::test(settings())]
fn column_batch_at_places_each_row_on_its_handle(tc: hegel::TestCase) {
    assert_d_balanced_at_start();
    let mut world = World::new();

    // Bystanders first, then targets, then despawns: every id stays distinct,
    // so a repeated handle in the batch list is the only way two rows can
    // collide.
    let mut bystanders: Vec<(Entity, Spec)> = Vec::new();
    for _ in 0..tc.draw(gs::integers::<u32>().min_value(0).max_value(4)) {
        let s = Spec {
            b: Some(tc.draw(val())),
            ..Spec::default()
        };
        bystanders.push((world.spawn(make_builder(s).build()), s));
    }

    let kinds: Vec<Target> = (0..tc.draw(gs::integers::<usize>().min_value(0).max_value(6)))
        .map(|_| {
            if tc.draw(gs::booleans()) {
                Target::Live
            } else {
                Target::Despawned
            }
        })
        .collect();
    let mut targets: Vec<Entity> = Vec::new();
    for _ in &kinds {
        // Every target starts with a full component set, so the batch has to
        // remove B and C as well as overwrite A and D.
        let s = Spec {
            a: Some(tc.draw(val())),
            b: Some(tc.draw(val())),
            c: true,
            d: Some(tc.draw(val())),
        };
        targets.push(world.spawn(make_builder(s).build()));
    }
    for (&kind, &e) in kinds.iter().zip(&targets) {
        if kind == Target::Despawned {
            world.despawn(e).expect("despawn of a live target");
        }
    }

    if targets.is_empty() {
        return;
    }
    let rows = draw_rows(&tc, 8);
    let handles: Vec<Entity> = rows.iter().map(|_| pick(&tc, &targets).unwrap()).collect();

    let before = fingerprint(&world);
    let d_before = d_live();
    let batch = build_batch(&rows);
    assert_eq!(
        d_live(),
        d_before + rows.len() as i64,
        "the batch holds one D per row"
    );
    world.spawn_column_batch_at(&handles, batch);

    // The last row for an id wins.
    let mut expected: Vec<(Entity, Spec)> = bystanders.clone();
    let mut placed: Vec<(Entity, Spec)> = Vec::new();
    for (&e, &(a, d)) in handles.iter().zip(&rows) {
        let spec = Spec {
            a: Some(a),
            b: None,
            c: false,
            d: Some(d),
        };
        placed.retain(|(other, _)| other.id() != e.id());
        placed.push((e, spec));
    }
    let replaced: HashSet<u32> = handles.iter().map(|e| e.id()).collect();
    for (&kind, &e) in kinds.iter().zip(&targets) {
        if kind == Target::Live && !replaced.contains(&e.id()) {
            expected.push((e, before[&e]));
        }
    }
    expected.extend(placed);

    let after = fingerprint(&world);
    assert_eq!(
        after.len(),
        expected.len(),
        "world holds the wrong number of entities"
    );
    for (e, spec) in &expected {
        assert_eq!(
            after.get(e),
            Some(spec),
            "contents of {e:?} after the batch"
        );
    }
    check_archetypes(&world, "batch-at world");
    assert_eq!(
        d_live(),
        d_in(&world),
        "replaced components leaked or were dropped twice"
    );
}

/// The three ways to declare a batch's component types agree: `add::<T>()`,
/// `add_dynamic(TypeInfo::of::<T>())` — documented as "`add()` but using type
/// information determined at runtime" — and `add_bundle::<T>()`, documented as
/// including "all the components in bundle `T`".
#[hegel::test(settings())]
fn the_ways_of_declaring_a_batch_type_agree(tc: hegel::TestCase) {
    assert_d_balanced_at_start();
    let rows = draw_rows(&tc, 8);

    let mut individually = ColumnBatchType::new();
    individually.add::<A>();
    individually.add::<D>();

    let mut dynamically = ColumnBatchType::new();
    dynamically.add_dynamic(TypeInfo::of::<A>());
    dynamically.add_dynamic(TypeInfo::of::<D>());

    let mut as_bundle = ColumnBatchType::new();
    as_bundle.add_bundle::<(A, D)>();

    let mut worlds = Vec::new();
    for types in [individually, dynamically, as_bundle] {
        let mut world = World::new();
        // `ColumnBatchBuilder::new` is the non-consuming spelling of
        // `into_batch`.
        let builder = ColumnBatchBuilder::new(types, rows.len() as u32);
        {
            let mut writer = builder.writer::<A>().expect("A was declared");
            for &(a, _) in &rows {
                writer.push(A(a)).expect("push within capacity");
            }
        }
        {
            let mut writer = builder.writer::<D>().expect("D was declared");
            for &(_, d) in &rows {
                writer.push(D::new(d)).expect("push within capacity");
            }
        }
        world.spawn_column_batch(builder.build().expect("a filled batch must build"));
        worlds.push(world);
    }
    let expected = fingerprint(&worlds[0]);
    for (i, world) in worlds.iter().enumerate().skip(1) {
        assert_eq!(
            expected,
            fingerprint(world),
            "batch type {i} produced different entities"
        );
    }
}

/// A builder with an underfilled column refuses to build. All the components
/// here are `Copy`, so this checks only the `Err` half of the contract; the
/// drop half is `a_refused_build_drops_the_written_components` below.
#[hegel::test(settings())]
fn an_underfilled_batch_is_refused(tc: hegel::TestCase) {
    let capacity = tc.draw(gs::integers::<u32>().min_value(1).max_value(8));
    let written = tc.draw(gs::integers::<u32>().min_value(0).max_value(capacity - 1));

    let mut types = ColumnBatchType::new();
    types.add::<A>();
    types.add::<B>();
    let builder = types.into_batch(capacity);
    {
        let mut writer = builder.writer::<A>().expect("A is in the batch type");
        for _ in 0..capacity {
            writer.push(A(0)).expect("push within capacity");
        }
    }
    {
        let mut writer = builder.writer::<B>().expect("B is in the batch type");
        for _ in 0..written {
            writer.push(B(0)).expect("push within capacity");
        }
    }
    let Err(error) = builder.build() else {
        panic!("build accepted an underfilled column");
    };
    assert!(
        !error.to_string().is_empty(),
        "BatchIncomplete has no message"
    );
}

/// Dropping a `ColumnBatchBuilder` without building drops whatever was written
/// into it. `ColumnBatchBuilder::drop` stepped a `*mut u8` by byte index and
/// dropped it as a `u8`, leaking every written component (issue #450); this is
/// the regression witness.
#[hegel::test(settings())]
fn dropping_an_unbuilt_batch_drops_its_components(tc: hegel::TestCase) {
    assert_d_balanced_at_start();
    let capacity = tc.draw(gs::integers::<u32>().min_value(0).max_value(8));
    let written = tc.draw(gs::integers::<u32>().min_value(0).max_value(capacity));

    let mut types = ColumnBatchType::new();
    types.add::<A>();
    types.add::<D>();
    let builder = types.into_batch(capacity);
    {
        let mut writer = builder.writer::<D>().expect("D is in the batch type");
        for _ in 0..written {
            writer
                .push(D::new(tc.draw(val())))
                .expect("push within capacity");
        }
    }
    assert_eq!(
        d_live(),
        written as i64,
        "the builder is not holding what was written"
    );
    drop(builder);
    assert_eq!(
        d_live(),
        0,
        "dropping an unbuilt batch leaked {written} components"
    );
}

/// A builder with an underfilled column must refuse to build, and the values
/// already written into it must still be dropped.
///
/// `build` moves the archetype out with its length still zero and only then
/// checks completeness, so the components written so far are leaked. The `Err`
/// half of the contract holds; the drop half does not. Reproduces on hecs
/// 0.11.1 — the same defect as issue #450, on the path its fix did not cover
/// (issue #459).
#[test]
#[ignore = "hecs#459: ColumnBatchBuilder::build() -> Err leaks the written components"]
fn a_refused_build_drops_the_written_components() {
    assert_d_balanced_at_start();
    let mut types = ColumnBatchType::new();
    types.add::<A>();
    types.add::<D>();
    let builder = types.into_batch(2);
    {
        let mut writer = builder.writer::<D>().expect("D is in the batch type");
        writer.push(D::new(1)).unwrap();
        writer.push(D::new(2)).unwrap();
        // The A column is left empty, so build() must fail.
    }
    assert_eq!(d_live(), 2, "the builder is not holding what was written");
    assert!(
        builder.build().is_err(),
        "build() accepted an underfilled column"
    );
    assert_eq!(d_live(), 0, "a refused build leaked the written components");
}
