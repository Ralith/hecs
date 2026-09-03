//! Shared fixtures for the property tests: a fixed component universe, an
//! observational fingerprint of a `World`, and twin worlds replayed from one
//! generated history.
//!
//! A property is a test whose inputs come from a `hegel::TestCase`:
//!
//! ```
//! #[hegel::test(settings())]
//! fn a_spawned_component_reads_back(tc: hegel::TestCase) {
//!     assert_d_balanced_at_start();
//!     let v = tc.draw(val());
//!     let mut world = World::new();
//!     let e = world.spawn((A(v),));
//!     assert_eq!(world.get::<&A>(e).unwrap().0, v);
//! }
//! ```
//!
//! `tc.draw(g)` returns one value from the generator `g`. hegel runs the body
//! once per case with fresh draws, and on a failure reruns it on the smallest
//! inputs it can find and prints them. Every `TestCase` method takes `&self`.
//! `TestCase` is `Send` but not `Sync`, so drawing from another thread means
//! moving a clone of it there, which none of these tests do.
//!
//! Each integration-test binary compiles this module separately, so parts of it
//! are unused in some binaries.
#![allow(dead_code)]

use std::cell::Cell;
use std::collections::{BTreeMap, HashSet};

use hecs::{Entity, EntityBuilder, World};
use hegel::generators as gs;
use serde::{Deserialize, Deserializer, Serialize};

/// Component universe: two same-shaped payload components, a zero-sized marker,
/// and a drop-tracked non-`Copy` component.
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

/// hegel runs every case of a property in the test's own thread, one after
/// another, so the thread-local carries over between cases and an imbalance
/// at case start means a previous case leaked or double-dropped.
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
/// which is what makes a swapped or stale component visible. Bounds on hegel's
/// integer generators are inclusive. `draw` accepts only generators that can
/// print what they drew, which is how a failing case's inputs get reported.
pub fn val() -> impl gs::PrintableGenerator<i32> {
    gs::integers::<i32>().min_value(-3).max_value(3)
}

/// Draw from `pool`, which deliberately retains despawned handles.
pub fn pick(tc: &hegel::TestCase, pool: &[Entity]) -> Option<Entity> {
    if pool.is_empty() {
        return None;
    }
    let i = tc.draw(
        gs::integers::<usize>()
            .min_value(0)
            .max_value(pool.len() - 1),
    );
    Some(pool[i])
}

/// An arbitrary subset of the component universe, as drawn payloads. `D`
/// instances are materialized only when a bundle is built, so the drop count
/// tracks exactly the instances that entered a world.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Spec {
    pub a: Option<i32>,
    pub b: Option<i32>,
    pub c: bool,
    pub d: Option<i32>,
}

impl Spec {
    /// How many components an entity built from this spec has.
    pub fn component_count(&self) -> usize {
        self.a.is_some() as usize
            + self.b.is_some() as usize
            + self.c as usize
            + self.d.is_some() as usize
    }
}

// `draw` refuses a generator whose values it cannot print. This prints `Spec`
// with its `Debug` impl.
hegel::pretty_print_as_debug!(Spec);

/// An arbitrary component subset. The attribute turns this into a
/// zero-argument `specs()` returning a generator of `Spec`, and
/// `tc.draw(specs())` supplies the `TestCase`.
#[hegel::composite]
pub fn specs(tc: &hegel::TestCase) -> Spec {
    Spec {
        a: tc.draw(gs::optional(val())),
        b: tc.draw(gs::optional(val())),
        c: tc.draw(gs::booleans()),
        d: tc.draw(gs::optional(val())),
    }
}

pub fn make_builder(s: Spec) -> EntityBuilder {
    let mut b = EntityBuilder::new();
    if let Some(v) = s.a {
        b.add(A(v));
    }
    if let Some(v) = s.b {
        b.add(B(v));
    }
    if s.c {
        b.add(C);
    }
    if let Some(v) = s.d {
        b.add(D::new(v));
    }
    b
}

/// Canonical snapshot of everything a caller can observe about a `World`: the
/// exact `Entity` handles (id and generation) and, per entity, the exact
/// component set and values. Two worlds are observationally equivalent iff
/// their fingerprints compare equal.
pub type Fingerprint = BTreeMap<Entity, Spec>;

pub fn fingerprint(world: &World) -> Fingerprint {
    let mut fp = Fingerprint::new();
    for eref in world.iter() {
        let obs = Spec {
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

/// How many live `D` components a fingerprint accounts for.
pub fn fingerprint_d_count(fp: &Fingerprint) -> i64 {
    fp.values().filter(|o| o.d.is_some()).count() as i64
}

/// Live `D` components summed over `worlds`. Each twin holds its own copy of
/// every `D` it was fed, so `d_live()` should equal this sum rather than one
/// world's count.
pub fn total_d(worlds: &[World]) -> i64 {
    worlds
        .iter()
        .map(|w| fingerprint_d_count(&fingerprint(w)))
        .sum()
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

/// Build `n_worlds` observationally identical worlds by replaying one drawn
/// history (spawns of arbitrary specs, then a drawn subset of despawns) into
/// each, and return them with a target pool that retains the despawned handles.
///
/// `World` is not `Clone`, so this is how a relation between two executions
/// "from the same state" is set up. It relies on hecs allocating handles
/// deterministically, which the `deterministic_ids` test in src/world.rs pins.
/// The assertions below fail if it ever stops holding.
pub fn build_twins(
    tc: &hegel::TestCase,
    n_worlds: usize,
    max_entities: u32,
) -> (Vec<World>, Vec<Entity>) {
    let mut worlds: Vec<World> = (0..n_worlds).map(|_| World::new()).collect();
    let mut pool: Vec<Entity> = Vec::new();

    let n = tc.draw(gs::integers::<u32>().min_value(0).max_value(max_entities));
    for _ in 0..n {
        let s = tc.draw(specs());
        let mut handles = worlds.iter_mut().map(|w| w.spawn(make_builder(s).build()));
        let first = handles.next().expect("at least one world");
        for h in handles {
            assert_eq!(first, h, "twin worlds allocated different handles");
        }
        pool.push(first);
    }
    for &e in &pool {
        if tc.draw(gs::booleans()) {
            let mut oks = worlds.iter_mut().map(|w| w.despawn(e).is_ok());
            let first = oks.next().expect("at least one world");
            for ok in oks {
                assert_eq!(first, ok, "twin despawn disagreed for {e:?}");
            }
        }
    }
    let fp0 = fingerprint(&worlds[0]);
    for (i, w) in worlds.iter().enumerate().skip(1) {
        assert_eq!(fp0, fingerprint(w), "twin world {i} diverged during setup");
    }
    (worlds, pool)
}
