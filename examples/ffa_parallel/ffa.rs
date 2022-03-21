use hecs::*;
use rand::{thread_rng, Rng};
use std::{
    any::TypeId,
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Mutex,
};

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
the systems.  The sort is based on normal Rust borrow rules such that at any
time during execution there is never a mutable reference to data at the same
time as any immutable reference.  The topological sort generates a DAG which
is used to determine execution order which maintains the required constraints.
For the purposes here, the sorting is performed at time of system insertion
and rather than an actual graph, systems are grouped in depth ordered vectors.

Since hecs does not have a concept of shared components, a few fake components
are used to represent external to world data flows.  For instance, the damage
queue which is written to within the nearest search and read from within the
apply damage systems.

Provided here is the high level concept of a full threaded scheduler.  While
not complete or using a real graph executor, the concepts are all in place.
Using this as the starting point for a full blown threaded execution of the
ECS is viable, though to make it ergonomic and safer in practice requires a
fair amount of additional abstraction.

ECS execution within a work stealing job system is not a great solution to
the backend threading.  A work stealing job system is great for a large number
of types of work but the overhead for job management increases greatly
for the patterns used in an ECS.  Using a proper graph executor rather than
the current example based on Rayon can provide considerably higher performance
and scalability, unfortunately this is already much larger as an example than
originally intended.
 */

// A system which executes on a single thread.
trait SerialCall: Sync {
    fn call(&self, world: &World);
}

// A system which is executed in parallel on several threads.
trait ParallelCall: Sync {
    fn call(&self, world: &World, iter: ParallelIter);
}

// Struct for storing serial systems.
struct Serial<'a, Q: Query + 'static> {
    pub f: &'a fn(&World, Entity, QueryItem<Q>),
}

impl<'a, Q: Query + 'static> SerialCall for Serial<'a, Q> {
    fn call(&self, world: &World) {
        let mut borrow = world.query::<Q>();
        for item in borrow.into_iter() {
            (*self.f)(world, item.0, item.1);
        }
    }
}

unsafe impl<'a, Q: Query + 'static> Sync for Serial<'a, Q> {}

// Struct for storing parallel systems.
struct Parallel<'a, Q: Query + 'static> {
    f: &'a fn(&World, Entity, QueryItem<Q>),
}

impl<'a, Q: Query + 'static> ParallelCall for Parallel<'a, Q> {
    fn call(&self, world: &World, iter: ParallelIter) {
        unsafe {
            world.parallel_query::<Q>(iter, 100, &|entity, item| (*self.f)(world, entity, item))
        };
    }
}

unsafe impl<'a, Q: Query + 'static> Sync for Parallel<'a, Q> {}

// Execution style enum.  Systems can be executed serial or parallel depending
// on requirements.  Not all systems can be made fully parallel, but they can
// usually still run in parallel with other systems depending on data access.
enum Execution {
    Serial(Box<dyn SerialCall>),
    Parallel(Box<dyn ParallelCall>),
}

// An entry in the schedule representing either an iteration or a command buffer
// flush.
enum Entry {
    Systems(Vec<Execution>),
    Flush,
}

// A compiled schedule.
struct Schedule(Vec<Entry>, Mutex<CommandBuffer>);

impl Schedule {
    fn create() -> ScheduleBuilder {
        ScheduleBuilder::new()
    }

    fn new() -> Self {
        Self(Vec::new(), Mutex::new(CommandBuffer::new()))
    }

    pub fn execute(&self, world: &mut World) {
        // Using Rayon is not the best solution for graph execution but there
        // was no existing crate with a graph executor which was available.
        rayon::scope(|_| {
            // This scope represents the overall execution of the entire schedule.
            // In rayon, this is an issued job to the backend which will block the
            // calling thread until the entire schedule completes.  AKA: fork/join
            // in order to maintain Rust "scoping" on the main thread.

            // During schedule execution, this scope owns the mutable world reference.
            let world = world;

            // Each loop issues a new scope.  This is equivalent to what a graph
            // executor would do in terms that it issues the system calls then a
            // fence/barrier or other synchronization point.
            for entry in &self.0 {
                match entry {
                    Entry::Systems(systems) => {
                        // The list of systems which can run in parallel at this level
                        // of traversing the dag.
                        rayon::scope(|scope| {
                            // Use the immutable world within this scope.
                            let world: &World = world;
                            for system in systems {
                                match system {
                                    Execution::Serial(serial) => {
                                        // Issue a single job for this.
                                        scope.spawn({
                                            let world: &World = world;
                                            move |_| {
                                                serial.call(world);
                                            }
                                        });
                                    }
                                    Execution::Parallel(parallel) => {
                                        // Execute x jobs.
                                        let iter = ParallelIter::new();
                                        for _ in 0..rayon::current_num_threads() {
                                            scope.spawn({
                                                let world: &World = world;
                                                let iter = iter.clone();
                                                move |_| {
                                                    parallel.call(world, iter);
                                                }
                                            });
                                        }
                                    }
                                }
                            }
                        });
                    }
                    Entry::Flush => {
                        // Flush the command buffer.
                        rayon::scope({
                            let world: &mut World = world;
                            move |_| {
                                let mut commands = self.1.lock().unwrap();
                                commands.run_on(world);
                            }
                        });
                    }
                }
            }
        });
    }
}

/// A component set which is used during topological sorting.
struct ComponentSet(BTreeMap<TypeId, bool>);

impl ComponentSet {
    pub fn new() -> Self {
        Self(BTreeMap::new())
    }

    pub fn from_query<Q: Query>() -> Self {
        let mut components = BTreeMap::<TypeId, bool>::new();
        Q::Fetch::for_each_borrow(|id, access| {
            components.insert(id, access);
        });
        Self(components)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    // Immutable types.
    pub fn immutable(&self) -> BTreeSet<TypeId> {
        self.0.iter().filter(|kv| !*kv.1).map(|kv| *kv.0).collect()
    }

    // Mutable types.
    pub fn mutable(&self) -> BTreeSet<TypeId> {
        self.0.iter().filter(|kv| *kv.1).map(|kv| *kv.0).collect()
    }

    // Check two sets for compatibility.
    pub fn is_compatible(&self, rhs: &Self) -> bool {
        for entry in &self.0 {
            if *entry.1 {
                // If the lhs type is mutable, the rhs is incompatible if it contains
                // the same typeid no matter what it's mutability.
                if rhs.0.contains_key(entry.0) {
                    return false;
                }
            } else {
                // The lhs is immutable, the rhs is only incompatible if it contains
                // a mutable of the same typeid.
                if let Some(rhs) = rhs.0.get_key_value(entry.0) {
                    if *rhs.1 {
                        return false;
                    }
                }
            }
        }
        false
    }

    // A union of the two sets of keys where the mutability is the logical
    // `or` of the two sides.
    pub fn merge(&self, rhs: &ComponentSet) -> Self {
        // Resulting map with mutability raised to highest value.
        let mut result = self.0.clone();

        for (k, v) in &rhs.0 {
            if let Some(e) = result.get_mut(k) {
                *e = *v || *e;
            } else {
                result.insert(*k, *v);
            }
        }

        Self(result)
    }
}

struct ScheduleBuilder {
    dag: Vec<(ComponentSet, Entry)>,
}

impl ScheduleBuilder {
    pub fn new() -> Self {
        Self { dag: Vec::new() }
    }

    // Two queries here.  The `Q` query type represents the set of component arguments
    // to the system function.  The 'D' query is used to represent any other components
    // which the execution of the system might end up touching.  There are no protections
    // against incorrect lists of components here, a full solution would check for
    // validity.
    pub fn serial<Q: Query + 'static, D: Query>(
        mut self,
        f: &'static fn(&World, Entity, QueryItem<Q>),
    ) -> Self {
        // Get a set of all components in use.
        let components = ComponentSet::from_query::<Q>();
        let components = components.merge(&ComponentSet::from_query::<D>());

        // Iterate from the end checking if this new system is compatible.
        let mut compatible: Option<usize> = None;
        let len = self.dag.len();
        for (index, target) in self.dag.iter_mut().rev().enumerate() {
            if target.0.is_empty() || !components.is_compatible(&target.0) {
                break;
            }
            compatible = Some(len - index - 1);
        }

        // Insert it.
        self.insert(
            compatible,
            Execution::Serial(Box::new(Serial::<'_, Q> { f })),
        );
        self
    }

    fn insert(&mut self, index: Option<usize>, execution: Execution) {
        if let Some(index) = index {
            // Insert into the last compatible group we found.
            match &mut self.dag[index].1 {
                Entry::Systems(systems) => {
                    systems.push(execution);
                }
                _ => panic!("Internal error."),
            }
        } else {
            // Just push to the end.
            self.dag
                .push((ComponentSet::new(), Entry::Systems(vec![execution])));
        }
    }

    pub fn flush(mut self) -> Self {
        // Empty vec will represent a flush in this schedule.
        self.dag.push((ComponentSet::new(), Entry::Flush));
        self
    }
}
