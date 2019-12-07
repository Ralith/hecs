use bencher::{benchmark_group, benchmark_main, Bencher};
use hecs::*;

struct Position(f32);
struct Velocity(f32);

fn spawn(b: &mut Bencher) {
    let mut world = World::new();
    b.iter(|| {
        world.spawn((Position(0.0), Velocity(0.0)));
    });
}

fn iterate_100k(b: &mut Bencher) {
    let mut world = World::new();
    for i in 0..100_000 {
        world.spawn((Position(-(i as f32)), Velocity(i as f32)));
    }
    b.iter(|| {
        for (_, (pos, vel)) in world.query::<(&mut Position, &Velocity)>() {
            pos.0 += vel.0;
        }
    })
}

benchmark_group!(benches, spawn, iterate_100k);
benchmark_main!(benches);
