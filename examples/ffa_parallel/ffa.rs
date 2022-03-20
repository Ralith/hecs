use hecs::*;
use rand::{thread_rng, Rng};
use std::io;

/*
 Modified example of the ffa_simulation.rs example for parallel execution.

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
    let mut rng = thread_rng();

    const WORLD_SIZE: i32 = 1000;
    let to_spawn = (0..n).map(|_| {
        let pos = Position {
            x: rng.gen_range(-WORLD_SIZE..WORLD_SIZE),
            y: rng.gen_range(-WORLD_SIZE..WORLD_SIZE),
        };
        let s = Speed(rng.gen_range(1..5));
        let hp = Health(rng.gen_range(30..50));
        let dmg = Damage(rng.gen_range(1..10));
        let kc = KillCount(0);

        (pos, s, hp, dmg, kc)
    });

    world.spawn_batch(to_spawn);
    // We could instead call `world.spawn((pos, s, hp, dmg, kc))` for each entity, but `spawn_batch`
    // is faster.
}

fn integrate_motion(id: Entity, (pos, s): (&mut Position, &Speed)) {
    let mut rng = thread_rng();
    let change = (rng.gen_range(-s.0..s.0), rng.gen_range(-s.0..s.0));
    pos.x += change.0;
    pos.y += change.1;
    println!("Unit {:?} moved to {:?}", id, pos);
}

fn fire_at_closest(
    world: &World,
    id0: Entity,
    pos0: &Position,
    tx: &std::sync::mpsc::Sender<(Entity, Entity)>,
) {
    // Find closest:
    // Nested queries are O(n^2) and you usually want to avoid that by using some sort of
    // spatial index like a quadtree or more general BVH, which we don't bother with here since
    // it's out of scope for the example.
    let closest = world
        .query::<With<Health, &Position>>()
        .iter()
        .filter(|(id1, _)| *id1 != id0)
        .min_by_key(|(_, pos1)| manhattan_dist(pos0.x, pos1.x, pos0.y, pos1.y))
        .map(|(entity, _pos)| entity);

    // Since the application of damage is an inherently single threaded piece
    // of work, it is split into it's own system.  This simply pushes the id
    // of the targets into a mpsc queue so the single threaded integration can
    // pull the items off the queue.
    match closest {
        Some(entity) => {
            let _ = tx.send((id0, entity));
        }
        None => {
            println!("{:?} is the last survivor!", id0);
        }
    };
}

// Not everything in a parallel execution graph actually can run in parallel.
// This system is a case where running in parallel would have the potential to
// break the borrow rules when more than one entity tries to damage a single
// target.  So, it remains single threaded.
fn system_apply_damage(world: &World, rx: &std::sync::mpsc::Receiver<(Entity, Entity)>) {
    while let Ok((source, target)) = rx.try_recv() {
        // Get the damage being done from the source entity.
        let dmg0 = world.get::<Damage>(source).unwrap();
        // Get the entity receiving the damage.
        let mut hp1 = world.get_mut::<Health>(target).unwrap();

        // Is target unit still alive?
        if hp1.0 > 0 {
            // apply damage
            hp1.0 -= dmg0.0;
            println!(
                "Unit {:?} was damaged by {:?} for {:?} HP",
                target, source, dmg0.0
            );
            if hp1.0 <= 0 {
                // if this killed it, increase own killcount
                let mut kc0 = world.get_mut::<KillCount>(source).unwrap();
                kc0.0 += 1;
                println!("Unit {:?} was killed by unit {:?}!", target, source);
            }
        }
    }
}

fn system_remove_dead(world: &World) -> hecs::CommandBuffer {
    // Cheesy but effective.  The better solution, if you delete a lot of
    // entities regularly, is a command buffer per thread.  At points in
    // the execution graph you would then run those buffers on a mutable
    // world in a single threaded function.
    let commands = std::sync::Mutex::new(hecs::CommandBuffer::new());

    // Make a list of all dead enemies.  NOTE: This list could be done safely
    // within the `apply_damage` function to avoid the iteration here.  But,
    // the purpose is to show parallel execution, not so much about
    // specific optimization.
    let iter = ParallelIter::new();
    rayon::scope({
        // Make a reference to the commands to be given to each thread.
        let commands = &commands;
        move |scope| {
            for _ in 0..4 {
                scope.spawn({
                    let iter = iter.clone();
                    move |_| unsafe {
                        world.parallel_query::<&Health>(iter, 10, &|id, hp| {
                            if hp.0 <= 0 {
                                let mut commands = commands.lock().unwrap();
                                commands.despawn(id);
                            }
                        })
                    }
                })
            }
        }
    });

    commands.into_inner().unwrap()
}

fn print_world_state(world: &mut World) {
    println!("\nEntity stats:");
    for (id, (hp, pos, dmg, kc)) in &mut world.query::<(&Health, &Position, &Damage, &KillCount)>()
    {
        println!("ID: {:?}, {:?}, {:?}, {:?}, {:?}", id, hp, dmg, pos, kc);
    }
}

pub fn main() {
    let mut world = World::new();

    const ENTITY_COUNT: usize = 1000;
    batch_spawn_entities(&mut world, ENTITY_COUNT);

    // Create a queue for damage processing.  The message type is
    // a tuple of source entity and target entity.
    let (tx, rx) = std::sync::mpsc::channel::<(Entity, Entity)>();

    loop {
        println!("\n'Enter' to continue simulation, '?' for entity list, 'q' to quit");

        let mut input = String::new();

        io::stdin().read_line(&mut input).unwrap();

        match input.trim() {
            "" => {
                // Run all simulation systems.
                // Unfortunately the way Rayon works behind the scenes is via a work stealing
                // job system.  A better execution model for pre-defined graphs is unfortunately
                // beyond the scope here so, we'll fake it up a bit to make it "look" like a
                // graph executor.
                rayon::scope(|scope| {
                    // Execute in four jobs.
                    let iter = ParallelIter::new();

                    for _ in 0..4 {
                        scope.spawn({
                            let iter = iter.clone();
                            |_| unsafe {
                                world.parallel_query::<(&mut Position, &Speed)>(
                                    iter,
                                    100,
                                    &integrate_motion,
                                );
                            }
                        });
                    }
                });
                rayon::scope({
                    // Clone the sender into scope since it is not Sync.
                    let tx = tx.clone();
                    // Get a reference to the world rather than a copy.
                    let world = &world;
                    move |scope| {
                        // Execute in four jobs.
                        let iter = ParallelIter::new();

                        for _ in 0..4 {
                            scope.spawn({
                                // Clone the data per spawn.
                                let iter = iter.clone();
                                // Clone the sender into the new task.
                                let tx = tx.clone();

                                move |_| unsafe {
                                    world.parallel_query::<With<KillCount, &Position>>(
                                        iter,
                                        100,
                                        {
                                            &move |id0, pos0| {
                                                fire_at_closest(&world, id0, pos0, &tx);
                                            }
                                        },
                                    );
                                }
                            });
                        }
                    }
                });

                // This is a return to single threaded for this system.
                system_apply_damage(&world, &rx);

                // Fake up a thread local style solution.
                // Each "thread" would own a command buffer where it would
                // accumulate changes to the world for later application.
                // In this case, we just use the job index to access the
                // specific buffer via a little unsafe buggery.
                let mut commands = Vec::<CommandBuffer>::new();
                (0..4)
                    .into_iter()
                    .for_each(|_| commands.push(CommandBuffer::new()));

                rayon::scope({
                    let world = &world;
                    let commands = &commands;
                    move |scope| {
                        // Removal of dead units can not directly modify the world in
                        // parallel so it creates a `CommandBuffer` to be run later.
                        for index in 0..4 {
                            scope.spawn({
                                // Evil jiggery/pokery..  Just done this way in the example,
                                // a full solution would make these command buffers available
                                // per thread.
                                #[allow(mutable_transmutes)]
                                let commands: &mut CommandBuffer =
                                    unsafe { std::mem::transmute(&commands[index]) };
                                move |_| {
                                    *commands = system_remove_dead(&world);
                                }
                            });
                        }
                    }
                });

                // Execute the each command buffer on the world.
                for command in &mut commands {
                    command.run_on(&mut world);
                }
            }
            "q" => break,
            "?" => {
                print_world_state(&mut world);
            }
            _ => {}
        }
    }
}

/*
In order to run an ECS in a graph executor, a topological sort is executed on
the systems based on the data which they access and if that access is mutable
or immutable.  The goal is to end up with a generic list of execution groups
which can be instanced once a frame.
 */

// A trait representing a system which can be executed on the an immutable
// world.
trait SystemCall {
    fn call(&self, world: &World);
}

// An entry in the schedule representing either an iteration or a command buffer
// flush.  Individually these are synchronized so there is no overlapping execution.
enum Entry {
    Immutable(Vec<Box<dyn SystemCall>>),
    Flush,
}

// A compiled schedule.
struct Schedule(Vec<Entry>);

impl Schedule {
    fn create() -> ScheduleBuilder {
        ScheduleBuilder::new()
    }

    fn new() -> Self {
        Self(Vec::new())
    }

    pub fn execute(&self, world: &mut World) {
        // Using Rayon is not exactly the best example but I was unable to find
        // a threaded graph executor to use for demo purposes.  The issue with
        // this implementation is that it will end up dynamically building jobs
        // rather than issuing the entire graph at one time.  I'll fake it up
        // but there are much better solutions for games.
        for entry in &self.0 {
            // Each entry represents a layer in the DAG produced by topological
            // sorting of the system data access.  The layers can not overlap
            // but everything within the layer can execute simultaneously.
        }
    }
}

use std::collections::BTreeSet;
struct ComponentSet(BTreeSet<(std::any::TypeId, bool)>);

impl ComponentSet {
    pub fn new() -> Self {
        Self(BTreeSet::new())
    }

    pub fn union(&self, rhs: &ComponentSet) -> Self {
        Self(self.0.union(&rhs.0).cloned().collect())
    }

    pub fn difference(&self, rhs: &ComponentSet) -> Self {
        Self(self.0.difference(&rhs.0).cloned().collect())
    }
}

struct ScheduleBuilder {
    dag: Vec<(ComponentSet, Vec<Entry>)>,
}

impl ScheduleBuilder {
    pub fn new() -> Self {
        Self { dag: Vec::new() }
    }

    // This reuses the hecs query type rather than introducing a bunch of
    // additional wrapper code to make it all pretty and de-duplicated.
    // In the query, list all components (and fake ones) which the system
    // can access.  This list will be used for the topological insert sort.
    pub fn immutable<Q: hecs::Query>(mut self, call: &dyn SystemCall) -> Self {
        let mut components = Vec::new();
        Q::Fetch::for_each_borrow(|id, access| {
            components.push((id, access));
        });

        self
    }

    pub fn flush(mut self) -> Self {
        // Empty vec will represent a flush in this schedule.
        self.dag.push((ComponentSet::new(), Vec::new()));
        self
    }
}
