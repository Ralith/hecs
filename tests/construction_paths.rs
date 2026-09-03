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

const ROUTES: usize = 5;

/// Route 2: the static `Bundle` impls, one concrete tuple type per subset of
/// `{A, B, C, D}`.
fn spawn_tuple(world: &mut World, cs: Components) -> Entity {
    match (cs.a, cs.b, cs.c, cs.d) {
        (None, None, false, None) => world.spawn(()),
        (Some(a), None, false, None) => world.spawn((A(a),)),
        (None, Some(b), false, None) => world.spawn((B(b),)),
        (None, None, true, None) => world.spawn((C,)),
        (None, None, false, Some(d)) => world.spawn((D::new(d),)),
        (Some(a), Some(b), false, None) => world.spawn((A(a), B(b))),
        (Some(a), None, true, None) => world.spawn((A(a), C)),
        (Some(a), None, false, Some(d)) => world.spawn((A(a), D::new(d))),
        (None, Some(b), true, None) => world.spawn((B(b), C)),
        (None, Some(b), false, Some(d)) => world.spawn((B(b), D::new(d))),
        (None, None, true, Some(d)) => world.spawn((C, D::new(d))),
        (Some(a), Some(b), true, None) => world.spawn((A(a), B(b), C)),
        (Some(a), Some(b), false, Some(d)) => world.spawn((A(a), B(b), D::new(d))),
        (Some(a), None, true, Some(d)) => world.spawn((A(a), C, D::new(d))),
        (None, Some(b), true, Some(d)) => world.spawn((B(b), C, D::new(d))),
        (Some(a), Some(b), true, Some(d)) => world.spawn((A(a), B(b), C, D::new(d))),
    }
}

/// Route 5: an empty entity migrated through one intermediate archetype per
/// component.
fn spawn_incrementally(world: &mut World, cs: Components) -> Entity {
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
            .insert_one(e, D::new(v))
            .expect("insert on a just-spawned entity");
    }
    e
}

#[derive(Clone, Copy, Debug, hegel::PrettyPrintable)]
enum Mutation {
    InsertOne(u8, i32),
    RemoveOne(u8),
    InsertBundle(Components),
    RemoveAB,
    Despawn,
    ExchangeAToB(i32),
}

#[hegel::composite]
fn mutations(tc: &hegel::TestCase) -> Mutation {
    fn which() -> impl hegel::generators::PrintableGenerator<u8> {
        gs::integers::<u8>().min_value(0).max_value(3)
    }
    tc.draw(hegel::one_of!(
        hegel::compose!(|tc| { Mutation::InsertOne(tc.draw(which()), tc.draw(val())) }),
        hegel::compose!(|tc| { Mutation::RemoveOne(tc.draw(which())) }),
        hegel::compose!(|tc| { Mutation::InsertBundle(tc.draw(components())) }),
        gs::just(Mutation::RemoveAB),
        gs::just(Mutation::Despawn),
        hegel::compose!(|tc| { Mutation::ExchangeAToB(tc.draw(val())) }),
    ))
}

fn apply(world: &mut World, e: Entity, m: Mutation) -> bool {
    match m {
        Mutation::InsertOne(which, v) => match which {
            0 => world.insert_one(e, A(v)).is_ok(),
            1 => world.insert_one(e, B(v)).is_ok(),
            2 => world.insert_one(e, C).is_ok(),
            _ => world.insert_one(e, D::new(v)).is_ok(),
        },
        Mutation::RemoveOne(which) => match which {
            0 => world.remove_one::<A>(e).is_ok(),
            1 => world.remove_one::<B>(e).is_ok(),
            2 => world.remove_one::<C>(e).is_ok(),
            _ => world.remove_one::<D>(e).is_ok(),
        },
        Mutation::InsertBundle(cs) => world.insert(e, cs.builder().build()).is_ok(),
        Mutation::RemoveAB => world.remove::<(A, B)>(e).is_ok(),
        Mutation::Despawn => world.despawn(e).is_ok(),
        Mutation::ExchangeAToB(v) => world.exchange_one::<A, B>(e, B(v)).is_ok(),
    }
}

/// The same entities built through `EntityBuilder`, static tuple bundles,
/// `reserve_entity` + `insert`, `CommandBuffer::spawn`, and a chain of
/// `insert_one` calls are observationally identical, and stay identical under
/// any subsequent operations.
#[hegel::test(settings())]
fn construction_routes_are_observationally_equal(tc: hegel::TestCase) {
    assert_d_balanced_at_start();
    // A shared history puts every allocator into the same non-trivial state
    // (non-empty freelist, advanced generations) before the routes diverge.
    let history = tc.draw(histories(0, MAX_ENTITIES));
    let (mut worlds, mut pool) = build_twins(&history, ROUTES);

    let to_spawn: Vec<Components> = tc.draw(gs::vecs(components()).max_size(MAX_ENTITIES as usize));

    let mut buffer = CommandBuffer::new();
    for &cs in &to_spawn {
        buffer.spawn(cs.builder().build());
    }

    let mut handles: Vec<Vec<Entity>> = Vec::with_capacity(ROUTES);
    handles.push(
        to_spawn
            .iter()
            .map(|&cs| worlds[0].spawn(cs.builder().build()))
            .collect(),
    );
    handles.push(
        to_spawn
            .iter()
            .map(|&cs| spawn_tuple(&mut worlds[1], cs))
            .collect(),
    );
    handles.push(
        to_spawn
            .iter()
            .map(|&cs| {
                let e = worlds[2].reserve_entity();
                worlds[2]
                    .insert(e, cs.builder().build())
                    .expect("insert on a freshly reserved entity");
                e
            })
            .collect(),
    );
    // CommandBuffer does not surface the handles it allocates; they are
    // recovered from the fingerprint comparison below.
    buffer.run_on(&mut worlds[3]);
    handles.push(
        to_spawn
            .iter()
            .map(|&cs| spawn_incrementally(&mut worlds[4], cs))
            .collect(),
    );

    for (i, route) in handles.iter().enumerate().skip(1) {
        assert_eq!(&handles[0], route, "route {i} allocated different handles");
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
            "route {i} built an observationally different world"
        );
        check_archetypes(w, "constructed world");
    }
    assert_eq!(
        d_live(),
        total_d(&worlds),
        "drop imbalance across construction routes"
    );

    if pool.is_empty() {
        return;
    }
    let steps = tc.draw(gs::integers::<u32>().min_value(0).max_value(MAX_ENTITIES));
    for _ in 0..steps {
        let e = tc.draw(handle_from(&pool));
        let m = tc.draw(mutations());
        let mut results = worlds.iter_mut().map(|w| apply(w, e, m));
        let first = results.next().expect("at least one world");
        for (i, r) in results.enumerate() {
            assert_eq!(
                first,
                r,
                "{m:?} on {e:?} returned a different result in route {}",
                i + 1
            );
        }
        let after = fingerprint(&worlds[0]);
        for (i, w) in worlds.iter().enumerate().skip(1) {
            assert_eq!(
                after,
                fingerprint(w),
                "route {i} diverged after {m:?} on {e:?}"
            );
        }
    }
    assert_eq!(
        d_live(),
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
    InsertOne(Entity, u8, i32),
    RemoveAB(Entity),
    RemoveCD(Entity),
    RemoveOne(Entity, u8),
    Despawn(Entity),
}

/// A command on a handle from `pool`, or a spawn. Two of the eight kinds
/// spawn, so the pool grows fast enough for the other commands to have
/// entities to act on. An empty pool only gets spawns.
#[hegel::composite]
fn commands_on(tc: &hegel::TestCase, pool: &[Entity]) -> Command {
    let which = gs::integers::<u8>().min_value(0).max_value(3);
    let kinds = gs::integers::<u8>()
        .min_value(0)
        .max_value(if pool.is_empty() { 1 } else { 7 });
    match tc.draw(kinds) {
        0 | 1 => Command::Spawn(tc.draw(components())),
        2 => Command::Insert(tc.draw(handle_from(pool)), tc.draw(components())),
        3 => Command::InsertOne(tc.draw(handle_from(pool)), tc.draw(which), tc.draw(val())),
        4 => Command::RemoveAB(tc.draw(handle_from(pool))),
        5 => Command::RemoveCD(tc.draw(handle_from(pool))),
        6 => Command::RemoveOne(tc.draw(handle_from(pool)), tc.draw(which)),
        _ => Command::Despawn(tc.draw(handle_from(pool))),
    }
}

fn record(buffer: &mut CommandBuffer, c: Command) {
    match c {
        Command::Spawn(cs) => buffer.spawn(cs.builder().build()),
        Command::Insert(e, cs) => buffer.insert(e, cs.builder().build()),
        Command::InsertOne(e, which, v) => match which {
            0 => buffer.insert_one(e, A(v)),
            1 => buffer.insert_one(e, B(v)),
            2 => buffer.insert_one(e, C),
            _ => buffer.insert_one(e, D::new(v)),
        },
        Command::RemoveAB(e) => buffer.remove::<(A, B)>(e),
        Command::RemoveCD(e) => buffer.remove::<(C, D)>(e),
        Command::RemoveOne(e, which) => match which {
            0 => buffer.remove_one::<A>(e),
            1 => buffer.remove_one::<B>(e),
            2 => buffer.remove_one::<C>(e),
            _ => buffer.remove_one::<D>(e),
        },
        Command::Despawn(e) => buffer.despawn(e),
    }
}

/// `run_on` is documented to replay commands in order, ignoring failures, so
/// each command's eager equivalent discards its error.
fn apply_eagerly(world: &mut World, c: Command) {
    match c {
        Command::Spawn(cs) => {
            world.spawn(cs.builder().build());
        }
        Command::Insert(e, cs) => drop(world.insert(e, cs.builder().build())),
        Command::InsertOne(e, which, v) => match which {
            0 => drop(world.insert_one(e, A(v))),
            1 => drop(world.insert_one(e, B(v))),
            2 => drop(world.insert_one(e, C)),
            _ => drop(world.insert_one(e, D::new(v))),
        },
        Command::RemoveAB(e) => drop(world.remove::<(A, B)>(e)),
        Command::RemoveCD(e) => drop(world.remove::<(C, D)>(e)),
        Command::RemoveOne(e, which) => match which {
            0 => drop(world.remove_one::<A>(e)),
            1 => drop(world.remove_one::<B>(e)),
            2 => drop(world.remove_one::<C>(e)),
            _ => drop(world.remove_one::<D>(e)),
        },
        Command::Despawn(e) => drop(world.despawn(e)),
    }
}

/// How many `D` values a recorded sequence is holding inside the buffer.
fn pending_d(commands: &[Command]) -> i64 {
    commands
        .iter()
        .map(|c| match c {
            Command::Spawn(cs) | Command::Insert(_, cs) => cs.d.is_some() as i64,
            Command::InsertOne(_, 3, _) => 1,
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
    assert_d_balanced_at_start();
    let history = tc.draw(histories(0, MAX_ENTITIES));
    let (mut worlds, mut pool) = build_twins(&history, 2);

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
            record(&mut buffer, c);
        }

        // Recording must not touch either world, and every `D` handed to the
        // buffer must still be alive and unduplicated inside it.
        assert_eq!(
            fingerprint(&worlds[0]),
            fingerprint(&worlds[1]),
            "recording commands changed a world"
        );
        assert_eq!(
            d_live(),
            total_d(&worlds) + pending_d(&commands),
            "a buffered D was dropped early or leaked"
        );

        let outcome = tc.draw(gs::integers::<u8>().min_value(0).max_value(3));
        match outcome {
            0 | 1 => {
                buffer.run_on(&mut worlds[1]);
                for &c in &commands {
                    apply_eagerly(&mut worlds[0], c);
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
            2 => {
                buffer.clear();
                buffer.run_on(&mut worlds[1]);
                assert_eq!(
                    fingerprint(&worlds[0]),
                    fingerprint(&worlds[1]),
                    "commands survived clear()"
                );
            }
            // Dropping a non-empty buffer must release its stored components.
            _ => buffer = CommandBuffer::new(),
        }

        assert_eq!(
            d_live(),
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
