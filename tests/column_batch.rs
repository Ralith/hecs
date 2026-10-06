//! Properties of column-batch spawning: `ColumnBatchType`,
//! `ColumnBatchBuilder`, `BatchWriter`, `World::spawn_column_batch` and
//! `World::spawn_column_batch_at`.
//!
//! The oracle for a batch is the same rows spawned one at a time, plus the
//! push order the writers were given. `D` is drop-tracked, which is what makes
//! the leak properties here meaningful: this machinery moves component data
//! with raw pointer copies and owns partially initialized storage.

use std::collections::HashSet;

use fixtures::*;
use hecs::{ColumnBatch, ColumnBatchBuilder, ColumnBatchType, Entity, TypeInfo, World};
use hegel::generators as gs;
use hegel::stateful::pool;
use hegel::{DefaultGenerator, TestCase};

/// Batches spawned per state-machine run, enough that later batches merge
/// into the archetype the first one created. Size coverage comes from the
/// rows drawn per batch, not from the step count.
const STEPS: i64 = 4;

/// One entity's worth of batch data: payloads for its `A` and `D` columns.
type Row = (i32, i32);

fn row() -> impl gs::PrintableGenerator<Row> {
    hegel::tuples!(val(), val())
}

fn rows_up_to(max: usize) -> impl gs::PrintableGenerator<Vec<Row>> {
    gs::vecs(row()).max_size(max)
}

/// Push `rows` into a builder whose type declares `A` and `D`.
fn fill_batch(builder: &ColumnBatchBuilder, rows: &[Row], ds: &DropTracker) {
    let mut writer = builder.writer::<A>().expect("A is in the batch type");
    for &(a, _) in rows {
        writer.push(A(a)).expect("push within capacity");
    }
    drop(writer);
    let mut writer = builder.writer::<D>().expect("D is in the batch type");
    for &(_, d) in rows {
        writer.push(D::new(d, ds)).expect("push within capacity");
    }
}

/// Build a complete `{A, D}` batch from `rows`.
fn build_batch(rows: &[Row], ds: &DropTracker) -> ColumnBatch {
    let mut types = ColumnBatchType::new();
    types.add::<A>();
    types.add::<D>();
    let builder = types.into_batch(rows.len() as u32);
    fill_batch(&builder, rows, ds);
    builder.build().expect("a fully filled batch must build")
}

/// `writer` for a type outside the batch type is `None`, `fill` counts the
/// pushes so far, and a push past capacity hands the value back.
#[test]
fn writers_report_fill_and_reject_overflow() {
    let mut types = ColumnBatchType::new();
    types.add::<A>();
    let builder = types.into_batch(2);
    assert!(
        builder.writer::<B>().is_none(),
        "writer::<B> exists but B is not in the batch type"
    );
    let mut writer = builder.writer::<A>().expect("A is in the batch type");
    assert_eq!(writer.fill(), 0, "fill before any push");
    writer.push(A(1)).expect("push within capacity");
    writer.push(A(2)).expect("push within capacity");
    assert_eq!(writer.fill(), 2, "fill after two pushes");
    assert_eq!(
        writer.push(A(3)),
        Err(A(3)),
        "a push past capacity must return the value"
    );
}

/// A fingerprint's contents with the handles stripped, as an order-independent
/// multiset.
fn contents(fp: Fingerprint) -> Vec<Components> {
    let mut v: Vec<Components> = fp.into_values().collect();
    v.sort();
    v
}

/// The world batches spawn into and the one spawning the same rows one at a
/// time.
struct BatchVsIndividual {
    batched: World,
    individually: World,
    ds: DropTracker,
}

// Each batch is checked directly against its rows as it is spawned. The
// invariants then require the accumulated batch world to match the
// one-at-a-time world and the drop count to balance.
#[hegel::state_machine]
impl BatchVsIndividual {
    /// Spawn one drawn batch, which must hand out one distinct fresh handle
    /// per row carrying that row's components in push order, and spawn the
    /// same rows one at a time in the other world.
    #[rule]
    fn spawn_batch(&mut self, tc: TestCase) {
        let rows: Vec<Row> = tc.draw(rows_up_to(12));
        let len_before = self.batched.len();
        let mut seen: HashSet<Entity> = self.batched.iter().map(|eref| eref.entity()).collect();

        let iter = self
            .batched
            .spawn_column_batch(build_batch(&rows, &self.ds));
        assert_eq!(iter.len(), rows.len(), "SpawnColumnBatchIter::len");
        let handles: Vec<Entity> = iter.collect();
        assert_eq!(
            handles.len(),
            rows.len(),
            "SpawnColumnBatchIter yielded the wrong count"
        );
        assert_eq!(
            self.batched.len(),
            len_before + rows.len() as u32,
            "world.len() after spawn_column_batch"
        );

        for (i, (&e, &(a, d))) in handles.iter().zip(&rows).enumerate() {
            assert!(seen.insert(e), "row {i} reused handle {e:?}");
            assert!(
                self.batched.contains(e),
                "row {i} handle {e:?} is not contained"
            );
            assert_eq!(self.batched.get::<&A>(e).unwrap().0, a, "A of row {i}");
            assert_eq!(self.batched.get::<&D>(e).unwrap().value, d, "D of row {i}");
        }

        for &(a, d) in &rows {
            self.individually.spawn((A(a), D::new(d, &self.ds)));
        }
    }

    /// The worlds hold the same handles with the same multiset of contents,
    /// but not row for row: `alloc_many` hands out recycled ids in the
    /// opposite order to repeated `spawn`, so which row lands on which
    /// recycled handle differs between the worlds.
    #[invariant]
    fn worlds_agree(&self, _: TestCase) {
        let batched = fingerprint(&self.batched);
        let individually = fingerprint(&self.individually);
        assert_eq!(
            batched.keys().collect::<Vec<_>>(),
            individually.keys().collect::<Vec<_>>(),
            "the batch world and the one-at-a-time world hold different handles"
        );
        assert_eq!(
            contents(batched),
            contents(individually),
            "the batch world and the one-at-a-time world hold different entities"
        );
        check_archetypes(&self.batched, "batch world");
    }

    #[invariant]
    fn drops_balance(&self, _: TestCase) {
        assert_eq!(
            self.ds.live(),
            d_in(&self.batched) + d_in(&self.individually),
            "drop imbalance after a batch"
        );
    }
}

/// `spawn_column_batch` allocates the same set of handles and the same
/// contents as spawning the same rows one at a time, and hands out one
/// distinct handle per row carrying that row's components in push order.
///
/// The history seeds both worlds with unrelated archetypes and, when it
/// despawned something, a non-empty freelist — a state `spawn_column_batch`
/// once panicked on (fixed upstream in 8f6af23).
#[hegel::test(settings().stateful_step_count(STEPS))]
fn column_batch_matches_individual_spawns(tc: TestCase) {
    let ds = DropTracker::new();
    let history = tc.draw(histories(0, 6));
    let (mut worlds, _pool) = build_twins(&tc, &history, 2, &ds);
    let machine = BatchVsIndividual {
        individually: worlds.pop().expect("two worlds"),
        batched: worlds.pop().expect("two worlds"),
        ds,
    };
    hegel::stateful::run(machine, tc);
}

/// Whether a batch target is still live when the batch is spawned over it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, DefaultGenerator)]
enum Liveness {
    /// Live, with components the batch does not supply; they must not survive.
    Live,
    /// Spawned and then despawned; the batch resurrects the exact handle.
    Despawned,
}

/// A batch target. Every target starts with a full component set, so the
/// batch has to remove B and C as well as overwrite A and D. Drawn with its
/// derived generator: one of the `Liveness` values and any payloads.
#[derive(Clone, Copy, Debug, DefaultGenerator)]
struct Target {
    liveness: Liveness,
    a: i32,
    b: i32,
    d: i32,
}

/// `spawn_column_batch_at` places row `i` on handle `i`, replacing whatever
/// occupied that id — including the components the batch does not supply — and
/// leaves every other entity alone.
///
/// The handle list may repeat a handle. `spawn_column_batch_at_redundant` in
/// src/world.rs pins the semantics: the last row for an id wins and the earlier
/// duplicates are discarded. Repeated handles used to write out of bounds, to
/// leave zombie entities behind, and to leave the entity moved by the
/// deduplication with a stale location index.
// https://github.com/Ralith/hecs/issues/449
// https://github.com/Ralith/hecs/issues/465
#[hegel::test(settings())]
fn column_batch_at_places_each_row_on_its_handle(tc: hegel::TestCase) {
    let ds = DropTracker::new();
    let mut world = World::new();

    // Bystanders first, then targets, then despawns: every id stays distinct,
    // so a repeated handle in the batch list is the only way two rows can
    // collide.
    let bystander_bs: Vec<i32> = tc.draw(gs::vecs(val()).max_size(4));
    for &b in &bystander_bs {
        world.spawn((B(b),));
    }
    let targets: Vec<Target> = tc.draw(gs::vecs(gs::default::<Target>()).min_size(1).max_size(6));
    let mut target_handles = pool(&tc);
    let spawned: Vec<Entity> = targets
        .iter()
        .map(|t| world.spawn((A(t.a), B(t.b), C, D::new(t.d, &ds))))
        .collect();
    for (t, &e) in targets.iter().zip(&spawned) {
        if t.liveness == Liveness::Despawned {
            world.despawn(e).expect("despawn of a live target");
        }
        target_handles.add(e);
    }

    // The handle list may repeat a target; `handle_from` draws with
    // replacement, so collisions are common at this size.
    let placements: Vec<(Row, Entity)> =
        tc.draw(gs::vecs(hegel::tuples!(row(), handle_from(&target_handles))).max_size(8));
    let (rows, handles): (Vec<Row>, Vec<Entity>) = placements.iter().copied().unzip();

    // Every entity the batch does not name keeps its components, and a named
    // handle takes its last row.
    let mut expected = fingerprint(&world);
    for &((a, d), e) in &placements {
        expected.insert(
            e,
            Components {
                a: Some(a),
                d: Some(d),
                ..Components::default()
            },
        );
    }

    let d_before = ds.live();
    let batch = build_batch(&rows, &ds);
    assert_eq!(
        ds.live(),
        d_before + rows.len(),
        "the batch holds one D per row"
    );
    world.spawn_column_batch_at(&handles, batch);

    assert_eq!(
        fingerprint(&world),
        expected,
        "world contents after spawn_column_batch_at"
    );
    check_archetypes(&world, "batch-at world");
    assert_eq!(
        ds.live(),
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
    let ds = DropTracker::new();
    let rows = tc.draw(rows_up_to(8));

    let mut individually = ColumnBatchType::new();
    individually.add::<A>();
    individually.add::<D>();

    let mut dynamically = ColumnBatchType::new();
    dynamically.add_dynamic(TypeInfo::of::<A>());
    dynamically.add_dynamic(TypeInfo::of::<D>());

    let mut as_bundle = ColumnBatchType::new();
    as_bundle.add_bundle::<(A, D)>();

    let mut worlds = Vec::new();
    for (name, types) in [
        ("add", individually),
        ("add_dynamic", dynamically),
        ("add_bundle", as_bundle),
    ] {
        let mut world = World::new();
        // `ColumnBatchBuilder::new` consumes the type just as `into_batch`
        // does. Using it here covers the second spelling.
        let builder = ColumnBatchBuilder::new(types, rows.len() as u32);
        fill_batch(&builder, &rows, &ds);
        world.spawn_column_batch(builder.build().expect("a filled batch must build"));
        worlds.push((name, world));
    }
    let (_, first) = &worlds[0];
    let expected = fingerprint(first);
    for (name, world) in &worlds[1..] {
        assert_eq!(
            expected,
            fingerprint(world),
            "the batch type declared with {name} produced different entities"
        );
    }
}

/// A builder with an underfilled column refuses to build, and the values
/// already written into it are dropped.
// https://github.com/Ralith/hecs/issues/459
#[hegel::test(settings())]
fn an_underfilled_batch_is_refused(tc: hegel::TestCase) {
    let ds = DropTracker::new();
    let capacity = tc.draw(gs::integers::<u32>().min_value(1).max_value(8));
    let a_written = tc.draw(gs::integers::<u32>().max_value(capacity));
    let d_written = tc.draw(gs::integers::<u32>().max_value(capacity));
    // Discard the fully filled case: build would rightly succeed.
    tc.assume(a_written < capacity || d_written < capacity);

    let mut types = ColumnBatchType::new();
    types.add::<A>();
    types.add::<D>();
    let builder = types.into_batch(capacity);
    {
        let mut writer = builder.writer::<A>().expect("A is in the batch type");
        for _ in 0..a_written {
            writer.push(A(0)).expect("push within capacity");
        }
    }
    {
        let mut writer = builder.writer::<D>().expect("D is in the batch type");
        for _ in 0..d_written {
            writer.push(D::new(0, &ds)).expect("push within capacity");
        }
    }
    assert_eq!(
        ds.live(),
        d_written as usize,
        "the builder is not holding what was written"
    );
    assert!(
        builder.build().is_err(),
        "build accepted an underfilled column"
    );
    assert_eq!(
        ds.live(),
        0,
        "a refused build leaked the written components"
    );
}

/// Dropping a `ColumnBatchBuilder` without building drops whatever was written
/// into it. `ColumnBatchBuilder::drop` used to step a `*mut u8` by byte index
/// and drop it as a `u8`, leaking every written component.
// https://github.com/Ralith/hecs/issues/450
#[hegel::test(settings())]
fn dropping_an_unbuilt_batch_drops_its_components(tc: hegel::TestCase) {
    let ds = DropTracker::new();
    let written: Vec<i32> = tc.draw(gs::vecs(val()).max_size(8));
    let capacity = tc.draw(
        gs::integers::<u32>()
            .min_value(written.len() as u32)
            .max_value(8),
    );

    let mut types = ColumnBatchType::new();
    types.add::<A>();
    types.add::<D>();
    let builder = types.into_batch(capacity);
    {
        let mut writer = builder.writer::<D>().expect("D is in the batch type");
        for &d in &written {
            writer.push(D::new(d, &ds)).expect("push within capacity");
        }
    }
    assert_eq!(
        ds.live(),
        written.len(),
        "the builder is not holding what was written"
    );
    drop(builder);
    assert_eq!(
        ds.live(),
        0,
        "dropping an unbuilt batch leaked {} components",
        written.len()
    );
}

/// `build` used to move the archetype out before checking completeness, so a
/// refused build leaked the components written so far. This is the issue's
/// reproduction, and `an_underfilled_batch_is_refused` generalizes it.
// https://github.com/Ralith/hecs/issues/459
#[test]
fn a_refused_build_drops_the_written_components() {
    let ds = DropTracker::new();
    let mut types = ColumnBatchType::new();
    types.add::<A>();
    types.add::<D>();
    let builder = types.into_batch(2);
    {
        let mut writer = builder.writer::<D>().expect("D is in the batch type");
        writer.push(D::new(1, &ds)).unwrap();
        writer.push(D::new(2, &ds)).unwrap();
    }
    assert_eq!(ds.live(), 2, "the builder is not holding what was written");
    assert!(
        builder.build().is_err(),
        "build() accepted an underfilled column"
    );
    assert_eq!(
        ds.live(),
        0,
        "a refused build leaked the written components"
    );
}

/// Dropping an unbuilt batch whose type includes the zero-sized `C` is
/// undefined behavior: `ColumnBatchBuilder::drop` allocates a scratch buffer
/// with each column's layout, and `alloc` requires a non-zero size. Miri
/// rejects it, and the serialize tests keep their Miri worlds `C`-free for
/// the same reason.
// https://github.com/Ralith/hecs/issues/467
#[test]
#[ignore = "hecs#467: ColumnBatchBuilder::drop allocates a zero-size layout for a ZST column"]
fn dropping_an_unbuilt_batch_with_a_zst_column_is_sound() {
    let mut types = ColumnBatchType::new();
    types.add::<C>();
    drop(types.into_batch(1));
}
