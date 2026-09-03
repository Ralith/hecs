//! Model-based property test for `World`: a drawn sequence of operations is
//! applied to both a `World` and a `HashMap<Entity, Spec>` reference model, and
//! the two must agree.
//!
//! Each rule asserts the postcondition it establishes (the operation's
//! `Ok`/`Err` against modelled liveness, and the touched entity's resulting
//! component set). The global oracles — full bidirectional equivalence, the
//! archetype partition, the drop count, and the query surface — are invariants,
//! which hegel runs in full on the initial and final state and, after each
//! step, each with probability `1 / STEPS`, so about once more per case.
//!
//! The handle pool deliberately retains despawned handles, so operations
//! against dead entities are common and their error paths are exercised.

mod common;

use std::collections::HashMap;

use common::*;
use hecs::{Entity, EntityBuilderClone, Or, PreparedQuery, QueryOneError, With, Without, World};
use hegel::generators::{self as gs, Generator};
use hegel::stateful::{pool, Pool};
use hegel::TestCase;

#[cfg(miri)]
const STEPS: i64 = 8;
#[cfg(not(miri))]
const STEPS: i64 = 150;

struct WorldModel {
    world: World,
    model: HashMap<Entity, Spec>,
    /// Every handle ever handed out, including despawned ones.
    handles: Pool<Entity>,
    /// Handles from `reserve_entity`/`reserve_entities` awaiting a flush.
    reserved: Vec<Entity>,
    /// Handles whose entity was destroyed. `despawn` reuses the id but
    /// advances the generation, so none of these may resolve again; `clear`
    /// and `spawn_at` are the documented exceptions.
    retired: Vec<Entity>,
}

impl WorldModel {
    /// Mirror hecs's implicit flush: reserved handles become componentless
    /// entities. Called by every rule that runs an operation documented to
    /// flush (all variations of spawn, despawn, insert and remove).
    fn flush_model(&mut self) {
        for e in self.reserved.drain(..) {
            self.model.insert(e, Spec::default());
        }
    }

    fn draw_handle(&self, tc: &TestCase) -> Entity {
        *tc.draw(self.handles.values_reusable().print_as_debug())
    }

    /// The component set the model predicts for `e`, or `None` if dead.
    fn expect(&self, e: Entity) -> Option<Spec> {
        self.model.get(&e).copied()
    }

    /// Everything observable about `e` right now.
    fn observe(&self, e: Entity) -> Option<Spec> {
        let eref = self.world.entity(e).ok()?;
        Some(Spec {
            a: eref.get::<&A>().map(|r| r.0),
            b: eref.get::<&B>().map(|r| r.0),
            c: eref.get::<&C>().is_some(),
            d: eref.get::<&D>().map(|r| r.0),
        })
    }

    fn check_entity(&self, e: Entity, label: &str) {
        // An unflushed handle resolves to the empty archetype rather than to an
        // entity, so there is nothing to compare against the model yet.
        if self.reserved.contains(&e) {
            return;
        }
        assert_eq!(self.observe(e), self.expect(e), "{label}: {e:?}");
    }
}

// Each case applies up to `STEPS` rules, chosen at random, to the machine
// built in `world_matches_model` below.
#[hegel::state_machine]
impl WorldModel {
    /// `spawn` reports a fresh handle carrying exactly the bundle's components,
    /// and `EntityBuilder`'s introspection agrees with what was added.
    #[rule]
    fn spawn(&mut self, tc: TestCase) {
        self.flush_model();
        let s = tc.draw(specs());
        let mut builder = make_builder(s);
        assert_eq!(builder.has::<A>(), s.a.is_some(), "builder.has::<A>");
        assert_eq!(builder.has::<D>(), s.d.is_some(), "builder.has::<D>");
        assert_eq!(builder.get::<&A>().map(|r| r.0), s.a, "builder.get::<&A>");
        assert_eq!(
            builder.component_types().count(),
            s.component_count(),
            "builder.component_types"
        );
        let e = self.world.spawn(builder.build());
        assert!(
            self.model.insert(e, s).is_none(),
            "spawn reused a live handle {e:?}"
        );
        self.handles.add(e);
        self.check_entity(e, "spawn");
    }

    /// `spawn_at` makes exactly `handle` live, destroying whatever entity
    /// shared its id.
    #[rule]
    fn spawn_at(&mut self, tc: TestCase) {
        self.flush_model();
        let handle = self.draw_handle(&tc);
        let s = tc.draw(specs());
        let mut builder = make_builder(s);
        self.world.spawn_at(handle, builder.build());
        self.model.retain(|k, _| k.id() != handle.id());
        self.model.insert(handle, s);
        self.retired.retain(|r| r.id() != handle.id());
        self.check_entity(handle, "spawn_at");
        assert!(
            self.world.contains(handle),
            "spawn_at target is not contained"
        );
    }

    /// `spawn_at` at an id past the end of the metadata table grows it and
    /// leaves the intervening ids reusable. The offset is small to bound the
    /// test's memory, not because hecs limits it.
    #[rule]
    fn spawn_at_fresh_id(&mut self, tc: TestCase) {
        self.flush_model();
        let max_id = self.model.keys().map(|e| e.id()).max().unwrap_or(0);
        let offset = tc.draw(gs::integers::<u32>().min_value(1).max_value(8));
        let generation = tc.draw(gs::integers::<u32>().min_value(1).max_value(3));
        let bits = (u64::from(generation) << 32) | u64::from(max_id + offset);
        let handle = Entity::from_bits(bits).expect("nonzero generation");
        assert_eq!(
            handle.to_bits().get(),
            bits,
            "to_bits is not the inverse of from_bits"
        );
        let s = tc.draw(specs());
        let mut builder = make_builder(s);
        self.world.spawn_at(handle, builder.build());
        self.model.retain(|k, _| k.id() != handle.id());
        self.model.insert(handle, s);
        self.retired.retain(|r| r.id() != handle.id());
        self.handles.add(handle);
        self.check_entity(handle, "spawn_at_fresh_id");
    }

    /// `despawn` succeeds exactly for live handles and leaves the handle dead.
    #[rule]
    fn despawn(&mut self, tc: TestCase) {
        self.flush_model();
        let e = self.draw_handle(&tc);
        let ok = self.world.despawn(e).is_ok();
        assert_eq!(
            ok,
            self.model.remove(&e).is_some(),
            "despawn disagreed for {e:?}"
        );
        if ok {
            assert!(
                !self.world.contains(e),
                "despawned {e:?} is still contained"
            );
            self.retired.push(e);
        }
        self.check_entity(e, "despawn");
    }

    /// `insert_one` succeeds exactly for live handles, and overwrites any
    /// existing component of that type.
    #[rule]
    fn insert_one(&mut self, tc: TestCase) {
        self.flush_model();
        let e = self.draw_handle(&tc);
        let live = self.model.contains_key(&e);
        let v = tc.draw(val());
        let ok = match tc.draw(gs::integers::<u8>().min_value(0).max_value(3)) {
            0 => {
                let ok = self.world.insert_one(e, A(v)).is_ok();
                if let Some(m) = self.model.get_mut(&e) {
                    m.a = Some(v);
                }
                ok
            }
            1 => {
                let ok = self.world.insert_one(e, B(v)).is_ok();
                if let Some(m) = self.model.get_mut(&e) {
                    m.b = Some(v);
                }
                ok
            }
            2 => {
                let ok = self.world.insert_one(e, C).is_ok();
                if let Some(m) = self.model.get_mut(&e) {
                    m.c = true;
                }
                ok
            }
            // On a dead target the component is dropped rather than stored, so
            // the drop count stays balanced either way.
            _ => {
                let ok = self.world.insert_one(e, D::new(v)).is_ok();
                if let Some(m) = self.model.get_mut(&e) {
                    m.d = Some(v);
                }
                ok
            }
        };
        assert_eq!(ok, live, "insert_one disagreed for {e:?}");
        self.check_entity(e, "insert_one");
    }

    /// `insert` of a multi-component bundle adds every member in one migration.
    #[rule]
    fn insert_bundle(&mut self, tc: TestCase) {
        self.flush_model();
        let e = self.draw_handle(&tc);
        let live = self.model.contains_key(&e);
        let s = tc.draw(specs());
        let mut builder = make_builder(s);
        let ok = self.world.insert(e, builder.build()).is_ok();
        assert_eq!(ok, live, "insert bundle disagreed for {e:?}");
        if let Some(m) = self.model.get_mut(&e) {
            m.a = s.a.or(m.a);
            m.b = s.b.or(m.b);
            m.c |= s.c;
            m.d = s.d.or(m.d);
        }
        self.check_entity(e, "insert_bundle");
    }

    /// `remove_one` succeeds exactly when the component was present, and
    /// returns the value it removed.
    #[rule]
    fn remove_one(&mut self, tc: TestCase) {
        self.flush_model();
        let e = self.draw_handle(&tc);
        let before = self.expect(e);
        match tc.draw(gs::integers::<u8>().min_value(0).max_value(3)) {
            0 => {
                let got = self.world.remove_one::<A>(e).ok();
                assert_eq!(
                    got.map(|A(v)| v),
                    before.and_then(|s| s.a),
                    "remove_one::<A> {e:?}"
                );
                if let Some(m) = self.model.get_mut(&e) {
                    m.a = None;
                }
            }
            1 => {
                let got = self.world.remove_one::<B>(e).ok();
                assert_eq!(
                    got.map(|B(v)| v),
                    before.and_then(|s| s.b),
                    "remove_one::<B> {e:?}"
                );
                if let Some(m) = self.model.get_mut(&e) {
                    m.b = None;
                }
            }
            2 => {
                let ok = self.world.remove_one::<C>(e).is_ok();
                assert_eq!(ok, before.is_some_and(|s| s.c), "remove_one::<C> {e:?}");
                if let Some(m) = self.model.get_mut(&e) {
                    m.c = false;
                }
            }
            // The removed D is returned and dropped here, matching the model.
            _ => {
                let got = self.world.remove_one::<D>(e).ok();
                assert_eq!(
                    got.as_ref().map(|d| d.0),
                    before.and_then(|s| s.d),
                    "remove_one::<D> {e:?}"
                );
                if let Some(m) = self.model.get_mut(&e) {
                    m.d = None;
                }
            }
        }
        self.check_entity(e, "remove_one");
    }

    /// `remove` of a bundle is all-or-nothing: if any member is missing it
    /// returns `Err` and removes nothing.
    #[rule]
    fn remove_bundle(&mut self, tc: TestCase) {
        self.flush_model();
        let e = self.draw_handle(&tc);
        let before = self.expect(e);
        if tc.draw(gs::booleans()) {
            let had = before.is_some_and(|s| s.a.is_some() && s.b.is_some());
            assert_eq!(
                self.world.remove::<(A, B)>(e).is_ok(),
                had,
                "remove::<(A,B)> {e:?}"
            );
            if had {
                let m = self.model.get_mut(&e).unwrap();
                m.a = None;
                m.b = None;
            }
        } else {
            let had = before.is_some_and(|s| s.c && s.d.is_some());
            assert_eq!(
                self.world.remove::<(C, D)>(e).is_ok(),
                had,
                "remove::<(C,D)> {e:?}"
            );
            if had {
                let m = self.model.get_mut(&e).unwrap();
                m.c = false;
                m.d = None;
            }
        }
        self.check_entity(e, "remove_bundle");
    }

    /// `exchange_one` needs the outgoing component present, returns it, and
    /// leaves the incoming one in place.
    #[rule]
    fn exchange_one(&mut self, tc: TestCase) {
        self.flush_model();
        let e = self.draw_handle(&tc);
        let before = self.expect(e);
        let v = tc.draw(val());
        if tc.draw(gs::booleans()) {
            let got = self.world.exchange_one::<A, B>(e, B(v)).ok();
            assert_eq!(
                got.map(|A(x)| x),
                before.and_then(|s| s.a),
                "exchange A->B {e:?}"
            );
            if got.is_some() {
                let m = self.model.get_mut(&e).unwrap();
                m.a = None;
                m.b = Some(v);
            }
        } else {
            let got = self.world.exchange_one::<D, A>(e, A(v)).ok();
            assert_eq!(
                got.as_ref().map(|d| d.0),
                before.and_then(|s| s.d),
                "exchange D->A {e:?}"
            );
            if got.is_some() {
                let m = self.model.get_mut(&e).unwrap();
                m.d = None;
                m.a = Some(v);
            }
        }
        self.check_entity(e, "exchange_one");
    }

    /// A write through `get::<&mut T>` is visible to a subsequent read. This is
    /// a read path, so it does not flush.
    #[rule]
    fn mutate_in_place(&mut self, tc: TestCase) {
        let e = self.draw_handle(&tc);
        let v = tc.draw(val());
        match tc.draw(gs::integers::<u8>().min_value(0).max_value(2)) {
            0 => {
                if let Ok(mut a) = self.world.get::<&mut A>(e) {
                    a.0 = v;
                }
                if let Some(m) = self.model.get_mut(&e).filter(|m| m.a.is_some()) {
                    m.a = Some(v);
                }
            }
            1 => {
                if let Ok(mut b) = self.world.get::<&mut B>(e) {
                    b.0 = v;
                }
                if let Some(m) = self.model.get_mut(&e).filter(|m| m.b.is_some()) {
                    m.b = Some(v);
                }
            }
            // Editing D's payload constructs and drops nothing.
            _ => {
                if let Ok(mut d) = self.world.get::<&mut D>(e) {
                    d.0 = v;
                }
                if let Some(m) = self.model.get_mut(&e).filter(|m| m.d.is_some()) {
                    m.d = Some(v);
                }
            }
        }
        self.check_entity(e, "mutate_in_place");
    }

    /// A `query_mut` sweep reaches every entity with an `A`, and nothing else.
    #[rule]
    fn sweep_query_mut(&mut self, tc: TestCase) {
        let v = tc.draw(val());
        let mut swept = 0usize;
        for a in self.world.query_mut::<&mut A>() {
            a.0 = v;
            swept += 1;
        }
        let expected = self.model.values().filter(|m| m.a.is_some()).count();
        assert_eq!(
            swept, expected,
            "query_mut::<&mut A> visited the wrong number of entities"
        );
        for m in self.model.values_mut() {
            if m.a.is_some() {
                m.a = Some(v);
            }
        }
    }

    /// Two distinct handles can be fetched concurrently, and each result
    /// matches the model.
    #[rule]
    fn query_disjoint_mut(&mut self, tc: TestCase) {
        let e1 = self.draw_handle(&tc);
        let e2 = self.draw_handle(&tc);
        // `query_disjoint_mut` documents a panic on repeated handles.
        tc.assume(e1 != e2);
        let (v1, v2) = (tc.draw(val()), tc.draw(val()));
        let (got1, got2);
        {
            let [r1, r2] = self.world.query_disjoint_mut::<&mut A, 2>([e1, e2]);
            got1 = r1
                .map(|a| {
                    let old = a.0;
                    a.0 = v1;
                    old
                })
                .ok();
            got2 = r2
                .map(|a| {
                    let old = a.0;
                    a.0 = v2;
                    old
                })
                .ok();
        }
        assert_eq!(
            got1,
            self.expect(e1).and_then(|s| s.a),
            "query_disjoint_mut {e1:?}"
        );
        assert_eq!(
            got2,
            self.expect(e2).and_then(|s| s.a),
            "query_disjoint_mut {e2:?}"
        );
        if got1.is_some() {
            self.model.get_mut(&e1).unwrap().a = Some(v1);
        }
        if got2.is_some() {
            self.model.get_mut(&e2).unwrap().a = Some(v2);
        }
    }

    /// A `View` reaches by handle exactly the entities a query iterates, and
    /// writes through it are visible afterwards.
    #[rule]
    fn view_random_access(&mut self, tc: TestCase) {
        let e1 = self.draw_handle(&tc);
        let e2 = self.draw_handle(&tc);
        let (v1, v2) = (tc.draw(val()), tc.draw(val()));
        let (got1, got2);
        if e1 == e2 {
            let mut view = self.world.view_mut::<&mut B>();
            got1 = view.get_mut(e1).map(|b| {
                b.0 = v1;
            });
            got2 = None;
        } else {
            let mut view = self.world.view_mut::<&mut B>();
            let [r1, r2] = view.get_disjoint_mut([e1, e2]);
            got1 = r1.map(|b| b.0 = v1);
            got2 = r2.map(|b| b.0 = v2);
        }
        assert_eq!(
            got1.is_some(),
            self.expect(e1).is_some_and(|s| s.b.is_some()),
            "view B-presence {e1:?}"
        );
        if got1.is_some() {
            self.model.get_mut(&e1).unwrap().b = Some(v1);
        }
        if e1 != e2 {
            assert_eq!(
                got2.is_some(),
                self.expect(e2).is_some_and(|s| s.b.is_some()),
                "view B-presence {e2:?}"
            );
            if got2.is_some() {
                self.model.get_mut(&e2).unwrap().b = Some(v2);
            }
        }
    }

    /// `clear` empties the world. Entity values repeat afterwards, so pooled
    /// handles may alias freshly spawned entities; the model is keyed by the
    /// full handle, so that stays consistent.
    #[rule]
    fn clear(&mut self, _: TestCase) {
        self.world.clear();
        self.model.clear();
        self.reserved.clear();
        // `clear` documents that Entity values will repeat, so retired
        // handles may legitimately become live again.
        self.retired.clear();
        assert_eq!(self.world.len(), 0, "clear left live entities");
        assert_eq!(self.world.iter().count(), 0, "clear left iterable entities");
    }

    /// `spawn_batch` hands out one distinct handle per item, each carrying that
    /// item's components.
    #[rule]
    fn spawn_batch(&mut self, tc: TestCase) {
        self.flush_model();
        let n = tc.draw(gs::integers::<u32>().min_value(0).max_value(5));
        let v = tc.draw(val());
        let handles: Vec<Entity> = self
            .world
            .spawn_batch((0..n).map(|_| (A(v), B(v))))
            .collect();
        assert_eq!(
            handles.len(),
            n as usize,
            "spawn_batch yielded the wrong count"
        );
        let s = Spec {
            a: Some(v),
            b: Some(v),
            c: false,
            d: None,
        };
        for e in handles {
            assert!(
                self.model.insert(e, s).is_none(),
                "spawn_batch reused {e:?}"
            );
            self.handles.add(e);
            self.check_entity(e, "spawn_batch");
        }
    }

    /// `reserve` is a capacity hint with no observable effect.
    #[rule]
    fn reserve_capacity(&mut self, tc: TestCase) {
        self.world.flush();
        self.flush_model();
        let before = fingerprint(&self.world);
        let n = tc.draw(gs::integers::<u32>().min_value(0).max_value(8));
        self.world.reserve::<(A, B)>(n);
        assert_eq!(
            fingerprint(&self.world),
            before,
            "reserve changed observable state"
        );
    }

    /// Reserved handles are `contains`-true immediately but stay out of `len`,
    /// `iter` and every query until a flush.
    #[rule]
    fn reserve_entities(&mut self, tc: TestCase) {
        let len_before = self.world.len();
        let fresh: Vec<Entity> = if tc.draw(gs::booleans()) {
            vec![self.world.reserve_entity()]
        } else {
            let n = tc.draw(gs::integers::<u32>().min_value(0).max_value(4));
            self.world.reserve_entities(n).collect()
        };
        for &e in &fresh {
            assert!(self.world.contains(e), "reserved {e:?} not contained");
            self.reserved.push(e);
            self.handles.add(e);
        }
        assert_eq!(
            self.world.len(),
            len_before,
            "reserve_entity changed world.len()"
        );
    }

    /// An explicit `flush` turns reserved handles into componentless entities.
    #[rule]
    fn flush(&mut self, _: TestCase) {
        let expected = self.reserved.clone();
        self.world.flush();
        self.flush_model();
        for e in expected {
            assert_eq!(self.observe(e), Some(Spec::default()), "flushed {e:?}");
        }
    }

    /// `take` removes the entity; dropping the `TakenEntity` drops its
    /// components.
    #[rule]
    fn take_and_drop(&mut self, tc: TestCase) {
        self.flush_model();
        let e = self.draw_handle(&tc);
        let ok = self.world.take(e).is_ok();
        assert_eq!(
            ok,
            self.model.remove(&e).is_some(),
            "take disagreed for {e:?}"
        );
        if ok {
            self.retired.push(e);
        }
        self.check_entity(e, "take_and_drop");
    }

    /// Spawning a `TakenEntity` into another world moves its components across
    /// unchanged.
    #[rule]
    fn take_and_migrate(&mut self, tc: TestCase) {
        self.flush_model();
        let e = self.draw_handle(&tc);
        let expected = self.expect(e);
        match self.world.take(e) {
            Ok(taken) => {
                let mut scratch = World::new();
                let moved = scratch.spawn(taken);
                assert_eq!(
                    scratch.len(),
                    1,
                    "scratch world holds more than the moved entity"
                );
                let obs = Spec {
                    a: scratch.get::<&A>(moved).ok().map(|r| r.0),
                    b: scratch.get::<&B>(moved).ok().map(|r| r.0),
                    c: scratch.get::<&C>(moved).is_ok(),
                    d: scratch.get::<&D>(moved).ok().map(|r| r.0),
                };
                assert_eq!(Some(obs), expected, "migrated entity lost components");
                self.model.remove(&e);
                self.retired.push(e);
            }
            Err(_) => assert!(expected.is_none(), "take failed for live {e:?}"),
        }
    }

    /// A `BuiltEntityClone` can be spawned repeatedly, producing entities with
    /// identical components. `D` is not `Clone`, so this covers {A, B, C}.
    #[rule]
    fn spawn_clone_builder(&mut self, tc: TestCase) {
        self.flush_model();
        let s = tc.draw(specs_without_d());
        let mut builder = EntityBuilderClone::new();
        if let Some(v) = s.a {
            builder.add(A(v));
        }
        if let Some(v) = s.b {
            builder.add(B(v));
        }
        if s.c {
            builder.add(C);
        }
        let built = builder.build();
        for _ in 0..2 {
            let e = self.world.spawn(&built);
            assert!(
                self.model.insert(e, s).is_none(),
                "clone-builder spawn reused {e:?}"
            );
            self.handles.add(e);
            self.check_entity(e, "spawn_clone_builder");
        }
    }

    /// `find_entity_from_id` reconstructs the live handle for an id, so it must
    /// return the handle the id came from.
    #[rule]
    fn find_entity_from_id(&mut self, tc: TestCase) {
        self.world.flush();
        self.flush_model();
        let e = self.draw_handle(&tc);
        // SAFETY: the model tracks liveness, and `find_entity_from_id`
        // documents that the id must belong to a live entity.
        tc.assume(self.model.contains_key(&e));
        assert_eq!(
            unsafe { self.world.find_entity_from_id(e.id()) },
            e,
            "find_entity_from_id"
        );
    }

    // ---- global oracles ----

    /// The world and the model describe the same entities with the same
    /// components, in both directions.
    #[invariant]
    fn world_matches_model(&self, _: TestCase) {
        assert_eq!(
            self.world.len() as usize,
            self.model.len(),
            "world.len() != model.len()"
        );
        for (&e, s) in &self.model {
            assert!(self.world.contains(e), "world is missing modelled {e:?}");
            assert_eq!(self.observe(e), Some(*s), "components of {e:?}");
        }
        for eref in self.world.iter() {
            assert!(
                self.model.contains_key(&eref.entity()),
                "world has unmodelled {:?}",
                eref.entity()
            );
        }
    }

    #[invariant]
    fn archetypes_partition_entities(&self, _: TestCase) {
        check_archetypes(&self.world, "model world");
    }

    /// Exactly as many `D` values are alive as the model accounts for: a
    /// component leaked or dropped twice by an archetype migration shows up
    /// here.
    #[invariant]
    fn drops_balance(&self, _: TestCase) {
        let expected = self.model.values().filter(|s| s.d.is_some()).count() as i64;
        assert_eq!(d_live(), expected, "live D count != modelled D count");
    }

    /// A destroyed handle never resolves again: `despawn` reuses the id but
    /// advances the generation, which is what makes a stale handle safe to
    /// keep around.
    #[invariant]
    fn retired_handles_never_resolve(&self, _: TestCase) {
        for &e in &self.retired {
            assert!(!self.world.contains(e), "retired {e:?} became live again");
            assert!(
                self.world.entity(e).is_err(),
                "retired {e:?} resolved to an EntityRef"
            );
            assert!(
                matches!(
                    self.world.query_one::<&A>(e).get(),
                    Err(QueryOneError::NoSuchEntity)
                ),
                "query_one on retired {e:?} did not report NoSuchEntity"
            );
            assert!(
                !self.model.contains_key(&e),
                "retired {e:?} is still modelled"
            );
        }
    }

    /// No two live entities share an id.
    #[invariant]
    fn live_ids_are_unique(&self, _: TestCase) {
        let mut ids = std::collections::HashSet::new();
        for eref in self.world.iter() {
            let e = eref.entity();
            assert!(ids.insert(e.id()), "id {} is live twice", e.id());
        }
    }

    /// Reserved handles exist but are invisible to iteration and queries.
    #[invariant]
    fn reserved_handles_are_not_live(&self, _: TestCase) {
        for &e in &self.reserved {
            assert!(self.world.contains(e), "reserved {e:?} not contained");
            assert!(
                !self.model.contains_key(&e),
                "reserved {e:?} leaked into the model"
            );
            assert!(
                self.world.iter().all(|eref| eref.entity() != e),
                "reserved {e:?} appeared in iter()"
            );
        }
    }

    /// Every query shape yields exactly the entities and values the model
    /// predicts, with no duplicates.
    #[invariant]
    fn query_shapes_match_model(&self, _: TestCase) {
        let model = &self.model;
        let world = &self.world;

        let mut got = HashMap::new();
        for (e, a) in world.query::<(Entity, &A)>().iter() {
            assert!(
                got.insert(e, a.0).is_none(),
                "query::<&A> yielded {e:?} twice"
            );
        }
        let want: HashMap<Entity, i32> = model
            .iter()
            .filter_map(|(&e, s)| s.a.map(|v| (e, v)))
            .collect();
        assert_eq!(got, want, "query::<&A>");

        let mut got = HashMap::new();
        for (e, a, b) in world.query::<(Entity, &A, &B)>().iter() {
            assert!(
                got.insert(e, (a.0, b.0)).is_none(),
                "query::<(&A,&B)> yielded {e:?} twice"
            );
        }
        let want: HashMap<Entity, (i32, i32)> = model
            .iter()
            .filter_map(|(&e, s)| s.a.zip(s.b).map(|v| (e, v)))
            .collect();
        assert_eq!(got, want, "query::<(&A,&B)>");

        let with: HashMap<Entity, i32> = world
            .query::<With<(Entity, &A), &B>>()
            .iter()
            .map(|(e, a)| (e, a.0))
            .collect();
        let want: HashMap<Entity, i32> = model
            .iter()
            .filter_map(|(&e, s)| s.b.and(s.a).map(|v| (e, v)))
            .collect();
        assert_eq!(with, want, "query::<With<&A, &B>>");

        let without: HashMap<Entity, i32> = world
            .query::<Without<(Entity, &A), &B>>()
            .iter()
            .map(|(e, a)| (e, a.0))
            .collect();
        let want: HashMap<Entity, i32> = model
            .iter()
            .filter_map(|(&e, s)| match (s.a, s.b) {
                (Some(v), None) => Some((e, v)),
                _ => None,
            })
            .collect();
        assert_eq!(without, want, "query::<Without<&A, &B>>");

        let or: HashMap<Entity, (Option<i32>, Option<i32>)> = world
            .query::<(Entity, Or<&A, &B>)>()
            .iter()
            .map(|(e, ab)| (e, (ab.left().map(|a| a.0), ab.right().map(|b| b.0))))
            .collect();
        let want: HashMap<Entity, (Option<i32>, Option<i32>)> = model
            .iter()
            .filter(|(_, s)| s.a.is_some() || s.b.is_some())
            .map(|(&e, s)| (e, (s.a, s.b)))
            .collect();
        assert_eq!(or, want, "query::<Or<&A, &B>>");

        let opt: HashMap<Entity, Option<i32>> = world
            .query::<(Entity, &A, Option<&B>)>()
            .iter()
            .map(|(e, _, b)| (e, b.map(|b| b.0)))
            .collect();
        let want: HashMap<Entity, Option<i32>> = model
            .iter()
            .filter_map(|(&e, s)| s.a.map(|_| (e, s.b)))
            .collect();
        assert_eq!(opt, want, "query::<(&A, Option<&B>)>");
    }

    /// Per-entity access agrees with the model for every live entity,
    /// including the `Unsatisfied` and `NoSuchEntity` distinction.
    #[invariant]
    fn per_entity_access_matches_model(&self, _: TestCase) {
        for (&e, s) in &self.model {
            match self.world.query_one::<&A>(e).get() {
                Ok(a) => assert_eq!(Some(a.0), s.a, "query_one::<&A> value for {e:?}"),
                Err(QueryOneError::Unsatisfied) => {
                    assert!(
                        s.a.is_none(),
                        "query_one::<&A> unsatisfied but A modelled for {e:?}"
                    )
                }
                Err(QueryOneError::NoSuchEntity) => panic!("query_one on live {e:?}"),
            }
            // `with`/`without` filter the same query by another component's
            // presence without borrowing it.
            match self.world.query_one::<&A>(e).with::<&B>().get() {
                Ok(a) => assert_eq!((Some(a.0), true), (s.a, s.b.is_some()), "with::<&B> {e:?}"),
                Err(QueryOneError::Unsatisfied) => {
                    assert!(
                        s.a.is_none() || s.b.is_none(),
                        "with::<&B> unsatisfied for {e:?}"
                    )
                }
                Err(QueryOneError::NoSuchEntity) => panic!("with::<&B> on live {e:?}"),
            }
            match self.world.query_one::<&A>(e).without::<&B>().get() {
                Ok(a) => assert_eq!(
                    (Some(a.0), false),
                    (s.a, s.b.is_some()),
                    "without::<&B> {e:?}"
                ),
                Err(QueryOneError::Unsatisfied) => {
                    assert!(
                        s.a.is_none() || s.b.is_some(),
                        "without::<&B> unsatisfied for {e:?}"
                    )
                }
                Err(QueryOneError::NoSuchEntity) => panic!("without::<&B> on live {e:?}"),
            }
            assert_eq!(
                self.world.satisfies::<&A>(e),
                s.a.is_some(),
                "satisfies::<&A> {e:?}"
            );
            assert_eq!(
                self.world.satisfies::<(&A, &B)>(e),
                s.a.is_some() && s.b.is_some(),
                "satisfies::<(&A,&B)> {e:?}"
            );

            let eref = self.world.entity(e).expect("live modelled entity");
            assert_eq!(eref.entity(), e, "EntityRef::entity");
            assert_eq!(eref.has::<A>(), s.a.is_some(), "EntityRef::has::<A> {e:?}");
            assert_eq!(eref.has::<C>(), s.c, "EntityRef::has::<C> {e:?}");
            assert_eq!(eref.len(), s.component_count(), "EntityRef::len {e:?}");
            assert_eq!(
                eref.is_empty(),
                s.component_count() == 0,
                "EntityRef::is_empty {e:?}"
            );
            assert_eq!(
                eref.component_types().count(),
                s.component_count(),
                "EntityRef::component_types {e:?}"
            );
        }

        let view = self.world.view::<&A>();
        for (&e, s) in &self.model {
            assert_eq!(view.get(e).map(|a| a.0), s.a, "view.get {e:?}");
            assert_eq!(view.contains(e), s.a.is_some(), "view.contains {e:?}");
        }
    }

    /// A `PreparedQuery` caches archetype state across calls, so it must keep
    /// agreeing with a freshly built query as archetypes come and go.
    #[invariant]
    fn prepared_query_matches_fresh_query(&mut self, _: TestCase) {
        let fresh: HashMap<Entity, i32> = self
            .world
            .query::<(Entity, &A)>()
            .iter()
            .map(|(e, a)| (e, a.0))
            .collect();
        let mut pq = PreparedQuery::<(Entity, &A)>::new();
        let prepared: HashMap<Entity, i32> = {
            let mut borrow = pq.query(&self.world);
            borrow.iter().map(|(e, a)| (e, a.0)).collect()
        };
        assert_eq!(
            prepared, fresh,
            "PreparedQuery disagreed with a fresh query"
        );

        let mut pv = PreparedQuery::<&A>::new();
        let view = pv.view_mut(&mut self.world);
        for (&e, v) in &fresh {
            assert_eq!(
                view.get(e).map(|a| a.0),
                Some(*v),
                "PreparedView::get {e:?}"
            );
        }
    }

    /// Batched iteration visits exactly the same entities as flat iteration.
    #[invariant]
    fn batched_iteration_matches_flat(&self, tc: TestCase) {
        let flat: HashMap<Entity, i32> = self
            .world
            .query::<(Entity, &A)>()
            .iter()
            .map(|(e, a)| (e, a.0))
            .collect();
        // A zero batch size is documented to panic.
        let size = tc.draw(gs::integers::<u32>().min_value(1).max_value(4));
        let mut q = self.world.query::<(Entity, &A)>();
        let mut got = HashMap::new();
        for batch in q.iter_batched(size) {
            for (e, a) in batch {
                assert!(
                    got.insert(e, a.0).is_none(),
                    "iter_batched yielded {e:?} twice"
                );
            }
        }
        assert_eq!(
            got, flat,
            "iter_batched(={size}) disagreed with flat iteration"
        );
    }
}

#[hegel::test(settings().stateful_step_count(STEPS))]
fn world_matches_model(tc: TestCase) {
    assert_d_balanced_at_start();
    let machine = WorldModel {
        world: World::new(),
        model: HashMap::new(),
        handles: pool(&tc),
        reserved: Vec::new(),
        retired: Vec::new(),
    };
    hegel::stateful::run(machine, tc);
}
