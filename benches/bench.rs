// Copyright 2019 Google LLC
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use bencher::{benchmark_group, benchmark_main, Bencher};
use hecs::*;

#[derive(Clone)]
struct Position(f32);
#[derive(Clone)]
struct Velocity(f32);

fn spawn_tuple(b: &mut Bencher) {
    let mut world = World::new();
    b.iter(|| {
        world.spawn((Position(0.0), Velocity(0.0)));
    });
}

fn spawn_static(b: &mut Bencher) {
    #[derive(Bundle)]
    struct Bundle {
        pos: Position,
        vel: Velocity,
    }

    let mut world = World::new();
    b.iter(|| {
        world.spawn(Bundle {
            pos: Position(0.0),
            vel: Velocity(0.0),
        });
    });
}

fn spawn_batch(b: &mut Bencher) {
    #[derive(Bundle)]
    struct Bundle {
        pos: Position,
        vel: Velocity,
    }

    let mut world = World::new();
    b.iter(|| {
        world
            .spawn_batch((0..1_000).map(|_| Bundle {
                pos: Position(0.0),
                vel: Velocity(0.0),
            }))
            .for_each(|_| {});
        world.clear();
    });
}

fn remove(b: &mut Bencher) {
    let mut world = World::new();
    b.iter(|| {
        // This really shouldn't be counted as part of the benchmark, but bencher doesn't seem to
        // support that.
        let entities = world
            .spawn_batch((0..1_000).map(|_| (Position(0.0), Velocity(0.0))))
            .collect::<Vec<_>>();
        for e in entities {
            world.remove_one::<Velocity>(e).unwrap();
        }
        world.clear();
    });
}

fn insert(b: &mut Bencher) {
    let mut world = World::new();
    b.iter(|| {
        // This really shouldn't be counted as part of the benchmark, but bencher doesn't seem to
        // support that.
        let entities = world
            .spawn_batch((0..1_000).map(|_| (Position(0.0),)))
            .collect::<Vec<_>>();
        for e in entities {
            world.insert_one(e, Velocity(0.0)).unwrap();
        }
        world.clear();
    });
}

fn insert_remove(b: &mut Bencher) {
    let mut world = World::new();
    let entities = world
        .spawn_batch((0..1_000).map(|_| (Position(0.0), Velocity(0.0))))
        .collect::<Vec<_>>();
    let mut entities = entities.iter().cycle();
    b.iter(|| {
        let e = *entities.next().unwrap();
        world.remove_one::<Velocity>(e).unwrap();
        world.insert_one(e, true).unwrap();
        world.remove_one::<bool>(e).unwrap();
        world.insert_one(e, Velocity(0.0)).unwrap();
    });
}

fn iterate_100k(b: &mut Bencher) {
    let mut world = World::new();
    for i in 0..100_000 {
        world.spawn((Position(-(i as f32)), Velocity(i as f32)));
    }
    b.iter(|| {
        for (_, (pos, vel)) in &mut world.query::<(&mut Position, &Velocity)>() {
            pos.0 += vel.0;
        }
    })
}

fn iterate_mut_100k(b: &mut Bencher) {
    let mut world = World::new();
    for i in 0..100_000 {
        world.spawn((Position(-(i as f32)), Velocity(i as f32)));
    }
    b.iter(|| {
        for (_, (pos, vel)) in world.query_mut::<(&mut Position, &Velocity)>() {
            pos.0 += vel.0;
        }
    })
}

fn spawn_100_by_50(world: &mut World) {
    fn spawn_two<const N: usize>(world: &mut World, i: i32) {
        world.spawn((Position(-(i as f32)), Velocity(i as f32), [(); N]));
        world.spawn((Position(-(i as f32)), [(); N]));
    }

    for i in 0..2 {
        spawn_two::<0>(world, i);
        spawn_two::<1>(world, i);
        spawn_two::<2>(world, i);
        spawn_two::<3>(world, i);
        spawn_two::<4>(world, i);
        spawn_two::<5>(world, i);
        spawn_two::<6>(world, i);
        spawn_two::<7>(world, i);
        spawn_two::<8>(world, i);
        spawn_two::<9>(world, i);
        spawn_two::<10>(world, i);
        spawn_two::<11>(world, i);
        spawn_two::<12>(world, i);
        spawn_two::<13>(world, i);
        spawn_two::<14>(world, i);
        spawn_two::<15>(world, i);
        spawn_two::<16>(world, i);
        spawn_two::<17>(world, i);
        spawn_two::<18>(world, i);
        spawn_two::<19>(world, i);
        spawn_two::<20>(world, i);
        spawn_two::<21>(world, i);
        spawn_two::<22>(world, i);
        spawn_two::<23>(world, i);
        spawn_two::<24>(world, i);
    }
}

fn iterate_uncached_100_by_50(b: &mut Bencher) {
    let mut world = World::new();
    spawn_100_by_50(&mut world);
    b.iter(|| {
        for (_, (pos, vel)) in world.query::<(&mut Position, &Velocity)>().iter() {
            pos.0 += vel.0;
        }
    })
}

fn iterate_cached_100_by_50(b: &mut Bencher) {
    let mut world = World::new();
    spawn_100_by_50(&mut world);
    let mut query = PreparedQuery::<(&mut Position, &Velocity)>::default();
    let _ = query.query(&world).iter();
    b.iter(|| {
        for (_, (pos, vel)) in query.query(&world).iter() {
            pos.0 += vel.0;
        }
    })
}

fn iterate_mut_uncached_100_by_50(b: &mut Bencher) {
    let mut world = World::new();
    spawn_100_by_50(&mut world);
    b.iter(|| {
        for (_, (pos, vel)) in world.query_mut::<(&mut Position, &Velocity)>() {
            pos.0 += vel.0;
        }
    })
}

fn iterate_mut_cached_100_by_50(b: &mut Bencher) {
    let mut world = World::new();
    spawn_100_by_50(&mut world);
    let mut query = PreparedQuery::<(&mut Position, &Velocity)>::default();
    let _ = query.query_mut(&mut world);
    b.iter(|| {
        for (_, (pos, vel)) in query.query_mut(&mut world) {
            pos.0 += vel.0;
        }
    })
}

fn build(b: &mut Bencher) {
    let mut world = World::new();
    let mut builder = EntityBuilder::new();
    b.iter(|| {
        builder.add(Position(0.0)).add(Velocity(0.0));
        world.spawn(builder.build());
    });
}

fn build_cloneable(b: &mut Bencher) {
    let mut world = World::new();
    let mut builder = EntityBuilderClone::new();
    builder.add(Position(0.0)).add(Velocity(0.0));
    let bundle = builder.build();
    b.iter(|| {
        world.spawn(&bundle);
    });
}

fn access_column(b: &mut Bencher) {
    let mut world = World::new();
    let _enta = world.spawn((Position(0.0), Velocity(0.0)));
    let _entb = world.spawn((true, 12));
    let entc = world.spawn((Position(3.0),));
    let _entd = world.spawn((13, true, 4.0));
    let column = world.column::<Position>();
    b.iter(|| {
        let _comp = bencher::black_box(column.get(entc).unwrap());
    });
}

fn access_view(b: &mut Bencher) {
    let mut world = World::new();
    let _enta = world.spawn((Position(0.0), Velocity(0.0)));
    let _entb = world.spawn((true, 12));
    let entc = world.spawn((Position(3.0),));
    let _entd = world.spawn((13, true, 4.0));
    let mut query = PreparedQuery::<&Position>::new();
    let mut query = query.query(&world);
    let view = query.view();
    b.iter(|| {
        let _comp = bencher::black_box(view.get(entc).unwrap());
    });
}

fn spawn_buffered(b: &mut Bencher) {
    let mut world = World::new();
    let mut buffer = CommandBuffer::new();
    let ent = world.reserve_entity();
    b.iter(|| {
        buffer.insert(ent, (Position(0.0), Velocity(0.0)));
        buffer.run_on(&mut world);
    });
}

benchmark_group!(
    benches,
    spawn_tuple,
    spawn_static,
    spawn_batch,
    remove,
    insert,
    insert_remove,
    iterate_100k,
    iterate_mut_100k,
    iterate_uncached_100_by_50,
    iterate_cached_100_by_50,
    iterate_mut_uncached_100_by_50,
    iterate_mut_cached_100_by_50,
    build,
    build_cloneable,
    access_column,
    access_view,
    spawn_buffered,
);
benchmark_main!(benches);
