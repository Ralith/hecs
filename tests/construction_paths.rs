//! Differential tests over the several API routes that build the same logical
//! world, plus `CommandBuffer` against eager application.
//!
//! hecs allocates handles deterministically, so worlds built by different
//! routes from the same drawn components must be observationally identical:
//! same handles, same component sets, same values. The drawn components are
//! the ground truth, so the routes cannot all agree on a wrong answer. After
//! construction, the same mutations are applied to every world: internally they
//! differ (the incremental route left a chain of intermediate archetypes behind,
//! the tuple route did not), so this phase checks that construction history is
//! not observable.

use fixtures::*;
use hecs::{CommandBuffer, Entity, World};
use hegel::generators::{self as gs, Generator};

/// The routes that spawn drawn components and return the handle. The
/// `CommandBuffer` route is driven separately, since `CommandBuffer::spawn`
/// does not return one.
const ROUTES: [(&str, fn(&mut World, Components, &DropTracker) -> Entity); 4] = [
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
    match (cs.a, cs.b, cs.c, cs.d) {
        (None, None, false, None) => world.spawn(()),
        (Some(a), None, false, None) => world.spawn((A(a),)),
        (None, Some(b), false, None) => world.spawn((B(b),)),
        (None, None, true, None) => world.spawn((C,)),
        (None, None, false, Some(d)) => world.spawn((D::new(d, ds),)),
        (Some(a), Some(b), false, None) => world.spawn((A(a), B(b))),
        (Some(a), None, true, None) => world.spawn((A(a), C)),
        (Some(a), None, false, Some(d)) => world.spawn((A(a), D::new(d, ds))),
        (None, Some(b), true, None) => world.spawn((B(b), C)),
        (None, Some(b), false, Some(d)) => world.spawn((B(b), D::new(d, ds))),
        (None, None, true, Some(d)) => world.spawn((C, D::new(d, ds))),
        (Some(a), Some(b), true, None) => world.spawn((A(a), B(b), C)),
        (Some(a), Some(b), false, Some(d)) => world.spawn((A(a), B(b), D::new(d, ds))),
        (Some(a), None, true, Some(d)) => world.spawn((A(a), C, D::new(d, ds))),
        (None, Some(b), true, Some(d)) => world.spawn((B(b), C, D::new(d, ds))),
        (Some(a), Some(b), true, Some(d)) => world.spawn((A(a), B(b), C, D::new(d, ds))),
    }
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

/// The same entities built through `EntityBuilder`, static tuple bundles,
/// `reserve_entity` + `insert`, `CommandBuffer::spawn`, and a chain of
/// `insert_one` calls are observationally identical, and stay identical under
/// any subsequent operations.
#[hegel::test(settings())]
fn construction_routes_are_observationally_equal(tc: hegel::TestCase) {
    let ds = DropTracker::new();
    // A shared history puts every allocator into the same non-trivial state
    // (non-empty freelist, advanced generations) before the routes diverge.
    let history = tc.draw(histories(0, MAX_ENTITIES));
    let (mut worlds, mut pool) = build_twins(&history, ROUTES.len() + 1, &ds);

    let to_spawn: Vec<Components> = tc.draw(gs::vecs(components()).max_size(MAX_ENTITIES as usize));

    let mut buffer = CommandBuffer::new();
    for &cs in &to_spawn {
        buffer.spawn(cs.builder(&ds).build());
    }

    let handles: Vec<Vec<Entity>> = ROUTES
        .iter()
        .zip(&mut worlds)
        .map(|(&(_, route), world)| to_spawn.iter().map(|&cs| route(world, cs, &ds)).collect())
        .collect();
    // CommandBuffer does not surface the handles it allocates; they are
    // recovered from the fingerprint comparison below.
    buffer.run_on(&mut worlds[ROUTES.len()]);

    for (&(name, _), route) in ROUTES.iter().zip(&handles).skip(1) {
        assert_eq!(
            &handles[0], route,
            "the {name} route allocated different handles"
        );
    }
    pool.extend(handles[0].iter().copied());

    let expected = fingerprint(&worlds[0]);
    for (e, cs) in handles[0].iter().zip(&to_spawn) {
        assert_eq!(
            expected.get(e),
            Some(cs),
            "the builder route disagrees with the drawn components for {e:?}"
        );
    }
    for (i, w) in worlds.iter().enumerate().skip(1) {
        assert_eq!(
            expected,
            fingerprint(w),
            "the {} route built an observationally different world",
            route_name(i)
        );
        check_archetypes(w, "constructed world");
    }
    assert_eq!(
        ds.live(),
        total_d(&worlds),
        "drop imbalance across construction routes"
    );

    if pool.is_empty() {
        return;
    }
    let steps = tc.draw(gs::integers::<u32>().min_value(0).max_value(MAX_ENTITIES));
    for _ in 0..steps {
        let e = tc.draw(handle_from(&pool));
        let m = tc.draw(ops());
        let mut results = worlds.iter_mut().map(|w| apply(w, e, m, &ds));
        let first = results.next().expect("at least one world");
        for (i, r) in results.enumerate() {
            assert_eq!(
                first,
                r,
                "{m:?} on {e:?} returned a different result in the {} route",
                route_name(i + 1)
            );
        }
        let after = fingerprint(&worlds[0]);
        for (i, w) in worlds.iter().enumerate().skip(1) {
            assert_eq!(
                after,
                fingerprint(w),
                "the {} route diverged after {m:?} on {e:?}",
                route_name(i)
            );
        }
    }
    assert_eq!(
        ds.live(),
        total_d(&worlds),
        "drop imbalance after the mutation phase"
    );
}

/// One logical operation, recorded into a `CommandBuffer` and also applied
/// directly.
#[derive(Clone, Copy, Debug)]
enum Command {
    Spawn(Components),
    Insert(Entity, Components),
    InsertOne(Entity, Kind, i32),
    RemoveAB(Entity),
    RemoveCD(Entity),
    RemoveOne(Entity, Kind),
    Despawn(Entity),
}

/// A command on a handle from `pool`, or a spawn. A quarter of the commands
/// spawn, so the pool grows fast enough for the other commands to have
/// entities to act on. An empty pool only gets spawns.
#[hegel::composite]
fn commands_on(tc: &hegel::TestCase, pool: &[Entity]) -> Command {
    if pool.is_empty() || tc.draw(gs::weighted_booleans(0.25)) {
        return Command::Spawn(tc.draw(components()));
    }
    let e = tc.draw(handle_from(pool));
    tc.draw(
        hegel::one_of!(
            hegel::compose!(|tc| { Command::Insert(e, tc.draw(components())) }),
            hegel::compose!(|tc| { Command::InsertOne(e, tc.draw(kinds()), tc.draw(val())) }),
            gs::just(Command::RemoveAB(e)),
            gs::just(Command::RemoveCD(e)),
            hegel::compose!(|tc| { Command::RemoveOne(e, tc.draw(kinds())) }),
            gs::just(Command::Despawn(e)),
        )
        .print_as_debug(),
    )
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

/// `run_on` is documented to replay commands in order, ignoring failures, so
/// each command's eager equivalent discards its error.
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

/// What is done with the buffer once a round of commands is recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, hegel::PrettyPrintable)]
enum Outcome {
    Run,
    Clear,
    Drop,
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

/// Replaying a `CommandBuffer` is equivalent to applying the same operations
/// directly and ignoring failures. Comparing the worlds exactly (handles
/// included) is well-defined because both undergo the same logical sequence,
/// so `CommandBuffer::spawn` — whose handle is never surfaced — must allocate
/// the handle the eager `spawn` at the same position did.
#[hegel::test(settings())]
fn command_buffer_matches_eager_application(tc: hegel::TestCase) {
    let ds = DropTracker::new();
    let history = tc.draw(histories(0, MAX_ENTITIES));
    let (mut worlds, mut pool) = build_twins(&history, 2, &ds);

    // `CommandBuffer::insert` documents `reserve_entity` as the way to obtain a
    // handle for an entity that does not exist yet.
    let reserved = tc.draw(gs::integers::<u32>().min_value(0).max_value(3));
    for _ in 0..reserved {
        let eager = worlds[0].reserve_entity();
        let buffered = worlds[1].reserve_entity();
        assert_eq!(eager, buffered, "reserve_entity was not deterministic");
        pool.push(eager);
    }

    let mut buffer = CommandBuffer::new();
    let rounds = tc.draw(gs::integers::<u32>().min_value(1).max_value(4));
    for _ in 0..rounds {
        let commands: Vec<Command> =
            tc.draw(gs::vecs(commands_on(&pool).print_as_debug()).max_size(12));
        for &c in &commands {
            record(&mut buffer, c, &ds);
        }

        // Recording must not touch either world, and every `D` handed to the
        // buffer must still be alive and unduplicated inside it.
        assert_eq!(
            fingerprint(&worlds[0]),
            fingerprint(&worlds[1]),
            "recording commands changed a world"
        );
        assert_eq!(
            ds.live(),
            total_d(&worlds) + pending_d(&commands),
            "a buffered D was dropped early or leaked"
        );

        let outcome = tc.draw(gs::sampled_from(
            &[Outcome::Run, Outcome::Clear, Outcome::Drop][..],
        ));
        match outcome {
            Outcome::Run => {
                buffer.run_on(&mut worlds[1]);
                for &c in &commands {
                    apply_eagerly(&mut worlds[0], c, &ds);
                }
                assert_eq!(
                    fingerprint(&worlds[0]),
                    fingerprint(&worlds[1]),
                    "run_on diverged from eager application"
                );
                // `run_on` empties the buffer, so replaying it is a no-op and
                // the buffer is reusable for the next round.
                buffer.run_on(&mut worlds[1]);
                assert_eq!(
                    fingerprint(&worlds[0]),
                    fingerprint(&worlds[1]),
                    "a second run_on applied the commands again"
                );
            }
            // `clear` discards the recorded commands.
            Outcome::Clear => {
                buffer.clear();
                buffer.run_on(&mut worlds[1]);
                assert_eq!(
                    fingerprint(&worlds[0]),
                    fingerprint(&worlds[1]),
                    "commands survived clear()"
                );
            }
            // Dropping a non-empty buffer must release its stored components.
            Outcome::Drop => buffer = CommandBuffer::new(),
        }

        assert_eq!(
            ds.live(),
            total_d(&worlds),
            "drop imbalance after the buffer was consumed or discarded"
        );

        for eref in worlds[0].iter() {
            let e = eref.entity();
            if !pool.contains(&e) {
                pool.push(e);
            }
        }
    }
}
