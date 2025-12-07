use hecs::*;
use rand::Rng;
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
struct Health(i32);

#[derive(Debug)]
struct Speed(i32);

#[derive(Debug)]
struct Damage(i32);

#[derive(Debug)]
struct KillCount(i32);

fn manhattan_dist(x0: i32, x1: i32, y0: i32, y1: i32) -> i32 {
    let dx = (x0 - x1).abs();
    let dy = (y0 - y1).abs();
    dx + dy
}

fn batch_spawn_entities(world: &mut World, n: usize) {
    let mut rng = rand::rng();

    let to_spawn = (0..n).map(|_| {
        let pos = Position {
            x: rng.random_range(-10..10),
            y: rng.random_range(-10..10),
        };
        let s = Speed(rng.random_range(1..5));
        let hp = Health(rng.random_range(30..50));
        let dmg = Damage(rng.random_range(1..10));
        let kc = KillCount(0);

        (pos, s, hp, dmg, kc)
    });

    world.spawn_batch(to_spawn);
    // We could instead call `world.spawn((pos, s, hp, dmg, kc))` for each entity, but `spawn_batch`
    // is faster.
}

fn system_integrate_motion(
    world: &mut World,
    query: &mut PreparedQuery<(Entity, &mut Position, &Speed)>,
) {
    let mut rng = rand::rng();

    for (id, pos, s) in query.query_mut(world) {
        let change = (rng.random_range(-s.0..s.0), rng.random_range(-s.0..s.0));
        pos.x += change.0;
        pos.y += change.1;
        println!("Unit {id:?} moved to {pos:?}");
    }
}

// In this system entities find the closest entity and fire at them
fn system_fire_at_closest(world: &mut World) {
    for (id0, pos0, dmg0, kc0) in
        &mut world.query::<With<(Entity, &Position, &Damage, &mut KillCount), &Health>>()
    {
        // Find closest:
        // Nested queries are O(n^2) and you usually want to avoid that by using some sort of
        // spatial index like a quadtree or more general BVH, which we don't bother with here since
        // it's out of scope for the example.
        let closest = world
            .query::<With<(Entity, &Position), &Health>>()
            .iter()
            .filter(|(id1, _)| *id1 != id0)
            .min_by_key(|(_, pos1)| manhattan_dist(pos0.x, pos1.x, pos0.y, pos1.y))
            .map(|(entity, _pos)| entity);

        let closest = match closest {
            Some(entity) => entity,
            None => {
                println!("{id0:?} is the last survivor!");
                return;
            }
        };

        // Deal damage:
        /*
                // Get target unit hp like this:
                let mut hp1 = world.query_one::<&mut Health>(closest_id.unwrap()).unwrap();
                let hp1 = hp1.get().unwrap();
        */

        // Or like this:
        let mut hp1 = world.get::<&mut Health>(closest).unwrap();

        // Is target unit still alive?
        if hp1.0 > 0 {
            // apply damage
            hp1.0 -= dmg0.0;
            println!(
                "Unit {closest:?} was damaged by {id0:?} for {:?} HP",
                dmg0.0
            );
            if hp1.0 <= 0 {
                // if this killed it, increase own killcount
                kc0.0 += 1;
                println!("Unit {closest:?} was killed by unit {id0:?}!");
            }
        }
    }
}

fn system_remove_dead(world: &mut World) {
    // Here we query entities with 0 or less hp and despawn them
    let mut to_remove: Vec<Entity> = Vec::new();
    for (id, hp) in &mut world.query::<(Entity, &Health)>() {
        if hp.0 <= 0 {
            to_remove.push(id);
        }
    }

    for entity in to_remove {
        world.despawn(entity).unwrap();
    }
}

fn print_world_state(world: &mut World) {
    println!("\nEntity stats:");
    for (id, hp, pos, dmg, kc) in
        &mut world.query::<(Entity, &Health, &Position, &Damage, &KillCount)>()
    {
        println!("ID: {id:?}, {hp:?}, {dmg:?}, {pos:?}, {kc:?}");
    }
}

fn main() {
    let mut world = World::new();

    batch_spawn_entities(&mut world, 5);

    let mut motion_query = PreparedQuery::<(Entity, &mut Position, &Speed)>::default();

    loop {
        println!("\n'Enter' to continue simulation, '?' for entity list, 'q' to quit");

        let mut input = String::new();

        io::stdin().read_line(&mut input).unwrap();

        match input.trim() {
            "" => {
                // Run all simulation systems:
                system_integrate_motion(&mut world, &mut motion_query);
                system_fire_at_closest(&mut world);
                system_remove_dead(&mut world);
            }
            "q" => break,
            "?" => {
                print_world_state(&mut world);
            }
            _ => {}
        }
    }
}
