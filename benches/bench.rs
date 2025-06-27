#![allow(clippy::incompatible_msrv)] // Dev only

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use hecs::*;

#[derive(Clone)]
struct Position(f32);
#[derive(Clone)]
struct Velocity(f32);

fn spawn_tuple(c: &mut Criterion) {
    let mut world = World::new();
    c.bench_function("spawn_tuple", |b| {
        b.iter(|| {
            world.spawn((Position(0.0), Velocity(0.0)));
        })
    });
}

fn spawn_static(c: &mut Criterion) {
    #[derive(Bundle)]
    struct Bundle {
        pos: Position,
        vel: Velocity,
    }

    let mut world = World::new();
    c.bench_function("spawn_static", |b| {
        b.iter(|| {
            world.spawn(Bundle {
                pos: Position(0.0),
                vel: Velocity(0.0),
            });
        })
    });
}

fn spawn_batch(c: &mut Criterion) {
    #[derive(Bundle)]
    struct Bundle {
        pos: Position,
        vel: Velocity,
    }

    let mut world = World::new();
    c.bench_function("spawn_batch", |b| {
        b.iter(|| {
            world
                .spawn_batch((0..1_000).map(|_| Bundle {
                    pos: Position(0.0),
                    vel: Velocity(0.0),
                }))
                .for_each(|_| {});
            world.clear();
        })
    });
}

fn remove(c: &mut Criterion) {
    c.bench_function("remove", |b| {
        b.iter_batched(
            || {
                let mut world = World::new();
                let entities = world
                    .spawn_batch((0..1_000).map(|_| (Position(0.0), Velocity(0.0))))
                    .collect::<Vec<_>>();
                (world, entities)
            },
            |(mut world, entities)| {
                for e in entities {
                    world.remove_one::<Velocity>(e).unwrap();
                }
            },
            BatchSize::SmallInput,
        )
    });
}

fn insert(c: &mut Criterion) {
    c.bench_function("insert", |b| {
        b.iter_batched(
            || {
                let mut world = World::new();
                let entities = world
                    .spawn_batch((0..1_000).map(|_| (Position(0.0),)))
                    .collect::<Vec<_>>();
                (world, entities)
            },
            |(mut world, entities)| {
                for e in entities {
                    world.insert_one(e, Velocity(0.0)).unwrap();
                }
            },
            BatchSize::SmallInput,
        )
    });
}

fn insert_remove(c: &mut Criterion) {
    let mut world = World::new();
    let entities = world
        .spawn_batch((0..1_000).map(|_| (Position(0.0), Velocity(0.0))))
        .collect::<Vec<_>>();
    let mut entities = entities.iter().cycle();
    c.bench_function("insert_remove", |b| {
        b.iter(|| {
            let e = *entities.next().unwrap();
            world.remove_one::<Velocity>(e).unwrap();
            world.insert_one(e, true).unwrap();
            world.remove_one::<bool>(e).unwrap();
            world.insert_one(e, Velocity(0.0)).unwrap();
        })
    });
}

fn exchange(c: &mut Criterion) {
    let mut world = World::new();
    let entities = world
        .spawn_batch((0..1_000).map(|_| (Position(0.0), Velocity(0.0))))
        .collect::<Vec<_>>();
    let mut entities = entities.iter().cycle();
    c.bench_function("exchange", |b| {
        b.iter(|| {
            let e = *entities.next().unwrap();
            world.exchange_one::<Velocity, _>(e, true).unwrap();
            world.exchange_one::<bool, _>(e, Velocity(0.0)).unwrap();
        })
    });
}

fn iterate_100k(c: &mut Criterion) {
    let mut world = World::new();
    for i in 0..100_000 {
        world.spawn((Position(-(i as f32)), Velocity(i as f32)));
    }
    c.bench_function("iterate 100k", |b| {
        b.iter(|| {
            for (_, (pos, vel)) in &mut world.query::<(&mut Position, &Velocity)>() {
                pos.0 += vel.0;
            }
        })
    });
}

fn iterate_mut_100k(c: &mut Criterion) {
    let mut world = World::new();
    for i in 0..100_000 {
        world.spawn((Position(-(i as f32)), Velocity(i as f32)));
    }
    c.bench_function("iterate mut 100k", |b| {
        b.iter(|| {
            for (_, (pos, vel)) in world.query_mut::<(&mut Position, &Velocity)>() {
                pos.0 += vel.0;
            }
        })
    });
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

fn iterate_uncached_100_by_50(c: &mut Criterion) {
    let mut world = World::new();
    spawn_100_by_50(&mut world);
    c.bench_function("iterate_uncached_100_by_50", |b| {
        b.iter(|| {
            for (_, (pos, vel)) in world.query::<(&mut Position, &Velocity)>().iter() {
                pos.0 += vel.0;
            }
        })
    });
}

fn iterate_uncached_1_of_100_by_50(c: &mut Criterion) {
    let mut world = World::new();
    spawn_100_by_50(&mut world);
    c.bench_function("iterate_uncached_1_of_100_by_50", |b| {
        b.iter(|| {
            for (_, (pos, vel)) in world
                .query::<(&mut Position, &Velocity)>()
                .with::<&[(); 0]>()
                .iter()
            {
                pos.0 += vel.0;
            }
        })
    });
}

fn iterate_cached_100_by_50(c: &mut Criterion) {
    let mut world = World::new();
    spawn_100_by_50(&mut world);
    let mut query = PreparedQuery::<(&mut Position, &Velocity)>::default();
    let _ = query.query(&world).iter();
    c.bench_function("iterate_cached_100_by_50", |b| {
        b.iter(|| {
            for (_, (pos, vel)) in query.query(&world).iter() {
                pos.0 += vel.0;
            }
        })
    });
}

fn iterate_mut_uncached_100_by_50(c: &mut Criterion) {
    let mut world = World::new();
    spawn_100_by_50(&mut world);
    c.bench_function("iterate_mut_uncached_100_by_50", |b| {
        b.iter(|| {
            for (_, (pos, vel)) in world.query_mut::<(&mut Position, &Velocity)>() {
                pos.0 += vel.0;
            }
        })
    });
}

fn iterate_mut_cached_100_by_50(c: &mut Criterion) {
    let mut world = World::new();
    spawn_100_by_50(&mut world);
    let mut query = PreparedQuery::<(&mut Position, &Velocity)>::default();
    let _ = query.query_mut(&mut world);
    c.bench_function("iterate_mut_cached_100_by_50", |b| {
        b.iter(|| {
            for (_, (pos, vel)) in query.query_mut(&mut world) {
                pos.0 += vel.0;
            }
        })
    });
}

fn build(c: &mut Criterion) {
    let mut world = World::new();
    let mut builder = EntityBuilder::new();
    c.bench_function("build", |b| {
        b.iter(|| {
            builder.add(Position(0.0)).add(Velocity(0.0));
            world.spawn(builder.build());
        })
    });
}

fn build_cloneable(c: &mut Criterion) {
    let mut world = World::new();
    let mut builder = EntityBuilderClone::new();
    builder.add(Position(0.0)).add(Velocity(0.0));
    let bundle = builder.build();
    c.bench_function("build_cloneable", |b| {
        b.iter(|| {
            world.spawn(&bundle);
        })
    });
}

fn access_view(c: &mut Criterion) {
    let mut world = World::new();
    let _enta = world.spawn((Position(0.0), Velocity(0.0)));
    let _entb = world.spawn((true, 12));
    let entc = world.spawn((Position(3.0),));
    let _entd = world.spawn((13, true, 4.0));
    let mut query = PreparedQuery::<&Position>::new();
    let mut query = query.query(&world);
    let view = query.view();
    c.bench_function("access_view", |b| {
        b.iter(|| {
            let _comp = black_box(view.get(entc).unwrap());
        })
    });
}

fn spawn_buffered(c: &mut Criterion) {
    let mut world = World::new();
    let mut buffer = CommandBuffer::new();
    let ent = world.reserve_entity();
    c.bench_function("spawn_buffered", |b| {
        b.iter(|| {
            buffer.insert(ent, (Position(0.0), Velocity(0.0)));
            buffer.run_on(&mut world);
        })
    });
}

criterion_group!(
    benches,
    spawn_tuple,
    spawn_static,
    spawn_batch,
    remove,
    insert,
    insert_remove,
    exchange,
    iterate_100k,
    iterate_mut_100k,
    iterate_uncached_100_by_50,
    iterate_uncached_1_of_100_by_50,
    iterate_cached_100_by_50,
    iterate_mut_uncached_100_by_50,
    iterate_mut_cached_100_by_50,
    build,
    build_cloneable,
    access_view,
    spawn_buffered,
);
criterion_main!(benches);
