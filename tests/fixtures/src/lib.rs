//! Shared fixtures for the property tests: four component types, an
//! observational fingerprint of a `World`, and twin worlds replayed from one
//! generated history.

use std::cell::Cell;
use std::collections::{BTreeMap, HashSet};

use hecs::{Entity, EntityBuilder, World};
use hegel::generators::{self as gs, Generator};
use serde::{Deserialize, Deserializer, Serialize};

/// The component types the tests use: two same-shaped payload components, a
/// zero-sized marker, and a drop-tracked non-`Copy` component.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct A(pub i32);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct B(pub i32);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct C;

thread_local! { static D_LIVE: Cell<i64> = const { Cell::new(0) }; }

/// Every `D` is constructed through `D::new`, which bumps a thread-local count
/// that `Drop` decrements, so a leak or double-drop in hecs's unsafe component
/// moves shows up as a count that disagrees with the world's contents.
#[derive(Debug, Serialize)]
pub struct D(pub i32);

impl D {
    pub fn new(v: i32) -> D {
        D_LIVE.with(|c| c.set(c.get() + 1));
        D(v)
    }
}

impl Drop for D {
    fn drop(&mut self) {
        D_LIVE.with(|c| c.set(c.get() - 1));
    }
}

impl<'de> Deserialize<'de> for D {
    fn deserialize<De: Deserializer<'de>>(de: De) -> Result<Self, De::Error> {
        i32::deserialize(de).map(D::new)
    }
}

pub fn d_live() -> i64 {
    D_LIVE.with(|c| c.get())
}

/// Cases run one after another on the test's thread, so the thread-local
/// carries over and an imbalance at case start means a previous case leaked or
/// double-dropped.
pub fn assert_d_balanced_at_start() {
    assert_eq!(d_live(), 0, "live D count nonzero at case start");
}

/// Under Miri each operation is interpreted, so these tests run four fixed
/// cases each and serve as a UB oracle for hecs's unsafe component moves
/// rather than as a search for logic bugs. Miri's isolation denies the file
/// and random-device access hegel uses for a fresh seed and for saving
/// failures, and hegel's check that ten cases arrive within thirty seconds
/// would report on the interpreter, so all three are turned off.
#[cfg(miri)]
pub fn settings() -> hegel::Settings {
    hegel::Settings::new()
        .test_cases(4)
        .derandomize(true)
        .database(None)
        .suppress_health_check([hegel::HealthCheck::TooSlow])
}

/// 250 cases per property. Outside CI hegel seeds each run afresh and saves
/// any failing case under `.hegel/` (gitignored) so the next run replays it
/// first. When `CI` or `GITHUB_ACTIONS` is set the seed is fixed per test and
/// nothing is saved, so a failure there reproduces locally with `CI=1`.
#[cfg(not(miri))]
pub fn settings() -> hegel::Settings {
    hegel::Settings::new().test_cases(250)
}

/// Worlds are kept small so that handle collisions, empty archetypes and
/// stale handles all occur often; the bound protects the tests' runtime, not
/// any hecs contract.
#[cfg(miri)]
pub const MAX_ENTITIES: u32 = 4;
#[cfg(not(miri))]
pub const MAX_ENTITIES: u32 = 8;

/// Component payloads are small so that distinct values recur across entities,
/// which is what makes a swapped or stale component visible. Bounds are
/// inclusive.
pub fn val() -> impl gs::PrintableGenerator<i32> {
    gs::integers::<i32>().min_value(-3).max_value(3)
}

/// One of `handles`, drawn uniformly: `sampled_from` made printable, since
/// `Entity` is not `PrettyPrintable`. Callers pass every handle their world
/// has issued, despawned ones included, so a draw often names a dead entity
/// and the `NoSuchEntity` paths get exercised too. `handles` must not be
/// empty.
pub fn handle_from(handles: &[Entity]) -> impl gs::PrintableGenerator<Entity> + '_ {
    gs::sampled_from(handles).print_as_debug()
}

/// The components of one entity: the payloads of its `A`, `B` and `D`, and
/// whether it has a `C`. `D` payloads stay as `i32` here and become `D`
/// values in `builder`, so the drop count only ever counts values that were
/// handed to hecs.
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

    pub fn builder(&self) -> EntityBuilder {
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
            b.add(D::new(v));
        }
        b
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

/// Canonical snapshot of everything a caller can observe about a `World`: the
/// exact `Entity` handles (id and generation) and, per entity, the exact
/// component set and values. Two worlds are observationally equivalent iff
/// their fingerprints compare equal.
pub type Fingerprint = BTreeMap<Entity, Components>;

pub fn fingerprint(world: &World) -> Fingerprint {
    let mut fp = Fingerprint::new();
    for eref in world.iter() {
        let obs = Components {
            a: eref.get::<&A>().map(|r| r.0),
            b: eref.get::<&B>().map(|r| r.0),
            c: eref.get::<&C>().is_some(),
            d: eref.get::<&D>().map(|r| r.0),
        };
        assert!(
            fp.insert(eref.entity(), obs).is_none(),
            "world.iter() yielded {:?} twice",
            eref.entity()
        );
    }
    assert_eq!(fp.len() as u32, world.len(), "iter() count != world.len()");
    fp
}

/// How many `D` components `world` holds.
pub fn d_in(world: &World) -> i64 {
    world.query::<&D>().iter().count() as i64
}

/// `d_in` summed over `worlds`. Each twin holds its own copy of every `D` it
/// was fed, so `d_live()` should equal this sum rather than one world's count.
pub fn total_d(worlds: &[World]) -> i64 {
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
    let mut live: Vec<usize> = Vec::new();
    let mut spawned = 0;
    for _ in 0..n {
        if !live.is_empty() && tc.draw(gs::weighted_booleans(0.25)) {
            let i = tc.draw(gs::sampled_from(&live));
            live.retain(|&l| l != i);
            steps.push(Step::Despawn(i));
        } else {
            steps.push(Step::Spawn(tc.draw(components())));
            live.push(spawned);
            spawned += 1;
        }
    }
    steps
}

/// Replay `history` into `n_worlds` fresh worlds. Also returns every handle
/// the replay spawned, in spawn order and despawned ones included, as the
/// `Vec` callers draw their targets from.
///
/// `World` is not `Clone`, so this is how a relation between two executions
/// "from the same state" is set up. Cloning through the column API as in
/// examples/cloning.rs would not serve: that example says the clone may hand
/// out different entity ids (issue #332), and the tests here drive one set of
/// handles into every twin. The replay relies instead on hecs allocating
/// handles deterministically, which the `deterministic_ids` test in
/// src/world.rs pins. The assertions below fail if that ever stops holding.
pub fn build_twins(history: &[Step], n_worlds: usize) -> (Vec<World>, Vec<Entity>) {
    let mut worlds: Vec<World> = (0..n_worlds).map(|_| World::new()).collect();
    let mut handles: Vec<Entity> = Vec::new();
    for step in history {
        match *step {
            Step::Spawn(cs) => {
                let mut spawned = worlds.iter_mut().map(|w| w.spawn(cs.builder().build()));
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

/// One world replayed from `history`, with the same handle `Vec`.
pub fn build_world_with_handles(history: &[Step]) -> (World, Vec<Entity>) {
    let (mut worlds, handles) = build_twins(history, 1);
    (worlds.pop().expect("one world"), handles)
}

/// One world replayed from `history`.
pub fn build_world(history: &[Step]) -> World {
    build_world_with_handles(history).0
}
