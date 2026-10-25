//! Every public way of reading a `World` must project the same state.
//!
//! Ground truth is the observational fingerprint, taken once with plain
//! per-entity reads. Each property then drives one read shape — a query
//! combinator, a batched iterator, a random-access view, a `Ref` projection,
//! a whole-column archetype read — and asserts it yields exactly the entities
//! and values the fingerprint predicts, with no duplicates.

use std::collections::BTreeMap;

use fixtures::*;
use hecs::{Entity, Or, PreparedQuery, Ref, RefMut, Satisfies, World};
use hegel::generators as gs;

/// The `A` values the fingerprint predicts, keyed by entity.
fn expected_a(fp: &Fingerprint) -> BTreeMap<Entity, i32> {
    fp.iter()
        .filter_map(|(&e, o)| o.a.map(|v| (e, v)))
        .collect()
}

/// Collect `(Entity, value)` pairs, failing on a repeated entity.
fn collect<V>(it: impl Iterator<Item = (Entity, V)>, label: &str) -> BTreeMap<Entity, V> {
    let mut got = BTreeMap::new();
    for (e, v) in it {
        assert!(got.insert(e, v).is_none(), "{label} yielded {e:?} twice");
    }
    got
}

/// `Satisfies<Q>` matches every entity and reports whether it satisfies `Q`,
/// unlike `Q` itself, which filters. It must agree with component presence even
/// for entities with no components at all.
#[hegel::test(settings())]
fn satisfies_reports_query_matching_for_every_entity(tc: hegel::TestCase) {
    let ds = DropTracker::new();
    let history = tc.draw(histories(0, MAX_ENTITIES));
    let world = build_world(&history, &ds);
    let fp = fingerprint(&world);

    let got = collect(
        world
            .query::<(Entity, Satisfies<&A>, Satisfies<(&A, &B)>)>()
            .iter()
            .map(|(e, one, both)| (e, (one, both))),
        "Satisfies",
    );
    let want: BTreeMap<Entity, (bool, bool)> = fp
        .iter()
        .map(|(&e, o)| (e, (o.a.is_some(), o.a.is_some() && o.b.is_some())))
        .collect();
    assert_eq!(got, want, "query::<Satisfies<..>>");
}

/// A `&mut` fetch through the dynamically borrow-checked `query()` path reads
/// the current values and its writes are visible afterwards. `QueryIter` is an
/// `ExactSizeIterator`, so its length must be the number of matches.
#[hegel::test(settings())]
fn writes_through_a_unique_query_are_visible(tc: hegel::TestCase) {
    let ds = DropTracker::new();
    let history = tc.draw(histories(0, MAX_ENTITIES));
    let world = build_world(&history, &ds);
    let mut fp = fingerprint(&world);
    let v = tc.draw(val());

    {
        let mut q = world.query::<(Entity, &mut A)>();
        let mut it = q.iter();
        let matches = expected_a(&fp).len();
        assert_eq!(it.len(), matches, "QueryIter::len");
        assert_eq!(
            it.size_hint(),
            (matches, Some(matches)),
            "QueryIter::size_hint"
        );
        for (e, a) in &mut it {
            assert_eq!(
                Some(a.0),
                fp[&e].a,
                "query::<&mut A> read a stale value for {e:?}"
            );
            a.0 = v;
        }
    }
    // From here on `fp` predicts the post-write state.
    for o in fp.values_mut() {
        if o.a.is_some() {
            o.a = Some(v);
        }
    }
    assert_eq!(
        fingerprint(&world),
        fp,
        "writes through query::<&mut A> were lost"
    );
}

/// Every `Or` accessor reports the same thing about which side is present:
/// `split`, `left`, `right`, and `cloned().as_mut()`. The matched set is
/// exactly the entities holding an `A` or a `B`, with no duplicates.
#[hegel::test(settings())]
fn or_accessors_agree_with_the_matched_variant(tc: hegel::TestCase) {
    let ds = DropTracker::new();
    let history = tc.draw(histories(0, MAX_ENTITIES));
    let world = build_world(&history, &ds);
    let fp = fingerprint(&world);

    let got = collect(
        world.query::<(Entity, Or<&A, &B>)>().iter().map(|(e, or)| {
            let (left, right) = or.split();
            let a = left.map(|a| a.0);
            let b = right.map(|b| b.0);
            assert_eq!(or.left().map(|a| a.0), a, "Or::left for {e:?}");
            assert_eq!(or.right().map(|b| b.0), b, "Or::right for {e:?}");
            let mut owned: Or<A, B> = or.cloned();
            let (left, right) = owned.as_mut().split();
            assert_eq!(left.map(|a| a.0), a, "Or::cloned().as_mut() left for {e:?}");
            assert_eq!(
                right.map(|b| b.0),
                b,
                "Or::cloned().as_mut() right for {e:?}"
            );
            (e, (a, b))
        }),
        "query::<Or<&A, &B>>",
    );
    let want: BTreeMap<Entity, (Option<i32>, Option<i32>)> = fp
        .iter()
        .filter(|(_, o)| o.a.is_some() || o.b.is_some())
        .map(|(&e, o)| (e, (o.a, o.b)))
        .collect();
    assert_eq!(got, want, "query::<Or<&A, &B>> matched the wrong entities");
}

/// `with` and `without` filter by the presence of a component without
/// borrowing it, identically on the shared-borrow (`QueryBorrow`) and
/// unique-borrow (`QueryMut`) paths.
#[hegel::test(settings())]
fn query_filters_select_by_component_presence(tc: hegel::TestCase) {
    let ds = DropTracker::new();
    let history = tc.draw(histories(0, MAX_ENTITIES));
    let mut world = build_world(&history, &ds);
    let fp = fingerprint(&world);

    let with_b: BTreeMap<Entity, i32> = fp
        .iter()
        .filter_map(|(&e, o)| match (o.a, o.b) {
            (Some(v), Some(_)) => Some((e, v)),
            _ => None,
        })
        .collect();
    let without_b: BTreeMap<Entity, i32> = fp
        .iter()
        .filter_map(|(&e, o)| match (o.a, o.b) {
            (Some(v), None) => Some((e, v)),
            _ => None,
        })
        .collect();

    let got = collect(
        world
            .query::<(Entity, &A)>()
            .with::<&B>()
            .iter()
            .map(|(e, a)| (e, a.0)),
        "QueryBorrow::with",
    );
    assert_eq!(got, with_b, "QueryBorrow::with::<&B>");

    let got = collect(
        world
            .query::<(Entity, &A)>()
            .without::<&B>()
            .iter()
            .map(|(e, a)| (e, a.0)),
        "QueryBorrow::without",
    );
    assert_eq!(got, without_b, "QueryBorrow::without::<&B>");

    let got = collect(
        world
            .query_mut::<(Entity, &A)>()
            .with::<&B>()
            .into_iter()
            .map(|(e, a)| (e, a.0)),
        "QueryMut::with",
    );
    assert_eq!(got, with_b, "QueryMut::with::<&B>");

    let got = collect(
        world
            .query_mut::<(Entity, &A)>()
            .without::<&B>()
            .into_iter()
            .map(|(e, a)| (e, a.0)),
        "QueryMut::without",
    );
    assert_eq!(got, without_b, "QueryMut::without::<&B>");
}

/// Batched iteration partitions the matches: taken together the batches visit
/// exactly what flat iteration does, whatever the batch size.
#[hegel::test(settings())]
fn batched_iteration_partitions_the_matches(tc: hegel::TestCase) {
    let ds = DropTracker::new();
    let history = tc.draw(histories(0, MAX_ENTITIES));
    let mut world = build_world(&history, &ds);
    let want = expected_a(&fingerprint(&world));
    // `iter_batched` requires a batch size greater than 0; sizes up to 4 give
    // both ragged and exact partitions of archetypes this small.
    let size = tc.draw(gs::integers::<u32>().min_value(1).max_value(4));

    let got = collect(
        world
            .query::<(Entity, &A)>()
            .iter_batched(size)
            .flatten()
            .map(|(e, a)| (e, a.0)),
        "iter_batched",
    );
    assert_eq!(got, want, "QueryBorrow::iter_batched({size})");

    let got = collect(
        world
            .query_mut::<(Entity, &A)>()
            .into_iter_batched(size)
            .flatten()
            .map(|(e, a)| (e, a.0)),
        "into_iter_batched",
    );
    assert_eq!(got, want, "QueryMut::into_iter_batched({size})");
}

/// Iterating a view — `View`, `ViewBorrow` or `PreparedView` — visits the same
/// matches as a query.
#[hegel::test(settings())]
fn view_iteration_visits_every_match(tc: hegel::TestCase) {
    let ds = DropTracker::new();
    let history = tc.draw(histories(0, MAX_ENTITIES));
    let mut world = build_world(&history, &ds);
    let want = expected_a(&fingerprint(&world));

    {
        let mut view = world.view_mut::<(Entity, &mut A)>();
        let got = collect((&mut view).into_iter().map(|(e, a)| (e, a.0)), "&mut View");
        assert_eq!(got, want, "IntoIterator for &mut View");
        let got = collect(view.iter_mut().map(|(e, a)| (e, a.0)), "View::iter_mut");
        assert_eq!(got, want, "View::iter_mut");
    }
    {
        let mut view = world.view::<(Entity, &A)>();
        let got = collect(
            (&mut view).into_iter().map(|(e, a)| (e, a.0)),
            "&mut ViewBorrow",
        );
        assert_eq!(got, want, "IntoIterator for &mut ViewBorrow");
    }
    {
        let mut prepared = PreparedQuery::<(Entity, &A)>::default();
        let mut view = prepared.view_mut(&mut world);
        let got = collect(
            (&mut view).into_iter().map(|(e, a)| (e, a.0)),
            "&mut PreparedView",
        );
        assert_eq!(got, want, "IntoIterator for &mut PreparedView");
        let got = collect(
            view.iter_mut().map(|(e, a)| (e, a.0)),
            "PreparedView::iter_mut",
        );
        assert_eq!(got, want, "PreparedView::iter_mut");
    }
}

/// Reaching an entity by handle through a view agrees with iterating: `get`,
/// `get_mut`, `get_unchecked` and `get_disjoint_mut` all resolve the same
/// component, and miss exactly the entities the query does not match.
#[hegel::test(settings())]
fn random_access_views_agree_with_iteration(tc: hegel::TestCase) {
    let ds = DropTracker::new();
    let history = tc.draw(histories(1, MAX_ENTITIES));
    let (mut world, pool) = build_world_with_handles(&tc, &history, &ds);
    let fp = fingerprint(&world);
    // Either draw may name a dead entity, so the expectations below go
    // through `fp.get(..)`. `get_disjoint_mut` asserts its handles are
    // distinct, so a collided draw rejects the case.
    let e1 = tc.draw(handle_from(&pool));
    let e2 = tc.draw(handle_from(&pool));
    tc.assume(e1 != e2);

    {
        let mut q = world.query::<&A>();
        let view = q.view();
        for (&e, o) in &fp {
            assert_eq!(
                view.get(e).map(|a| a.0),
                o.a,
                "QueryBorrow::view().get({e:?})"
            );
        }
    }
    {
        let mut q = world.query_mut::<&A>();
        let view = q.view();
        for (&e, o) in &fp {
            assert_eq!(view.get(e).map(|a| a.0), o.a, "QueryMut::view().get({e:?})");
        }
    }
    {
        let mut view = world.view::<(Entity, &A)>();
        for (&e, o) in &fp {
            assert_eq!(
                view.get_mut(e).map(|(got, a)| (got, a.0)),
                o.a.map(|v| (e, v)),
                "ViewBorrow::get_mut({e:?})"
            );
            // SAFETY: the query yields only shared references and no unique
            // borrow of A is alive here.
            let unchecked = unsafe { view.get_unchecked(e) };
            assert_eq!(
                unchecked.map(|(got, a)| (got, a.0)),
                o.a.map(|v| (e, v)),
                "ViewBorrow::get_unchecked({e:?})"
            );
        }
        let [first, second] = view.get_disjoint_mut([e1, e2]);
        assert_eq!(
            first.map(|(_, a)| a.0),
            fp.get(&e1).and_then(|o| o.a),
            "ViewBorrow::get_disjoint_mut({e1:?})"
        );
        assert_eq!(
            second.map(|(_, a)| a.0),
            fp.get(&e2).and_then(|o| o.a),
            "ViewBorrow::get_disjoint_mut({e2:?})"
        );
    }
    {
        let mut prepared = PreparedQuery::<(Entity, &A)>::new();
        let mut view = prepared.view_mut(&mut world);
        for (&e, o) in &fp {
            assert_eq!(
                view.get_mut(e).map(|(got, a)| (got, a.0)),
                o.a.map(|v| (e, v)),
                "PreparedView::get_mut({e:?})"
            );
            // SAFETY: as above, the query is read-only and nothing else borrows A.
            let unchecked = unsafe { view.get_unchecked(e) };
            assert_eq!(
                unchecked.map(|(got, a)| (got, a.0)),
                o.a.map(|v| (e, v)),
                "PreparedView::get_unchecked({e:?})"
            );
        }
        let [first, second] = view.get_disjoint_mut([e1, e2]);
        assert_eq!(
            first.map(|(_, a)| a.0),
            fp.get(&e1).and_then(|o| o.a),
            "PreparedView::get_disjoint_mut({e1:?})"
        );
        assert_eq!(
            second.map(|(_, a)| a.0),
            fp.get(&e2).and_then(|o| o.a),
            "PreparedView::get_disjoint_mut({e2:?})"
        );
    }

    assert_eq!(
        fingerprint(&world),
        fp,
        "a read-only shape mutated the world"
    );
}

/// `Ref` and `RefMut` are transparent handles on a component: projecting with
/// `map` reads the same value, and a write through a projected `RefMut` lands
/// in the world.
#[hegel::test(settings())]
fn ref_projections_read_and_write_the_component(tc: hegel::TestCase) {
    let ds = DropTracker::new();
    let initial = tc.draw(val());
    let written = tc.draw(val());
    let mut world = World::new();
    let e = world.spawn((A(initial), D::new(initial, &ds)));

    {
        let shared: Ref<'_, A> = world.get::<&A>(e).expect("A was just spawned");
        assert_eq!(
            format!("{shared:?}"),
            format!("{:?}", A(initial)),
            "Ref forwards Debug"
        );
        let copy = shared.clone();
        let projected: Ref<'_, i32> = Ref::map(shared, |a| &a.0);
        assert_eq!(*projected, initial, "Ref::map read a different value");
        assert_eq!(
            copy.0, initial,
            "a cloned Ref was disturbed by mapping the original"
        );
    }
    {
        let unique: RefMut<'_, A> = world.get::<&mut A>(e).expect("A was just spawned");
        let mut projected: RefMut<'_, i32> = RefMut::map(unique, |a| &mut a.0);
        *projected = written;
    }
    assert_eq!(
        world.get::<&A>(e).unwrap().0,
        written,
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
