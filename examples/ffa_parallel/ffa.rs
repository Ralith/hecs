use hecs::*;
use rand::{thread_rng, Rng};
use std::{
    any::TypeId,
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{
        mpsc::{channel, Receiver, Sender},
        Mutex,
    },
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

// Super duper cheeze...  These are shared resources used while
// running the systems.  In a real scheduler the world would own
// these items and they would be integrated into the topological
// sort.  That's beyond scope here.
struct Shared {
    damage_senders: Vec<Sender<(Entity, Entity)>>,
    damage_receiver: Receiver<(Entity, Entity)>,
    commands: Mutex<CommandBuffer>,
}
impl Shared {
    pub fn new() -> Self {
        let (tx, rx) = channel::<(Entity, Entity)>();
        Self {
            damage_senders: vec![tx; rayon::current_num_threads()],
            damage_receiver: rx,
            commands: Mutex::new(CommandBuffer::new()),
        }
    }
    pub fn sender(&self, index: usize) -> &Sender<(Entity, Entity)> {
        &self.damage_senders[index]
    }
    pub fn receiver(&self) -> &Receiver<(Entity, Entity)> {
        &self.damage_receiver
    }
    pub fn commands(&self) -> &Mutex<CommandBuffer> {
        &self.commands
    }
}

unsafe impl Sync for Shared {}

lazy_static::lazy_static! {
    static ref SHARED: Shared = {
        Shared::new()
    };
}

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

fn integrate_motion(_: &World, id: Entity, (pos, s): (&mut Position, &Speed)) {
    let mut rng = thread_rng();
    let change = (rng.gen_range(-s.0..s.0), rng.gen_range(-s.0..s.0));
    pos.x += change.0;
    pos.y += change.1;
    println!("Unit {:?} moved to {:?}", id, pos);
}

fn fire_at_closest(world: &World, id0: Entity, pos0: &Position) {
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
            let _ = SHARED
                .sender(rayon::current_thread_index().unwrap())
                .send((id0, entity));
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
fn system_apply_damage(world: &World) {
    let rx = SHARED.receiver();
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

fn system_remove_dead(_: &World, entity: Entity, health: &Health) {
    if health.0 <= 0 {
        let mut commands = SHARED.commands().lock().unwrap();
        commands.despawn(entity);
    }
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

    // Standing component type to track usage of the damage queue.
    struct DamageQueue;

    // Build a schedule for the systems.
    let schedule = Schedule::create()
        .parallel::<(&mut Position, &Speed), ()>(integrate_motion)
        .parallel::<&Position, &mut DamageQueue>(fire_at_closest)
        .func::<(&mut Health, &DamageQueue)>(system_apply_damage)
        .parallel::<&Health, ()>(system_remove_dead)
        .flush()
        .build();

    println!("Schedule: {:#?}", schedule);

    loop {
        println!("\n'Enter' to continue simulation, '?' for entity list, 'q' to quit");

        let mut input = String::new();

        io::stdin().read_line(&mut input).unwrap();

        match input.trim() {
            "" => {
                // Execute the schedule.
                schedule.execute(&mut world);
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
trait SerialCall: Sync + std::fmt::Debug {
    fn call(&self, world: &World);
}

// Struct for storing serial systems.
struct Serial<Q: Query + 'static> {
    pub f: fn(&World, Entity, QueryItem<Q>),
}

impl<Q: Query + 'static> SerialCall for Serial<Q> {
    fn call(&self, world: &World) {
        let mut borrow = world.query::<Q>();
        for item in borrow.into_iter() {
            (self.f)(world, item.0, item.1);
        }
    }
}

impl<Q: Query + 'static> std::fmt::Debug for Serial<Q> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        write!(f, "Serial")
    }
}

unsafe impl<Q: Query + 'static> Sync for Serial<Q> {}

// A system which is executed in parallel on several threads.
trait ParallelCall: Sync + std::fmt::Debug {
    fn call(&self, world: &World, iter: ParallelIter);
}

// Struct for storing parallel systems.
struct Parallel<Q: Query + 'static> {
    f: fn(&World, Entity, QueryItem<Q>),
}

impl<Q: Query + 'static> ParallelCall for Parallel<Q> {
    fn call(&self, world: &World, iter: ParallelIter) {
        unsafe {
            world.parallel_query::<Q>(iter, 100, &|entity, item| (self.f)(world, entity, item))
        };
    }
}

impl<Q: Query + 'static> std::fmt::Debug for Parallel<Q> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        write!(f, "Parallel")
    }
}

unsafe impl<Q: Query + 'static> Sync for Parallel<Q> {}

// A single function call, not a query.
trait Func: Sync + std::fmt::Debug {
    fn call(&self, world: &World);
}

// Struct for storing non-system call.
struct FuncCall {
    f: fn(&World),
}

impl Func for FuncCall {
    fn call(&self, world: &World) {
        (self.f)(world)
    }
}

impl std::fmt::Debug for FuncCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        write!(f, "Func")
    }
}

unsafe impl Sync for FuncCall {}

// Execution style enum.  Systems can be executed serial or parallel depending
// on requirements.  Not all systems can be made fully parallel, but they can
// usually still run in parallel with other systems depending on data access.
#[derive(Debug)]
enum Execution {
    Serial(Box<dyn SerialCall>),
    Parallel(Box<dyn ParallelCall>),
    Func(Box<dyn Func>),
}

// An entry in the schedule representing either an iteration or a command buffer
// flush.
#[derive(Debug)]
enum Entry {
    Systems(Vec<Execution>),
    Flush,
}

// A compiled schedule.
#[derive(Debug)]
struct Schedule(Vec<Entry>);

impl Schedule {
    fn create() -> ScheduleBuilder {
        ScheduleBuilder::new()
    }

    fn with(entries: Vec<Entry>) -> Self {
        Self(entries)
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
                                    Execution::Func(f) => {
                                        scope.spawn({
                                            let world: &World = world;
                                            move |_| {
                                                f.call(world);
                                            }
                                        });
                                    }
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
                                let mut commands = SHARED.commands().lock().unwrap();
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

    pub fn func<D: Query>(mut self, f: fn(&World)) -> Self {
        // Get a set of all components in use.
        let components = ComponentSet::from_query::<D>();

        // Insert sort checking for data constraints.
        let mut compatible: Option<usize> = None;
        let len = self.dag.len();
        for (index, target) in self.dag.iter_mut().rev().enumerate() {
            if target.0.is_empty() || !components.is_compatible(&target.0) {
                break;
            }
            compatible = Some(len - index - 1);
        }

        // Insert it.
        self.insert(compatible, Execution::Func(Box::new(FuncCall { f })));
        self
    }

    // Two queries here.  The `Q` query type represents the set of component arguments
    // to the system function.  The 'D' query is used to represent any other components
    // which the execution of the system might end up touching.  There are no protections
    // against incorrect lists of components here, a full solution would check for
    // validity.
    pub fn serial<Q: Query + 'static, D: Query>(
        mut self,
        f: fn(&World, Entity, QueryItem<Q>),
    ) -> Self {
        // Get a set of all components in use.
        let components = ComponentSet::from_query::<Q>();
        let components = components.merge(&ComponentSet::from_query::<D>());

        // Insert sort checking for data constraints.
        let mut compatible: Option<usize> = None;
        let len = self.dag.len();
        for (index, target) in self.dag.iter_mut().rev().enumerate() {
            if target.0.is_empty() || !components.is_compatible(&target.0) {
                break;
            }
            compatible = Some(len - index - 1);
        }

        // Insert it.
        self.insert(compatible, Execution::Serial(Box::new(Serial::<Q> { f })));
        self
    }

    pub fn parallel<Q: Query + 'static, D: Query>(
        mut self,
        f: fn(&World, Entity, QueryItem<'_, Q>),
    ) -> Self {
        // Get a set of all components in use.
        let components = ComponentSet::from_query::<Q>();
        let components = components.merge(&ComponentSet::from_query::<D>());

        // Insert sort checking for data constraints.
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
            Execution::Parallel(Box::new(Parallel::<Q> { f })),
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

    pub fn build(self) -> Schedule {
        Schedule::with(self.dag.into_iter().map(|(_, e)| e).collect())
    }
}
