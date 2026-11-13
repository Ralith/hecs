//! Shared fixtures for the property tests: four component types, an
//! observational fingerprint of a `World`, and twin worlds replayed from one
//! generated history.
//!
//! The hegel idioms the tests lean on:
//!
//! - `#[hegel::test(settings())]` runs the body once per generated case (250,
//!   or 4 under Miri); each `tc.draw(g)` pulls one value from the generator
//!   `g`, and a failing case is shrunk and replayed.
//! - `#[hegel::composite]` turns `fn f(tc: &TestCase, args..) -> T` into
//!   `fn f(args..) -> impl Generator<T>`: call sites drop the `tc` argument
//!   and get back a generator that runs the body per draw.
//! - `tc.assume(p)` discards the case when `p` is false (inside a
//!   state-machine rule it rejects just that step).
//! - On failure hegel prints every drawn value. `PrettyPrintable` and
//!   `print_as_debug` feed that report and nothing else: `print_as_debug`
//!   makes a generator of a type without `PrettyPrintable` (such as
//!   `Entity`) printable via `Debug`.
//! - A `stateful::Pool` holds values earlier draws produced (here: entity
//!   handles) so later draws can pick among them. Drawing from an empty pool
//!   rejects the case or step like a failed `assume`.
//! - `hegel::stateful::run` drives a `#[hegel::state_machine]`: each step
//!   applies one randomly chosen `#[rule]`, `stateful_step_count` times, and
//!   every `#[invariant]` is checked on the initial and final state and at
//!   sampled points in between. A rejected rule does not use up a step.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::sync::Arc;

use hecs::{
    CommandBuffer, ComponentError, DynamicBundle, DynamicBundleClone, Entity, EntityBuilder,
    EntityBuilderClone, EntityRef, NoSuchEntity, World,
};
use hegel::generators::{self as gs, Generator};
use hegel::stateful::{pool, Pool};
use hegel::TestCase;
use serde::de::DeserializeSeed;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The component types the tests use: two same-shaped payload components, a
/// zero-sized marker, and a drop-tracked non-`Copy` component.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct A(pub i32);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct B(pub i32);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct C;

/// Counts live `D` values: each holds a clone of the `Arc`, so the strong
/// count is one more than the number alive. A leak or a double drop in hecs's
/// unsafe component moves shows up as a count that disagrees with the worlds'
/// contents.
#[derive(Default)]
pub struct DropTracker(Arc<()>);

impl DropTracker {
    pub fn new() -> DropTracker {
        DropTracker::default()
    }

    /// How many `D` values made from this tracker are alive.
    pub fn live(&self) -> usize {
        Arc::strong_count(&self.0) - 1
    }
}

/// A non-`Copy` component counted by the `DropTracker` it was made from.
pub struct D {
    pub value: i32,
    _live: Arc<()>,
}

impl D {
    pub fn new(value: i32, ds: &DropTracker) -> D {
        D {
            value,
            _live: ds.0.clone(),
        }
    }
}

impl fmt::Debug for D {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("D").field(&self.value).finish()
    }
}

/// The wire form is the payload alone, as `#[derive(Serialize)]` on a newtype
/// `D(i32)` would give.
impl Serialize for D {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_newtype_struct("D", &self.value)
    }
}

/// Deserializes a `D` counted by the given tracker. `D` has no `Deserialize`
/// impl, because a bare `D` would belong to no tracker.
pub struct DSeed<'a>(pub &'a DropTracker);

impl<'de> DeserializeSeed<'de> for DSeed<'_> {
    type Value = D;

    fn deserialize<De: Deserializer<'de>>(self, de: De) -> Result<D, De::Error> {
        i32::deserialize(de).map(|v| D::new(v, self.0))
    }
}

/// Under Miri each operation is interpreted, so the tests run four fixed
/// cases each and serve as a UB oracle for hecs's unsafe component moves
/// rather than as a search for logic bugs. Miri's isolation denies the file
/// and random-device access behind hegel's fresh seeds and failure database
/// (the `.hegel` directory, where failing cases persist to be replayed), and
/// its slow-test health check would report on the interpreter, so those are
/// turned off. Outside Miri each property runs 250 cases.
pub fn settings() -> hegel::Settings {
    let settings = hegel::Settings::new();
    if cfg!(miri) {
        settings
            .test_cases(4)
            .derandomize(true)
            .database(None)
            .suppress_health_check([hegel::HealthCheck::TooSlow])
    } else {
        settings.test_cases(250)
    }
}

/// Worlds are kept small so that handle collisions, empty archetypes and
/// stale handles all occur often; the bound protects the tests' runtime, not
/// any hecs contract.
pub const MAX_ENTITIES: u32 = if cfg!(miri) { 4 } else { 8 };

/// Rule applications per state-machine run, sized like the histories so the
/// worlds stay small.
pub const STEPS: i64 = MAX_ENTITIES as i64 * 2;

/// Any `i32`, so that two entities rarely hold the same payload and a swapped
/// or stale component shows up as a wrong value.
pub fn val() -> impl gs::PrintableGenerator<i32> {
    gs::integers::<i32>()
}

/// One of the pool's handles, drawn uniformly. Callers keep every handle
/// their world has issued in the pool, despawned ones included, so a draw
/// often names a dead entity and the `NoSuchEntity` paths get exercised too.
pub fn handle_from(handles: &Pool<Entity>) -> impl gs::PrintableGenerator<Entity> + '_ {
    handles.values_reusable().map(|&e| e).print_as_debug()
}

/// One of the four component types, for operations that name a single
/// component.
#[derive(Clone, Copy, Debug, PartialEq, Eq, hegel::PrettyPrintable)]
pub enum Kind {
    A,
    B,
    C,
    D,
}

pub const KINDS: [Kind; 4] = [Kind::A, Kind::B, Kind::C, Kind::D];

pub fn kinds() -> impl gs::PrintableGenerator<Kind> {
    gs::sampled_from(&KINDS[..])
}

impl Kind {
    /// `insert_one` of this kind's component, with payload `v` where it has
    /// one.
    pub fn insert_one(
        self,
        world: &mut World,
        e: Entity,
        v: i32,
        ds: &DropTracker,
    ) -> Result<(), NoSuchEntity> {
        match self {
            Kind::A => world.insert_one(e, A(v)),
            Kind::B => world.insert_one(e, B(v)),
            Kind::C => world.insert_one(e, C),
            Kind::D => world.insert_one(e, D::new(v, ds)),
        }
    }

    /// `remove_one` of this kind's component. The removed value is dropped.
    pub fn remove_one(self, world: &mut World, e: Entity) -> Result<(), ComponentError> {
        match self {
            Kind::A => world.remove_one::<A>(e).map(drop),
            Kind::B => world.remove_one::<B>(e).map(drop),
            Kind::C => world.remove_one::<C>(e).map(drop),
            Kind::D => world.remove_one::<D>(e).map(drop),
        }
    }

    /// `CommandBuffer::insert_one` of this kind's component.
    pub fn buffer_insert_one(
        self,
        buffer: &mut CommandBuffer,
        e: Entity,
        v: i32,
        ds: &DropTracker,
    ) {
        match self {
            Kind::A => buffer.insert_one(e, A(v)),
            Kind::B => buffer.insert_one(e, B(v)),
            Kind::C => buffer.insert_one(e, C),
            Kind::D => buffer.insert_one(e, D::new(v, ds)),
        }
    }

    /// `CommandBuffer::remove_one` of this kind's component.
    pub fn buffer_remove_one(self, buffer: &mut CommandBuffer, e: Entity) {
        match self {
            Kind::A => buffer.remove_one::<A>(e),
            Kind::B => buffer.remove_one::<B>(e),
            Kind::C => buffer.remove_one::<C>(e),
            Kind::D => buffer.remove_one::<D>(e),
        }
    }
}

/// The components of one entity: the payloads of its `A`, `B` and `D`, and
/// whether it has a `C`. `D` payloads stay as `i32` here and become `D`
/// values in `builder`, so a tracker only ever counts values that were handed
/// to hecs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, hegel::PrettyPrintable)]
pub struct Components {
    pub a: Option<i32>,
    pub b: Option<i32>,
    pub c: bool,
    pub d: Option<i32>,
}

impl Components {
    pub fn component_count(&self) -> usize {
        self.a.is_some() as usize
            + self.b.is_some() as usize
            + self.c as usize
            + self.d.is_some() as usize
    }

    pub fn has(&self, kind: Kind) -> bool {
        match kind {
            Kind::A => self.a.is_some(),
            Kind::B => self.b.is_some(),
            Kind::C => self.c,
            Kind::D => self.d.is_some(),
        }
    }

    /// `self` with the `kind` component present, holding `v` where it has a
    /// payload.
    pub fn with(mut self, kind: Kind, v: i32) -> Components {
        match kind {
            Kind::A => self.a = Some(v),
            Kind::B => self.b = Some(v),
            Kind::C => self.c = true,
            Kind::D => self.d = Some(v),
        }
        self
    }

    /// `self` with the `kind` component absent.
    pub fn without(mut self, kind: Kind) -> Components {
        match kind {
            Kind::A => self.a = None,
            Kind::B => self.b = None,
            Kind::C => self.c = false,
            Kind::D => self.d = None,
        }
        self
    }

    /// What `eref` holds right now.
    pub fn observed(eref: EntityRef<'_>) -> Components {
        Components {
            a: eref.get::<&A>().map(|r| r.0),
            b: eref.get::<&B>().map(|r| r.0),
            c: eref.get::<&C>().is_some(),
            d: eref.get::<&D>().map(|r| r.value),
        }
    }

    pub fn builder(&self, ds: &DropTracker) -> EntityBuilder {
        let mut b = EntityBuilder::new();
        if let Some(v) = self.a {
            b.add(A(v));
        }
        if let Some(v) = self.b {
            b.add(B(v));
        }
        if self.c {
            b.add(C);
        }
        if let Some(v) = self.d {
            b.add(D::new(v, ds));
        }
        b
    }

    /// `builder` for `EntityBuilderClone`, which cannot hold the non-`Clone`
    /// `D`.
    pub fn clone_builder(&self) -> EntityBuilderClone {
        assert!(self.d.is_none(), "D is not Clone");
        let mut b = EntityBuilderClone::new();
        if let Some(v) = self.a {
            b.add(A(v));
        }
        if let Some(v) = self.b {
            b.add(B(v));
        }
        if self.c {
            b.add(C);
        }
        b
    }
}

/// A consumer of the one concrete tuple `put_as_tuple` picks. Tuples holding
/// the non-`Clone` `D` arrive through `put_with_d`, so a sink that needs
/// `DynamicBundleClone` still gets it on the `D`-free arms.
pub trait TupleSink {
    fn put(&mut self, bundle: impl DynamicBundleClone);
    fn put_with_d(&mut self, bundle: impl DynamicBundle);
}

/// Hand `sink` the components of `cs` as one concrete tuple, one static type
/// per subset of `{A, B, C, D}`. This is the single listing of the tuple
/// types, so the tests driving them cannot drift apart.
pub fn put_as_tuple(sink: &mut impl TupleSink, cs: Components, ds: &DropTracker) {
    match (cs.a, cs.b, cs.c, cs.d) {
        (None, None, false, None) => sink.put(()),
        (Some(a), None, false, None) => sink.put((A(a),)),
        (None, Some(b), false, None) => sink.put((B(b),)),
        (None, None, true, None) => sink.put((C,)),
        (None, None, false, Some(d)) => sink.put_with_d((D::new(d, ds),)),
        (Some(a), Some(b), false, None) => sink.put((A(a), B(b))),
        (Some(a), None, true, None) => sink.put((A(a), C)),
        (Some(a), None, false, Some(d)) => sink.put_with_d((A(a), D::new(d, ds))),
        (None, Some(b), true, None) => sink.put((B(b), C)),
        (None, Some(b), false, Some(d)) => sink.put_with_d((B(b), D::new(d, ds))),
        (None, None, true, Some(d)) => sink.put_with_d((C, D::new(d, ds))),
        (Some(a), Some(b), true, None) => sink.put((A(a), B(b), C)),
        (Some(a), Some(b), false, Some(d)) => sink.put_with_d((A(a), B(b), D::new(d, ds))),
        (Some(a), None, true, Some(d)) => sink.put_with_d((A(a), C, D::new(d, ds))),
        (None, Some(b), true, Some(d)) => sink.put_with_d((B(b), C, D::new(d, ds))),
        (Some(a), Some(b), true, Some(d)) => sink.put_with_d((A(a), B(b), C, D::new(d, ds))),
    }
}

/// Each component present or absent independently, with a drawn payload.
#[hegel::composite]
pub fn components(tc: &hegel::TestCase) -> Components {
    Components {
        a: tc.draw(gs::optional(val())),
        b: tc.draw(gs::optional(val())),
        c: tc.draw(gs::booleans()),
        d: tc.draw(gs::optional(val())),
    }
}

/// `components()` without `D`, which is not `Clone` and so cannot go into an
/// `EntityBuilderClone`.
pub fn components_without_d() -> impl gs::PrintableGenerator<Components> {
    components().map(|cs| Components { d: None, ..cs })
}

/// A single-entity operation, for tests that apply one operation to several
/// worlds. Spawning is excluded: the handle a spawn returns depends on
/// allocation order, so spawns do not commute.
#[derive(Clone, Copy, Debug, hegel::PrettyPrintable)]
pub enum Op {
    InsertOne(Kind, i32),
    RemoveOne(Kind),
    InsertBundle(Components),
    RemoveAB,
    RemoveCD,
    Despawn,
    Take,
    MutateA(i32),
    ExchangeAToB(i32),
    ExchangeDToA(i32),
}

pub fn ops() -> impl gs::PrintableGenerator<Op> {
    hegel::one_of!(
        hegel::compose!(|tc| { Op::InsertOne(tc.draw(kinds()), tc.draw(val())) }),
        hegel::compose!(|tc| { Op::RemoveOne(tc.draw(kinds())) }),
        hegel::compose!(|tc| { Op::InsertBundle(tc.draw(components())) }),
        gs::just(Op::RemoveAB),
        gs::just(Op::RemoveCD),
        gs::just(Op::Despawn),
        gs::just(Op::Take),
        hegel::compose!(|tc| { Op::MutateA(tc.draw(val())) }),
        hegel::compose!(|tc| { Op::ExchangeAToB(tc.draw(val())) }),
        hegel::compose!(|tc| { Op::ExchangeDToA(tc.draw(val())) }),
    )
}

/// Apply `op` to `e` and report whether it succeeded.
pub fn apply(world: &mut World, e: Entity, op: Op, ds: &DropTracker) -> bool {
    match op {
        Op::InsertOne(kind, v) => kind.insert_one(world, e, v, ds).is_ok(),
        Op::RemoveOne(kind) => kind.remove_one(world, e).is_ok(),
        Op::InsertBundle(cs) => world.insert(e, cs.builder(ds).build()).is_ok(),
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

/// Canonical snapshot of everything a caller can observe about a `World`: the
/// exact `Entity` handles (id and generation) and, per entity, the exact
/// component set and values. Two worlds are observationally equivalent iff
/// their fingerprints compare equal.
pub type Fingerprint = BTreeMap<Entity, Components>;

/// Snapshot `world`, asserting on the way that reads through `world.entity`
/// agree with iteration.
pub fn fingerprint(world: &World) -> Fingerprint {
    let mut fp = Fingerprint::new();
    for eref in world.iter() {
        let e = eref.entity();
        let via_iter = Components::observed(eref);
        // Handle-based access goes through the entity-location metadata,
        // which iteration never consults, so corrupt metadata makes the two
        // disagree (as it did in hecs#465).
        let via_handle = world.entity(e).ok().map(Components::observed);
        assert_eq!(
            via_handle,
            Some(via_iter),
            "world.entity({e:?}) disagrees with iteration"
        );
        assert!(
            fp.insert(e, via_iter).is_none(),
            "world.iter() yielded {e:?} twice"
        );
    }
    assert_eq!(fp.len() as u32, world.len(), "iter() count != world.len()");
    fp
}

/// How many `D` components `world` holds.
pub fn d_in(world: &World) -> usize {
    world.query::<&D>().iter().count()
}

/// `d_in` summed over `worlds`. Each twin holds its own copy of every `D` it
/// was fed, so the tracker's `live()` should equal this sum rather than one
/// world's count.
pub fn total_d(worlds: &[World]) -> usize {
    worlds.iter().map(d_in).sum()
}

/// The archetypes partition the live entities: each id appears in exactly one
/// archetype and the lengths sum to `world.len()`.
pub fn check_archetypes(world: &World, label: &str) {
    let mut total = 0u32;
    let mut ids = HashSet::new();
    for arch in world.archetypes() {
        total += arch.len();
        assert_eq!(
            arch.ids().len(),
            arch.len() as usize,
            "{label}: ids() length != len()"
        );
        for &id in arch.ids() {
            assert!(
                ids.insert(id),
                "{label}: entity id {id} in more than one archetype"
            );
        }
    }
    assert_eq!(
        total,
        world.len(),
        "{label}: archetype lengths sum to {total}, world.len() is {}",
        world.len()
    );
}

/// One step of a world's history: spawn an entity with the given components,
/// or despawn the `i`th entity spawned so far, which is still live at that
/// point.
#[derive(Clone, Copy, Debug, PartialEq, Eq, hegel::PrettyPrintable)]
pub enum Step {
    Spawn(Components),
    Despawn(usize),
}

/// Between `min_steps` and `max_steps` steps, so at most `max_steps` entities.
/// Despawns are interleaved with the spawns so that ids get recycled and
/// generations advance. The first step is always a spawn, so a nonzero
/// `min_steps` guarantees a non-empty handle pool.
#[hegel::composite]
pub fn histories(tc: &hegel::TestCase, min_steps: u32, max_steps: u32) -> Vec<Step> {
    let n = tc.draw(
        gs::integers::<u32>()
            .min_value(min_steps)
            .max_value(max_steps),
    );
    let mut steps = Vec::new();
    // The indices of the spawns still live at this point in the history. A
    // despawn consumes the index it draws, so no entity is despawned twice.
    let mut live = pool::<usize>(tc);
    let mut spawned = 0;
    for _ in 0..n {
        // Despawn with probability 1/4, so histories stay spawn-heavy.
        if !live.is_empty() && tc.draw(gs::weighted_booleans(0.25)) {
            let i = tc.draw(live.values_consumed());
            steps.push(Step::Despawn(i));
        } else {
            steps.push(Step::Spawn(tc.draw(components())));
            live.add(spawned);
            spawned += 1;
        }
    }
    steps
}

/// Replay `history` into `n_worlds` fresh worlds, collecting every handle the
/// replay spawns, in spawn order and despawned ones included.
///
/// `World` is not `Clone`, so this is how a relation between two executions
/// "from the same state" is set up. Cloning through the column API as in
/// examples/cloning.rs would not serve: that example says the clone may hand
/// out different entity ids, and the tests here drive one set of handles
/// into every twin. The replay relies instead on hecs allocating
/// handles deterministically, which the `deterministic_ids` test in
/// src/world.rs pins. The assertions below fail if that ever stops holding.
fn replay(history: &[Step], n_worlds: usize, ds: &DropTracker) -> (Vec<World>, Vec<Entity>) {
    let mut worlds: Vec<World> = (0..n_worlds).map(|_| World::new()).collect();
    let mut handles: Vec<Entity> = Vec::new();
    for step in history {
        match *step {
            Step::Spawn(cs) => {
                let mut spawned = worlds.iter_mut().map(|w| w.spawn(cs.builder(ds).build()));
                let first = spawned.next().expect("at least one world");
                for h in spawned {
                    assert_eq!(first, h, "twin worlds allocated different handles");
                }
                handles.push(first);
            }
            Step::Despawn(i) => {
                for w in &mut worlds {
                    w.despawn(handles[i])
                        .expect("a history only despawns live entities");
                }
            }
        }
    }
    let fp0 = fingerprint(&worlds[0]);
    for (i, w) in worlds.iter().enumerate().skip(1) {
        assert_eq!(fp0, fingerprint(w), "twin world {i} diverged during setup");
    }
    (worlds, handles)
}

/// `replay` into `n_worlds` fresh worlds. The spawned handles come back as a
/// draw pool for `handle_from`.
pub fn build_twins(
    tc: &TestCase,
    history: &[Step],
    n_worlds: usize,
    ds: &DropTracker,
) -> (Vec<World>, Pool<Entity>) {
    let (worlds, handles) = replay(history, n_worlds, ds);
    let mut handle_pool = pool(tc);
    for e in handles {
        handle_pool.add(e);
    }
    (worlds, handle_pool)
}

/// One world replayed from `history`, with the same handle pool.
pub fn build_world_with_handles(
    tc: &TestCase,
    history: &[Step],
    ds: &DropTracker,
) -> (World, Pool<Entity>) {
    let (mut worlds, handle_pool) = build_twins(tc, history, 1, ds);
    (worlds.pop().expect("one world"), handle_pool)
}

/// One world replayed from `history`.
pub fn build_world(history: &[Step], ds: &DropTracker) -> World {
    let (mut worlds, _) = replay(history, 1, ds);
    worlds.pop().expect("one world")
}
