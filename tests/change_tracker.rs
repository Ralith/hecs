//! Model-based property test for `ChangeTracker`.
//!
//! `ChangeTracker<T>` keeps a private `Previous<T>` component holding the value
//! as of the last `track` call, and `Changes` reports, relative to that:
//! `added` for entities that have `T` but no snapshot, `changed` for entities
//! whose current value differs from the snapshot *by `PartialEq`*, and
//! `removed` for live entities with a snapshot but no `T`. Iterators the caller
//! does not drain are drained when `Changes` drops, so the snapshot always
//! advances to the current state.
//!
//! The model carries the current payload and the snapshot payload per entity,
//! which pins the consequences exactly: removing and re-inserting a different
//! value between polls is a change, not a removal followed by an addition;
//! re-inserting an equal value is nothing; writing an equal value through
//! `&mut` is nothing; and an entity that is despawned is never reported as
//! removed.
//!
//! The tracked component counts its own live instances, so between polls the
//! only live values must be the world's components plus the tracker's
//! snapshots. That catches a snapshot leaked or dropped twice inside the
//! tracker.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use fixtures::{settings, val};
use hecs::{ChangeTracker, Changes, Entity, EntityBuilder, World};
use hegel::generators::{self as gs, Generator};
use hegel::stateful::{pool, Pool};
use hegel::TestCase;

const STEPS: i64 = 150;

/// The tracked component. Every `Tracked` holds a clone of one `Arc`, so the
/// strong count minus the test's own handle is how many are alive. `Clone`
/// counts too, because that is how the tracker takes its snapshots.
#[derive(Clone)]
struct Tracked {
    value: i32,
    _live: Arc<()>,
}

impl fmt::Debug for Tracked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Tracked").field(&self.value).finish()
    }
}

impl PartialEq for Tracked {
    fn eq(&self, other: &Tracked) -> bool {
        self.value == other.value
    }
}

/// An untracked component, used to force archetype migrations under the
/// tracker: the private snapshot has to migrate with the entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Other(i32);

#[derive(Clone, Copy, Debug, Default)]
struct Model {
    /// The entity's current `Tracked` payload.
    current: Option<i32>,
    /// The payload the tracker snapshotted at the last poll.
    snapshot: Option<i32>,
    other: Option<i32>,
}

struct TrackerModel {
    world: World,
    tracker: ChangeTracker<Tracked>,
    model: HashMap<Entity, Model>,
    handles: Pool<Entity>,
    /// The handle every `Tracked` is cloned from.
    live: Arc<()>,
}

/// What the next poll must report, derived from the model alone.
struct Expected {
    added: HashMap<Entity, i32>,
    changed: HashMap<Entity, (i32, i32)>,
    removed: HashMap<Entity, i32>,
}

/// One of the three reports `Changes` offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, hegel::PrettyPrintable)]
enum Report {
    Added,
    Changed,
    Removed,
}

const REPORTS: [Report; 3] = [Report::Added, Report::Changed, Report::Removed];

impl TrackerModel {
    fn expected(&self) -> Expected {
        let mut expected = Expected {
            added: HashMap::new(),
            changed: HashMap::new(),
            removed: HashMap::new(),
        };
        for (&e, m) in &self.model {
            match (m.current, m.snapshot) {
                (Some(v), None) => {
                    expected.added.insert(e, v);
                }
                (Some(new), Some(old)) if new != old => {
                    expected.changed.insert(e, (old, new));
                }
                (None, Some(old)) => {
                    expected.removed.insert(e, old);
                }
                _ => {}
            }
        }
        expected
    }

    fn draw_handle(&self, tc: &TestCase) -> Entity {
        *tc.draw(self.handles.values_reusable().print_as_debug())
    }

    fn tracked(&self, value: i32) -> Tracked {
        Tracked {
            value,
            _live: self.live.clone(),
        }
    }

    /// How many `Tracked` values are alive, snapshots included.
    fn live(&self) -> usize {
        Arc::strong_count(&self.live) - 1
    }
}

fn check_added(changes: &mut Changes<'_, Tracked>, want: &HashMap<Entity, i32>) {
    let it = changes.added();
    assert_eq!(it.len(), want.len(), "added().len()");
    let mut got = HashMap::new();
    for (e, t) in it {
        assert!(
            got.insert(e, t.value).is_none(),
            "added() yielded {e:?} twice"
        );
    }
    assert_eq!(got, *want, "added()");
}

/// Pull one element of `added()` and leave the rest to `Changes::drop`.
fn peek_added(changes: &mut Changes<'_, Tracked>, want: &HashMap<Entity, i32>) {
    let mut it = changes.added();
    assert_eq!(it.len(), want.len(), "added().len()");
    if let Some((e, t)) = it.next() {
        assert_eq!(want.get(&e), Some(&t.value), "first element of added()");
    }
}

fn check_changed(changes: &mut Changes<'_, Tracked>, want: &HashMap<Entity, (i32, i32)>) {
    let mut got = HashMap::new();
    for (e, old, new) in changes.changed() {
        assert!(
            got.insert(e, (old.value, new.value)).is_none(),
            "changed() yielded {e:?} twice"
        );
    }
    assert_eq!(got, *want, "changed()");
}

fn check_removed(changes: &mut Changes<'_, Tracked>, want: &HashMap<Entity, i32>) {
    let it = changes.removed();
    assert_eq!(it.len(), want.len(), "removed().len()");
    let mut got = HashMap::new();
    for (e, old) in it {
        assert!(
            got.insert(e, old.value).is_none(),
            "removed() yielded {e:?} twice"
        );
    }
    assert_eq!(got, *want, "removed()");
}

// Driven by `change_tracker_matches_model` below.
#[hegel::state_machine]
impl TrackerModel {
    #[rule]
    fn spawn(&mut self, tc: TestCase) {
        let tracked = tc.draw(gs::optional(val()));
        let other = tc.draw(gs::optional(val()));
        let mut builder = EntityBuilder::new();
        if let Some(v) = tracked {
            builder.add(self.tracked(v));
        }
        if let Some(v) = other {
            builder.add(Other(v));
        }
        let e = self.world.spawn(builder.build());
        self.model.insert(
            e,
            Model {
                current: tracked,
                snapshot: None,
                other,
            },
        );
        self.handles.add(e);
    }

    /// A homogeneous batch, so `spawn_batch` also feeds the tracker.
    #[rule]
    fn spawn_batch(&mut self, tc: TestCase) {
        let n = tc.draw(gs::integers::<u32>().min_value(0).max_value(4));
        let v = tc.draw(val());
        let live = &self.live;
        let tracked = || Tracked {
            value: v,
            _live: live.clone(),
        };
        let handles: Vec<Entity> = self
            .world
            .spawn_batch((0..n).map(|_| (tracked(), Other(v))))
            .collect();
        for e in handles {
            self.model.insert(
                e,
                Model {
                    current: Some(v),
                    snapshot: None,
                    other: Some(v),
                },
            );
            self.handles.add(e);
        }
    }

    /// Despawning drops both the component and its snapshot, and the entity
    /// must never be reported as removed.
    #[rule]
    fn despawn(&mut self, tc: TestCase) {
        let e = self.draw_handle(&tc);
        let ok = self.world.despawn(e).is_ok();
        assert_eq!(
            ok,
            self.model.remove(&e).is_some(),
            "despawn disagreed for {e:?}"
        );
    }

    #[rule]
    fn insert_tracked(&mut self, tc: TestCase) {
        let e = self.draw_handle(&tc);
        let v = tc.draw(val());
        let live_entity = self.model.contains_key(&e);
        let ok = self.world.insert_one(e, self.tracked(v)).is_ok();
        assert_eq!(ok, live_entity, "insert of Tracked disagreed for {e:?}");
        if let Some(m) = self.model.get_mut(&e) {
            m.current = Some(v);
        }
    }

    #[rule]
    fn remove_tracked(&mut self, tc: TestCase) {
        let e = self.draw_handle(&tc);
        let had = self.model.get(&e).is_some_and(|m| m.current.is_some());
        assert_eq!(
            self.world.remove_one::<Tracked>(e).is_ok(),
            had,
            "remove of Tracked {e:?}"
        );
        if let Some(m) = self.model.get_mut(&e) {
            m.current = None;
        }
    }

    #[rule]
    fn write_through_unique_reference(&mut self, tc: TestCase) {
        let e = self.draw_handle(&tc);
        let v = tc.draw(val());
        let had = self.model.get(&e).is_some_and(|m| m.current.is_some());
        match self.world.get::<&mut Tracked>(e) {
            Ok(mut t) => {
                assert!(
                    had,
                    "&mut Tracked succeeded for {e:?} but the model has none"
                );
                t.value = v;
            }
            Err(_) => assert!(!had, "&mut Tracked failed for {e:?} but the model has one"),
        }
        if had {
            self.model.get_mut(&e).unwrap().current = Some(v);
        }
    }

    /// Writing the value it already holds must not count as a change: the
    /// tracker compares by `PartialEq`, not by whether `&mut` was taken.
    #[rule]
    fn write_the_same_value(&mut self, tc: TestCase) {
        let e = self.draw_handle(&tc);
        if let Some(v) = self.model.get(&e).and_then(|m| m.current) {
            self.world
                .get::<&mut Tracked>(e)
                .expect("the model says Tracked is present")
                .value = v;
        }
    }

    #[rule]
    fn insert_other(&mut self, tc: TestCase) {
        let e = self.draw_handle(&tc);
        let v = tc.draw(val());
        let live_entity = self.model.contains_key(&e);
        assert_eq!(
            self.world.insert_one(e, Other(v)).is_ok(),
            live_entity,
            "insert Other {e:?}"
        );
        if let Some(m) = self.model.get_mut(&e) {
            m.other = Some(v);
        }
    }

    #[rule]
    fn remove_other(&mut self, tc: TestCase) {
        let e = self.draw_handle(&tc);
        let had = self.model.get(&e).is_some_and(|m| m.other.is_some());
        assert_eq!(
            self.world.remove_one::<Other>(e).is_ok(),
            had,
            "remove Other {e:?}"
        );
        if let Some(m) = self.model.get_mut(&e) {
            m.other = None;
        }
    }

    /// `clear` drops the snapshots along with everything else, so whatever is
    /// spawned next is reported as added.
    #[rule]
    fn clear(&mut self, _: TestCase) {
        self.world.clear();
        self.model.clear();
    }

    /// Poll the tracker, check the reports the caller pulls against the
    /// model, and advance the model's snapshots.
    ///
    /// `pulled` is which reports the caller asks for and in what order; the
    /// rest are left to `Changes::drop`, which must drain them so that the
    /// snapshot advances regardless. The next poll and the live-count
    /// invariant check that it did.
    #[rule]
    fn poll(&mut self, tc: TestCase) {
        let expected = self.expected();
        let pulled = tc.draw(gs::samples(&REPORTS[..]).without_replacement());
        let peek_added_only = tc.draw(gs::booleans());
        tc.note(&format!(
            "poll: pulled {pulled:?}, peek_added_only = {peek_added_only}"
        ));
        let mut changes = self.tracker.track(&mut self.world);
        for report in pulled {
            match report {
                Report::Added if peek_added_only => peek_added(&mut changes, &expected.added),
                Report::Added => check_added(&mut changes, &expected.added),
                Report::Changed => check_changed(&mut changes, &expected.changed),
                Report::Removed => check_removed(&mut changes, &expected.removed),
            }
        }
        drop(changes);
        for m in self.model.values_mut() {
            m.snapshot = m.current;
        }
    }

    #[invariant]
    fn world_matches_model(&self, _: TestCase) {
        assert_eq!(
            self.world.len() as usize,
            self.model.len(),
            "world.len() != model.len()"
        );
        for (&e, m) in &self.model {
            assert!(self.world.contains(e), "world is missing modelled {e:?}");
            assert_eq!(
                self.world.get::<&Tracked>(e).ok().map(|t| t.value),
                m.current,
                "Tracked on {e:?}"
            );
            assert_eq!(
                self.world.get::<&Other>(e).ok().map(|o| o.0),
                m.other,
                "Other on {e:?}"
            );
        }
        for eref in self.world.iter() {
            assert!(
                self.model.contains_key(&eref.entity()),
                "world has unmodelled {:?}",
                eref.entity()
            );
        }
    }

    /// Live tracked values are exactly the world's components plus the
    /// tracker's snapshots.
    #[invariant]
    fn tracked_values_are_accounted_for(&self, _: TestCase) {
        let current = self.model.values().filter(|m| m.current.is_some()).count();
        let snapshots = self.model.values().filter(|m| m.snapshot.is_some()).count();
        assert_eq!(
            self.live(),
            current + snapshots,
            "live Tracked count != current components + snapshots"
        );
    }
}

// Not run under Miri: `ChangeTracker` contains no unsafe code — it is built
// on `PreparedQuery`, `insert_one` and `remove_one` — so the interpreter has
// nothing here that the `World` and query properties do not already cover, and
// polling the tracker is by far the slowest thing in the suite under it.
#[cfg_attr(
    miri,
    ignore = "no unsafe code to check, and slow under the interpreter"
)]
#[hegel::test(settings().stateful_step_count(STEPS))]
fn change_tracker_matches_model(tc: TestCase) {
    let machine = TrackerModel {
        world: World::new(),
        tracker: ChangeTracker::new(),
        model: HashMap::new(),
        handles: pool(&tc),
        live: Arc::new(()),
    };
    hegel::stateful::run(machine, tc);
}
