use hecs::*;
use rand::{thread_rng, Rng};
use std::io;

/*
 Simple simulation
 Spawn multiple entities. They have health, damage, position and other components.
 On every tick every entity/unit:
    1. Moves in random direction.
     2. Finds closest entity to itself.
     3. Fires at it and applies damage.
     4. Gets damaged by other entities firing at them.
     5. If health <= 0, the unit dies.
State of the simulation is displayed in the sconsole through println! functions.
*/

#[derive(Debug)]
struct Position {
    x: i32,
    y: i32,
}

#[derive(Debug)]
struct Health {
    hp: i32,
}

#[derive(Debug)]
struct Speed {
    speed: i32,
}

#[derive(Debug)]
struct Damage {
    dmg: i32,
}

#[derive(Debug)]
struct KillCount {
    count: i32,
}

fn manhattan_dist(x0: i32, x1: i32, y0: i32, y1: i32) -> i32 {
    let dx = (x0 - x1).abs();
    let dy = (y0 - y1).abs();
    return dx + dy;
}

// Spawns entities one by one. It's preferable to use spawn_batch instead, which has better performance for spawning multiple entities.
fn _spawn_enitites(world: &mut World, n: usize) {
    let mut rng = thread_rng();

    for _ in 0..n {
        let pos = Position {
            x: rng.gen_range(-10, 10),
            y: rng.gen_range(-10, 10),
        };
        let s = Speed {
            speed: rng.gen_range(1, 5),
        };
        let hp = Health {
            hp: rng.gen_range(30, 50),
        };
        let dmg = Damage {
            dmg: rng.gen_range(1, 10),
        };
        let kc = KillCount { count: 0 };

        world.spawn((pos, s, hp, dmg, kc));
    }
}

fn batch_spawn_entities(world: &mut World, n: usize) {
    let mut rng = thread_rng();

    let to_spawn = (0..n).map(|_| {
        let pos = Position {
            x: rng.gen_range(-10, 10),
            y: rng.gen_range(-10, 10),
        };
        let s = Speed {
            speed: rng.gen_range(1, 5),
        };
        let hp = Health {
            hp: rng.gen_range(30, 50),
        };
        let dmg = Damage {
            dmg: rng.gen_range(1, 10),
        };
        let kc = KillCount { count: 0 };

        (pos, s, hp, dmg, kc)
    });

    world.spawn_batch(to_spawn);
}

fn system_integrate_motion(world: &mut World) {
    let mut rng = thread_rng();

    for (id, (pos, s)) in &mut world.query::<(&mut Position, &Speed)>() {
        let change = (
            rng.gen_range(-s.speed, s.speed),
            rng.gen_range(-s.speed, s.speed),
        );
        pos.x += change.0;
        pos.y += change.1;
        println!("Unit {:?} moved to {:?}", id, pos);
    }
}

// In this system entities find the closest entity and fire at them
fn system_fire_at_closest(world: &mut World) {
    for (id0, (pos0, dmg0, kc0)) in
        &mut world.query::<With<Health, (&Position, &Damage, &mut KillCount)>>()
    {
        let mut min_dist: Option<i32> = None;
        let mut closest_id: Option<Entity> = None;

        //Find closest:
        //Comment about implementation: Nested queries are O(n^2) and you usually want to avoid that by using some sort of spatial index like a quadtree or more general BVH, which we don't bother with here since it's out of scope for the example.
        'child_loop: for (id1, pos1) in &mut world.query::<With<Health, &Position>>() {
            if id0 == id1 {
                continue 'child_loop;
            }

            let dist = manhattan_dist(pos0.x, pos1.x, pos0.y, pos1.y);

            match min_dist {
                None => {
                    min_dist = Some(dist);
                }
                Some(mut _dist0) => {
                    _dist0 = _dist0.min(dist);
                }
            }

            if Some(dist) == min_dist {
                closest_id = Some(id1);
            }
        }

        if !closest_id.is_some() {
            println!("{:?} is the last survivor!", id0);
            return;
        }

        // Deal damage:
        /*
                //Get target unit hp like this:
                let mut hp1 = world.query_one::<&mut Health>(closest_id.unwrap()).unwrap();
                let hp1 = hp1.get().unwrap();
        */

        //Or like this:
        let mut hp1 = world.get_mut::<Health>(closest_id.unwrap()).unwrap();

        // Is target unit still alive?
        if hp1.hp > 0 {
            // apply damage
            hp1.hp = hp1.hp - dmg0.dmg;
            println!(
                "Unit {:?} was damaged by {:?} for {:?} HP",
                closest_id.unwrap(),
                id0,
                dmg0.dmg
            );
            if hp1.hp <= 0 {
                // if this killed it, increase own killcount
                kc0.count += 1;
                println!(
                    "Unit {:?} was killed by unit {:?}!",
                    closest_id.unwrap(),
                    id0
                );
            }
        }
    }
}

fn system_remove_dead(world: &mut World) {
    // Here we query entities with 0 or less hp and despawn them
    let mut to_remove: Vec<Entity> = Vec::new();
    for (id, hp) in &mut world.query::<&Health>() {
        if hp.hp <= 0 {
            to_remove.push(id);
        }
    }

    for i in 0..to_remove.len() {
        world.despawn(to_remove[i]).unwrap();
    }
}

fn print_world_state(world: &mut World) {
    println!("\nEntity stats:");
    for (id, (hp, pos, dmg, kc)) in &mut world.query::<(&Health, &Position, &Damage, &KillCount)>()
    {
        println!("ID: {:?}, {:?}, {:?}, {:?}, {:?}", id, hp, dmg, pos, kc);
    }
}

fn main() {
    let mut world = World::new();

    // _spawn_enitites(&mut world, 5);
    batch_spawn_entities(&mut world, 5);

    'running: loop {
        println!("\n'Enter' to continue simulation, '?' for enity list, 'q' to quit");

        let mut input = String::new();

        io::stdin().read_line(&mut input).unwrap();

        match input.trim() {
            "" => {
                // Run all simulation systems:
                system_integrate_motion(&mut world);
                system_fire_at_closest(&mut world);
                system_remove_dead(&mut world);
            }
            "q" => break 'running,
            "?" => {
                print_world_state(&mut world);
            }
            _ => {}
        }
    }
}
