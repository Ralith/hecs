// Copyright 2019 Google LLC
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use bencher::{benchmark_group, benchmark_main, Bencher};
use hecs::*;

struct Position(f32);
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

fn spawn_100k_by_50(world: &mut World) {
    for i in 0..2_000 {
        world.spawn((Position(-(i as f32)), Velocity(i as f32), [(); 0]));
        world.spawn((Position(-(i as f32)), [(); 0]));
        world.spawn((Position(-(i as f32)), Velocity(i as f32), [(); 1]));
        world.spawn((Position(-(i as f32)), [(); 1]));
        world.spawn((Position(-(i as f32)), Velocity(i as f32), [(); 2]));
        world.spawn((Position(-(i as f32)), [(); 2]));
        world.spawn((Position(-(i as f32)), Velocity(i as f32), [(); 3]));
        world.spawn((Position(-(i as f32)), [(); 3]));
        world.spawn((Position(-(i as f32)), Velocity(i as f32), [(); 4]));
        world.spawn((Position(-(i as f32)), [(); 4]));
        world.spawn((Position(-(i as f32)), Velocity(i as f32), [(); 5]));
        world.spawn((Position(-(i as f32)), [(); 5]));
        world.spawn((Position(-(i as f32)), Velocity(i as f32), [(); 6]));
        world.spawn((Position(-(i as f32)), [(); 6]));
        world.spawn((Position(-(i as f32)), Velocity(i as f32), [(); 7]));
        world.spawn((Position(-(i as f32)), [(); 7]));
        world.spawn((Position(-(i as f32)), Velocity(i as f32), [(); 8]));
        world.spawn((Position(-(i as f32)), [(); 8]));
        world.spawn((Position(-(i as f32)), Velocity(i as f32), [(); 9]));
        world.spawn((Position(-(i as f32)), [(); 9]));
        world.spawn((Position(-(i as f32)), Velocity(i as f32), [(); 10]));
        world.spawn((Position(-(i as f32)), [(); 10]));
        world.spawn((Position(-(i as f32)), Velocity(i as f32), [(); 11]));
        world.spawn((Position(-(i as f32)), [(); 11]));
        world.spawn((Position(-(i as f32)), Velocity(i as f32), [(); 12]));
        world.spawn((Position(-(i as f32)), [(); 12]));
        world.spawn((Position(-(i as f32)), Velocity(i as f32), [(); 13]));
        world.spawn((Position(-(i as f32)), [(); 13]));
        world.spawn((Position(-(i as f32)), Velocity(i as f32), [(); 14]));
        world.spawn((Position(-(i as f32)), [(); 14]));
        world.spawn((Position(-(i as f32)), Velocity(i as f32), [(); 15]));
        world.spawn((Position(-(i as f32)), [(); 15]));
        world.spawn((Position(-(i as f32)), Velocity(i as f32), [(); 16]));
        world.spawn((Position(-(i as f32)), [(); 16]));
        world.spawn((Position(-(i as f32)), Velocity(i as f32), [(); 17]));
        world.spawn((Position(-(i as f32)), [(); 17]));
        world.spawn((Position(-(i as f32)), Velocity(i as f32), [(); 18]));
        world.spawn((Position(-(i as f32)), [(); 18]));
        world.spawn((Position(-(i as f32)), Velocity(i as f32), [(); 19]));
        world.spawn((Position(-(i as f32)), [(); 19]));
        world.spawn((Position(-(i as f32)), Velocity(i as f32), [(); 20]));
        world.spawn((Position(-(i as f32)), [(); 20]));
        world.spawn((Position(-(i as f32)), Velocity(i as f32), [(); 21]));
        world.spawn((Position(-(i as f32)), [(); 21]));
        world.spawn((Position(-(i as f32)), Velocity(i as f32), [(); 22]));
        world.spawn((Position(-(i as f32)), [(); 22]));
        world.spawn((Position(-(i as f32)), Velocity(i as f32), [(); 23]));
        world.spawn((Position(-(i as f32)), [(); 23]));
        world.spawn((Position(-(i as f32)), Velocity(i as f32), [(); 24]));
        world.spawn((Position(-(i as f32)), [(); 24]));
    }
}

fn iterate_uncached_100k_by_50(b: &mut Bencher) {
    let mut world = World::new();
    spawn_100k_by_50(&mut world);
    b.iter(|| {
        for (_, (pos, vel)) in &mut world.query::<(&mut Position, &Velocity)>() {
            pos.0 += vel.0;
        }
    })
}

fn iterate_cached_100k_by_50(b: &mut Bencher) {
    let mut world = World::new();
    let mut cache = world.query_cache();
    spawn_100k_by_50(&mut world);
    b.iter(|| {
        let mut query = world.query::<(&mut Position, &Velocity)>();
        for (_, (pos, vel)) in query.iter_cached(&mut cache) {
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

benchmark_group!(
    benches,
    spawn_tuple,
    spawn_static,
    spawn_batch,
    remove,
    insert,
    iterate_100k,
    iterate_mut_100k,
    iterate_uncached_100k_by_50,
    iterate_cached_100k_by_50,
    build
);
benchmark_main!(benches);
