//! Every public way of reading a `World` must project the same state.
//!
//! Ground truth is the observational fingerprint, taken once with plain
//! per-entity reads. Each property then drives one read shape — a query
//! combinator, a batched iterator, a random-access view — and asserts it
//! yields exactly the entities and values the fingerprint predicts, with no
//! duplicates.

mod common;

use std::collections::BTreeMap;

use common::*;
use hecs::{Entity, Or, PreparedQuery, Satisfies};
use hegel::generators as gs;

/// The `A` values the fingerprint predicts, keyed by entity.
fn expected_a(fp: &Fingerprint) -> BTreeMap<Entity, i32> {
    fp.iter()
        .filter_map(|(&e, o)| o.a.map(|v| (e, v)))
        .collect()
}

/// Collect `(Entity, value)` pairs, failing on a repeated entity.
fn collect(it: impl Iterator<Item = (Entity, i32)>, label: &str) -> BTreeMap<Entity, i32> {
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
    let history = tc.draw(histories(0, MAX_ENTITIES));
    let (worlds, _pool) = build_twins(&history, 1);
    let fp = fingerprint(&worlds[0]);

    let mut got: BTreeMap<Entity, (bool, bool)> = BTreeMap::new();
    for (e, one, both) in worlds[0]
        .query::<(Entity, Satisfies<&A>, Satisfies<(&A, &B)>)>()
        .iter()
    {
        assert!(
            got.insert(e, (one, both)).is_none(),
            "Satisfies yielded {e:?} twice"
        );
    }
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
    let history = tc.draw(histories(0, MAX_ENTITIES));
    let (mut worlds, _pool) = build_twins(&history, 1);
    let world = &mut worlds[0];
    let mut fp = fingerprint(world);
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
    for o in fp.values_mut() {
        if o.a.is_some() {
            o.a = Some(v);
        }
    }
    assert_eq!(
        fingerprint(world),
        fp,
        "writes through query::<&mut A> were lost"
    );
}

/// Every `Or` accessor reports the same thing about which side is present:
/// `split`, `left`, `right`, and `cloned().as_mut()`.
#[hegel::test(settings())]
fn or_accessors_agree_with_the_matched_variant(tc: hegel::TestCase) {
    let history = tc.draw(histories(0, MAX_ENTITIES));
    let (worlds, _pool) = build_twins(&history, 1);
    let fp = fingerprint(&worlds[0]);

    let mut seen = 0usize;
    for (e, or) in worlds[0].query::<(Entity, Or<&A, &B>)>().iter() {
        let o = fp[&e];
        assert!(
            o.a.is_some() || o.b.is_some(),
            "Or matched {e:?} with neither A nor B"
        );
        let (left, right) = or.split();
        assert_eq!(left.map(|a| a.0), o.a, "Or::split left for {e:?}");
        assert_eq!(right.map(|b| b.0), o.b, "Or::split right for {e:?}");
        assert_eq!(or.left().map(|a| a.0), o.a, "Or::left for {e:?}");
        assert_eq!(or.right().map(|b| b.0), o.b, "Or::right for {e:?}");
        let mut owned: Or<A, B> = or.cloned();
        let (left, right) = owned.as_mut().split();
        assert_eq!(
            left.map(|a| a.0),
            o.a,
            "Or::cloned().as_mut() left for {e:?}"
        );
        assert_eq!(
            right.map(|b| b.0),
            o.b,
            "Or::cloned().as_mut() right for {e:?}"
        );
        seen += 1;
    }
    let want = fp
        .values()
        .filter(|o| o.a.is_some() || o.b.is_some())
        .count();
    assert_eq!(seen, want, "query::<Or<&A, &B>> matched the wrong entities");
}

/// `with` and `without` filter by the presence of a component without
/// borrowing it, identically on the shared-borrow (`QueryBorrow`) and
/// unique-borrow (`QueryMut`) paths.
#[hegel::test(settings())]
fn query_filters_select_by_component_presence(tc: hegel::TestCase) {
    let history = tc.draw(histories(0, MAX_ENTITIES));
    let (mut worlds, _pool) = build_twins(&history, 1);
    let world = &mut worlds[0];
    let fp = fingerprint(world);

    let with_b: BTreeMap<Entity, i32> = fp
        .iter()
        .filter_map(|(&e, o)| o.b.and(o.a).map(|v| (e, v)))
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
    let history = tc.draw(histories(0, MAX_ENTITIES));
    let (mut worlds, _pool) = build_twins(&history, 1);
    let world = &mut worlds[0];
    let want = expected_a(&fingerprint(world));
    // A batch size of 0 is documented to panic.
    let size = tc.draw(gs::integers::<u32>().min_value(1).max_value(4));

    let mut got = BTreeMap::new();
    for batch in world.query::<(Entity, &A)>().iter_batched(size) {
        for (e, a) in batch {
            assert!(
                got.insert(e, a.0).is_none(),
                "iter_batched yielded {e:?} twice"
            );
        }
    }
    assert_eq!(got, want, "QueryBorrow::iter_batched({size})");

    let mut got = BTreeMap::new();
    for batch in world.query_mut::<(Entity, &A)>().into_iter_batched(size) {
        for (e, a) in batch {
            assert!(
                got.insert(e, a.0).is_none(),
                "into_iter_batched yielded {e:?} twice"
            );
        }
    }
    assert_eq!(got, want, "QueryMut::into_iter_batched({size})");
}

/// Iterating a view — `View`, `ViewBorrow` or `PreparedView` — visits the same
/// matches as a query.
#[hegel::test(settings())]
fn view_iteration_visits_every_match(tc: hegel::TestCase) {
    let history = tc.draw(histories(0, MAX_ENTITIES));
    let (mut worlds, _pool) = build_twins(&history, 1);
    let world = &mut worlds[0];
    let want = expected_a(&fingerprint(world));

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
        let mut view = prepared.view_mut(world);
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
    let history = tc.draw(histories(1, MAX_ENTITIES));
    let (mut worlds, pool) = build_twins(&history, 1);
    let world = &mut worlds[0];
    let fp = fingerprint(world);
    let e1 = tc.draw(pick(&pool));
    let e2 = tc.draw(pick(&pool));

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
        if e1 != e2 {
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
    }
    {
        let mut prepared = PreparedQuery::<(Entity, &A)>::new();
        let mut view = prepared.view_mut(world);
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
        if e1 != e2 {
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
    }

    assert_eq!(
        fingerprint(world),
        fp,
        "a read-only shape mutated the world"
    );
}
