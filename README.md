# hecs

[![Documentation](https://docs.rs/hecs/badge.svg)](https://docs.rs/hecs/)
[![Crates.io](https://img.shields.io/crates/v/hecs.svg)](https://crates.io/crates/hecs)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE-APACHE)

hecs provides a high-performance, minimalist entity-component-system (ECS)
world. It is a library, not a framework. In place of an explicit "System"
abstraction, a `World`'s entities are easily queried from regular code. Organize
your application however you like!

### Example

```rust
let mut world = World::new();
// Nearly any type can be used as a component with zero boilerplate
let a = world.spawn((123, true, "abc"));
let b = world.spawn((42, false));
// Systems can be simple for loops
for (id, (number, &flag)) in world.query_mut::<(&mut i32, &bool)>() {
  if flag { *number *= 2; }
}
// Random access is simple and safe
assert_eq!(*world.get::<&i32>(a).unwrap(), 246);
assert_eq!(*world.get::<&i32>(b).unwrap(), 42);
```

### Why ECS?

Entity-component-system architecture makes it easy to compose loosely-coupled
state and behavior. An ECS world consists of:

- any number of **entities**, which represent distinct objects
- a collection of **component** data associated with each entity, where each
  entity has at most one component of any type, and two entities may have
  different components

That world is then manipulated by **systems**, each of which accesses all
entities having a particular set of component types. Systems implement
self-contained behavior like physics (e.g. by accessing "position", "velocity",
and "collision" components) or rendering (e.g. by accessing "position" and
"sprite" components).

New components and systems can be added to a complex application without
interfering with existing logic, making the ECS paradigm well suited to
applications where many layers of overlapping behavior will be defined on the
same set of objects, particularly if new behaviors will be added in the
future. This flexibility sets it apart from traditional approaches based on
heterogeneous collections of explicitly defined object types, where implementing
new combinations of behaviors (e.g. a vehicle which is also a questgiver) can
require far-reaching changes.

#### Performance

In addition to having excellent composability, the ECS paradigm can also provide
exceptional speed and cache locality. `hecs` internally tracks groups of
entities which all have the same components. Each group has a dense, contiguous
array for each type of component. When a system accesses all entities with a
certain set of components, a fast linear traversal can be made through each
group having a superset of those components. This is effectively a columnar
database, and has the same benefits: the CPU can accurately predict memory
accesses, bypassing unneeded data, maximizing cache use and minimizing latency.

### Why Not ECS?

hecs strives to be lightweight and unobtrusive so it can be useful in
a wide range of applications. Even so, it's not appropriate for every
game. If your game will have few types of entities, consider a simpler
architecture such as storing each type of entity in a separate plain
`Vec`. Similarly, ECS may be overkill for games that don't call for
batch processing of entities.

Even for games that benefit, an ECS world is not a be-all end-all data
structure. Most games will store significant amounts of state in other
structures. For example, many games maintain a spatial index structure
(e.g. a tile map or bounding volume hierarchy) used to find entities
and obstacles near a certain location for efficient collision
detection without searching the entire world.

If you need to search for specific entities using criteria other than the types
of their components, consider maintaining a specialized index beside your world,
storing `Entity` handles and whatever other data is necessary. Insert into the
index when spawning relevant entities, and include a component with that allows
efficiently removing them from the index when despawning.

### Other Libraries

hecs owes a great deal to the free exchange of ideas in Rust's ECS library
ecosystem. Particularly influential examples include:

- [bevy], which continually pushes the envelope for performance and ergonomics
  in the context of a batteries-included framework
- [specs], which was key in popularizing ECS in Rust
- [legion], which demonstrated archetypal memory layout and trait-less
  components

If hecs doesn't suit you, one of those might do the trick!

## License

Licensed under either of

 * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the
Apache-2.0 license, shall be dual licensed as above, without any
additional terms or conditions.

[bevy]: https://github.com/bevyengine/bevy
[specs]: https://github.com/amethyst/specs
[legion]: https://github.com/TomGillen/legion
