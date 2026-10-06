//! Differential tests over the several API routes that build the same logical
//! world, plus `CommandBuffer` against eager application.
//!
//! hecs allocates handles deterministically, so worlds built by different
//! routes from the same drawn components must be observationally identical:
//! same handles, same component sets, same values. The drawn components are
//! the ground truth, so the routes cannot all agree on a wrong answer. The
//! worlds are driven in lockstep by a state machine whose mutation rule also
//! checks that construction history is not observable: internally the worlds
//! differ (the incremental route left a chain of intermediate archetypes
//! behind, the tuple route did not), but the same operations must leave them
//! observationally equal.

use std::collections::HashSet;

use fixtures::*;
use hecs::{CommandBuffer, DynamicBundle, DynamicBundleClone, Entity, World};
use hegel::generators as gs;
use hegel::stateful::Pool;
use hegel::TestCase;


type Route = fn(&mut World, Components, &DropTracker) -> Entity;

/// The routes that spawn drawn components and return the handle. The
/// `CommandBuffer` route is driven separately, since `CommandBuffer::spawn`
/// does not return one.
const ROUTES: [(&str, Route); 4] = [
    ("EntityBuilder", spawn_builder),
    ("static tuple", spawn_tuple),
    ("reserve_entity + insert", spawn_reserved),
    ("insert_one chain", spawn_incrementally),
];

/// The name of the `i`th twin world: a `ROUTES` entry, then the buffer world.
fn route_name(i: usize) -> &'static str {
    ROUTES.get(i).map_or("CommandBuffer", |route| route.0)
}

fn spawn_builder(world: &mut World, cs: Components, ds: &DropTracker) -> Entity {
    world.spawn(cs.builder(ds).build())
}

/// The static `Bundle` impls, one concrete tuple type per subset of
/// `{A, B, C, D}`.
fn spawn_tuple(world: &mut World, cs: Components, ds: &DropTracker) -> Entity {
    struct Spawn<'a>(&'a mut World, Option<Entity>);
    impl TupleSink for Spawn<'_> {
        fn put(&mut self, bundle: impl DynamicBundleClone) {
            self.1 = Some(self.0.spawn(bundle));
        }
        fn put_with_d(&mut self, bundle: impl DynamicBundle) {
            self.1 = Some(self.0.spawn(bundle));
        }
    }
    let mut sink = Spawn(world, None);
    put_as_tuple(&mut sink, cs, ds);
    sink.1.expect("put_as_tuple hands over exactly one bundle")
}

fn spawn_reserved(world: &mut World, cs: Components, ds: &DropTracker) -> Entity {
    let e = world.reserve_entity();
    world
        .insert(e, cs.builder(ds).build())
        .expect("insert on a freshly reserved entity");
    e
}

/// An empty entity migrated through one intermediate archetype per component.
fn spawn_incrementally(world: &mut World, cs: Components, ds: &DropTracker) -> Entity {
    let e = world.spawn(());
    if let Some(v) = cs.a {
        world
            .insert_one(e, A(v))
            .expect("insert on a just-spawned entity");
    }
    if let Some(v) = cs.b {
        world
            .insert_one(e, B(v))
            .expect("insert on a just-spawned entity");
    }
    if cs.c {
        world
            .insert_one(e, C)
            .expect("insert on a just-spawned entity");
    }
    if let Some(v) = cs.d {
        world
            .insert_one(e, D::new(v, ds))
            .expect("insert on a just-spawned entity");
    }
    e
}

/// The worlds being built in lockstep: one per `ROUTES` entry, and a last one
/// driven through a `CommandBuffer`.
struct RouteTwins {
    worlds: Vec<World>,
    /// Every handle the routes handed out, despawned ones included.
    handles: Pool<Entity>,
    ds: DropTracker,
}

// The spawn rule drives every construction route with the same drawn
// components, and the mutate rule applies identical operations, so any
// observable trace of how a world was built shows up as divergence.
#[hegel::state_machine]
impl RouteTwins {
    /// Spawn one drawn entity through every route. Each route must hand out
    /// the same handle, carrying exactly the drawn components.
    #[rule]
    fn spawn_via_every_route(&mut self, tc: TestCase) {
        let cs = tc.draw(components());
        let handles: Vec<Entity> = ROUTES
            .iter()
            .zip(&mut self.worlds)
            .map(|(&(_, route), world)| route(world, cs, &self.ds))
            .collect();
        // `CommandBuffer::spawn` does not surface the handle it allocates.
        // The `routes_agree` invariant checks that the buffer world spawned
        // the same entity.
        let mut buffer = CommandBuffer::new();
        buffer.spawn(cs.builder(&self.ds).build());
        buffer.run_on(self.worlds.last_mut().expect("the buffer world"));

        let e = handles[0];
        for (i, &h) in handles.iter().enumerate().skip(1) {
            assert_eq!(
                e,
                h,
                "the {} route allocated a different handle",
                route_name(i)
            );
        }
        assert_eq!(
            self.worlds[0].entity(e).ok().map(Components::observed),
            Some(cs),
            "the builder route disagrees with the drawn components for {e:?}"
        );
        self.handles.add(e);
    }

    /// Apply one operation to the same entity in every world: the results
    /// must agree, and so must what is left on the entity.
    #[rule]
    fn mutate(&mut self, tc: TestCase) {
        let e = tc.draw(handle_from(&self.handles));
        let m = tc.draw(ops());
        let mut results = self.worlds.iter_mut().map(|w| apply(w, e, m, &self.ds));
        let first = results.next().expect("at least one world");
        for (i, r) in results.enumerate() {
            assert_eq!(
                first,
                r,
                "{m:?} on {e:?} returned a different result in the {} route",
                route_name(i + 1)
            );
        }
        let observed = self.worlds[0].entity(e).ok().map(Components::observed);
        for (i, w) in self.worlds.iter().enumerate().skip(1) {
            assert_eq!(
                observed,
                w.entity(e).ok().map(Components::observed),
                "the {} route diverged after {m:?} on {e:?}",
                route_name(i)
            );
        }
    }

    #[invariant]
    fn routes_agree(&self, _: TestCase) {
        let expected = fingerprint(&self.worlds[0]);
        for (i, w) in self.worlds.iter().enumerate().skip(1) {
            assert_eq!(
                expected,
                fingerprint(w),
                "the {} route built an observationally different world",
                route_name(i)
            );
        }
        for w in &self.worlds {
            check_archetypes(w, "route world");
        }
    }

    #[invariant]
    fn drops_balance(&self, _: TestCase) {
        assert_eq!(
            self.ds.live(),
            total_d(&self.worlds),
            "drop imbalance across the routes"
        );
    }
}

/// The same entities built through `EntityBuilder`, static tuple bundles,
/// `reserve_entity` + `insert`, `CommandBuffer::spawn`, and a chain of
/// `insert_one` calls are observationally identical, and stay identical under
/// any subsequent operations.
#[hegel::test(settings().stateful_step_count(STEPS))]
fn construction_routes_are_observationally_equal(tc: TestCase) {
    let ds = DropTracker::new();
    // A shared history puts every allocator into the same non-trivial state
    // (non-empty freelist, advanced generations) before the routes diverge.
    let history = tc.draw(histories(0, MAX_ENTITIES));
    let (worlds, handles) = build_twins(&tc, &history, ROUTES.len() + 1, &ds);
    hegel::stateful::run(
        RouteTwins {
            worlds,
            handles,
            ds,
        },
        tc,
    );
}

/// One logical operation, recorded into a `CommandBuffer` and also applied
/// directly. It differs from `fixtures::Op`: a command carries its target
/// entity, spawns are in scope (both worlds see the same sequence, so the
/// handles they allocate agree), and `CommandBuffer` has no counterpart to
/// take, exchange, or a `get_mut` write.
#[derive(Clone, Copy, Debug, hegel::PrettyPrintable)]
enum Command {
    Spawn(Components),
    Insert(#[pretty(debug)] Entity, Components),
    InsertOne(#[pretty(debug)] Entity, Kind, i32),
    RemoveAB(#[pretty(debug)] Entity),
    RemoveCD(#[pretty(debug)] Entity),
    RemoveOne(#[pretty(debug)] Entity, Kind),
    Despawn(#[pretty(debug)] Entity),
}

fn record(buffer: &mut CommandBuffer, c: Command, ds: &DropTracker) {
    match c {
        Command::Spawn(cs) => buffer.spawn(cs.builder(ds).build()),
        Command::Insert(e, cs) => buffer.insert(e, cs.builder(ds).build()),
        Command::InsertOne(e, kind, v) => kind.buffer_insert_one(buffer, e, v, ds),
        Command::RemoveAB(e) => buffer.remove::<(A, B)>(e),
        Command::RemoveCD(e) => buffer.remove::<(C, D)>(e),
        Command::RemoveOne(e, kind) => kind.buffer_remove_one(buffer, e),
        Command::Despawn(e) => buffer.despawn(e),
    }
}

/// `run_on`'s doc promises running the recorded commands and clearing the
/// buffer, nothing more. In-order replay and failure-swallowing are de-facto
/// behavior these tests pin, so each command's eager equivalent discards its
/// error.
fn apply_eagerly(world: &mut World, c: Command, ds: &DropTracker) {
    match c {
        Command::Spawn(cs) => {
            world.spawn(cs.builder(ds).build());
        }
        Command::Insert(e, cs) => drop(world.insert(e, cs.builder(ds).build())),
        Command::InsertOne(e, kind, v) => drop(kind.insert_one(world, e, v, ds)),
        Command::RemoveAB(e) => drop(world.remove::<(A, B)>(e)),
        Command::RemoveCD(e) => drop(world.remove::<(C, D)>(e)),
        Command::RemoveOne(e, kind) => drop(kind.remove_one(world, e)),
        Command::Despawn(e) => drop(world.despawn(e)),
    }
}

/// How many `D` values a recorded sequence is holding inside the buffer.
fn pending_d(commands: &[Command]) -> usize {
    commands
        .iter()
        .map(|c| match c {
            Command::Spawn(cs) | Command::Insert(_, cs) => cs.d.is_some() as usize,
            Command::InsertOne(_, Kind::D, _) => 1,
            _ => 0,
        })
        .sum()
}

/// The world commands are applied to eagerly and the one they reach through
/// the buffer.
struct BufferVsEager {
    eager: World,
    buffered: World,
    buffer: CommandBuffer,
    /// Commands recorded into `buffer` and not yet run, cleared, or dropped.
    pending: Vec<Command>,
    /// Every handle either world has issued, despawned ones included.
    handles: Pool<Entity>,
    /// What `handles` holds, so `adopt_spawned_handles` adds each handle once.
    known: HashSet<Entity>,
    ds: DropTracker,
}

impl BufferVsEager {
    /// A command on a pooled handle, or a spawn. A quarter of the commands
    /// spawn, so the pool grows fast enough for the other commands to have
    /// entities to act on. An empty pool only gets spawns.
    fn draw_command(&self, tc: &TestCase) -> Command {
        if self.handles.is_empty() || tc.draw(gs::weighted_booleans(0.25)) {
            return Command::Spawn(tc.draw(components()));
        }
        let e = tc.draw(handle_from(&self.handles));
        tc.draw(hegel::one_of!(
            hegel::compose!(|tc| { Command::Insert(e, tc.draw(components())) }),
            hegel::compose!(|tc| { Command::InsertOne(e, tc.draw(kinds()), tc.draw(val())) }),
            gs::just(Command::RemoveAB(e)),
            gs::just(Command::RemoveCD(e)),
            hegel::compose!(|tc| { Command::RemoveOne(e, tc.draw(kinds())) }),
            gs::just(Command::Despawn(e)),
        ))
    }

    fn add_handle(&mut self, e: Entity) {
        if self.known.insert(e) {
            self.handles.add(e);
        }
    }

    /// Pool the handles of entities that exist only in the worlds: the ones
    /// `Command::Spawn` created, whose handles `CommandBuffer` never
    /// surfaces.
    fn adopt_spawned_handles(&mut self) {
        let live: Vec<Entity> = self.eager.iter().map(|eref| eref.entity()).collect();
        for e in live {
            self.add_handle(e);
        }
    }
}

// The rules interleave recording, running, clearing, and dropping the buffer
// with the drop-tracked `D` in play, so a buffer that applies a command
// wrongly, twice, or not at all diverges from the eager world, and one that
// mishandles a stored component unbalances the drop count.
#[hegel::state_machine]
impl BufferVsEager {
    /// `CommandBuffer::insert` documents pairing with `World::reserve_entity`
    /// to spawn entities with a known handle. Both worlds have seen the same
    /// operations, so they must reserve the same handle.
    #[rule]
    fn reserve(&mut self, _: TestCase) {
        let in_eager = self.eager.reserve_entity();
        let in_buffered = self.buffered.reserve_entity();
        assert_eq!(
            in_eager, in_buffered,
            "reserve_entity was not deterministic"
        );
        self.add_handle(in_eager);
    }

    /// Record a batch of drawn commands. Recording only writes to the
    /// buffer, so neither world may change.
    #[rule]
    fn record_commands(&mut self, tc: TestCase) {
        let before = (fingerprint(&self.eager), fingerprint(&self.buffered));
        let n = tc.draw(gs::integers::<u32>().max_value(12));
        for _ in 0..n {
            let c = self.draw_command(&tc);
            record(&mut self.buffer, c, &self.ds);
            self.pending.push(c);
        }
        assert_eq!(
            before,
            (fingerprint(&self.eager), fingerprint(&self.buffered)),
            "recording commands changed a world"
        );
    }

    /// Applying the recorded commands directly must leave the same world
    /// `run_on` does. `run_on` is documented to clear the buffer, so a
    /// second `run_on` is a no-op and the buffer is reusable.
    #[rule]
    fn run(&mut self, _: TestCase) {
        self.buffer.run_on(&mut self.buffered);
        for &c in &self.pending {
            apply_eagerly(&mut self.eager, c, &self.ds);
        }
        self.pending.clear();
        assert_eq!(
            fingerprint(&self.eager),
            fingerprint(&self.buffered),
            "run_on diverged from eager application"
        );
        self.buffer.run_on(&mut self.buffered);
        assert_eq!(
            fingerprint(&self.eager),
            fingerprint(&self.buffered),
            "a second run_on applied the commands again"
        );
        self.adopt_spawned_handles();
    }

    /// `clear` discards the recorded commands, so a subsequent `run_on`
    /// applies nothing.
    #[rule]
    fn clear(&mut self, _: TestCase) {
        self.buffer.clear();
        self.pending.clear();
        self.buffer.run_on(&mut self.buffered);
        assert_eq!(
            fingerprint(&self.eager),
            fingerprint(&self.buffered),
            "commands survived clear()"
        );
    }

    /// Dropping a non-empty buffer must release its stored components. The
    /// `drops_balance` invariant catches a leak.
    #[rule]
    fn drop_buffer(&mut self, _: TestCase) {
        self.buffer = CommandBuffer::new();
        self.pending.clear();
    }

    /// The eager world only ever changes in `run`, in lockstep with the
    /// buffered one, so the two agree between rules too.
    #[invariant]
    fn worlds_agree(&self, _: TestCase) {
        assert_eq!(
            fingerprint(&self.eager),
            fingerprint(&self.buffered),
            "the eager and the buffered world diverged"
        );
    }

    /// Every `D` handed to the buffer must stay alive and unduplicated
    /// inside it until the buffer is run, cleared, or dropped.
    #[invariant]
    fn drops_balance(&self, _: TestCase) {
        assert_eq!(
            self.ds.live(),
            d_in(&self.eager) + d_in(&self.buffered) + pending_d(&self.pending),
            "a buffered D was dropped early or leaked"
        );
    }
}

/// Replaying a `CommandBuffer` is equivalent to applying the same operations
/// directly and ignoring failures. Comparing the worlds exactly (handles
/// included) is well-defined because both undergo the same logical sequence,
/// so `CommandBuffer::spawn` — whose handle is never surfaced — must allocate
/// the handle the eager `spawn` at the same position did.
#[hegel::test(settings().stateful_step_count(STEPS))]
fn command_buffer_matches_eager_application(tc: TestCase) {
    let ds = DropTracker::new();
    let history = tc.draw(histories(0, MAX_ENTITIES));
    let (mut worlds, handles) = build_twins(&tc, &history, 2, &ds);
    let buffered = worlds.pop().expect("two worlds");
    let eager = worlds.pop().expect("two worlds");
    // The pool already holds every history handle. Seeding `known` with the
    // live ones stops `adopt_spawned_handles` from pooling them again.
    let known: HashSet<Entity> = eager.iter().map(|eref| eref.entity()).collect();

    let machine = BufferVsEager {
        eager,
        buffered,
        buffer: CommandBuffer::new(),
        pending: Vec::new(),
        handles,
        known,
        ds,
    };
    hegel::stateful::run(machine, tc);
}
