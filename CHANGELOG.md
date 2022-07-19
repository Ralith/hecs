# Unreleased

### Removed
- APIs deprecated in 0.7

# 0.8.1

### Fixed
- Empty archetypes no longer participate in dynamic query borrow-checking

# 0.8.0

### Added
- `World::satisfies` and `EntityRef::satisfies` to check if an entity would match a query

### Changed
- Many generic methods that previously took a `Component` now instead take either a
  `ComponentRef<'a>` or a `Query` to improve consistency with queries and address a common footgun:
  - `World::get`, `EntityRef::get`, and `Archetype::get` now take shared or unique references to
    component types
  - `EntityBuilder` and `EntityBuilderClone`'s `get` and `get_mut` refactored along the same lines
    for consistency
  - The `With`/`Without` query transformers now take a query that entities must/mustn't match rather
    than a component type. Additionally, the order of their generic arguments was reversed to place
    the query whose results will be yielded first.
- `SerializeContext` traits now take their serializer arguments by value, and must call `end()`
  themselves.

# 0.7.7
  
### Added
- `Entity::DANGLING` convenience constant

### Fixed
- Various bad behavior when dangling `Entity` handles are used
- Inconsistent component counts in column serialization

# 0.7.6

### Added
- `World::take` for moving complete entities between worlds
- `CommandBuffer::remove` and `CommandBuffer::despawn`

### Fixed
- Panics on use of cloned `BuiltEntityClone`s

# 0.7.5

### Changed
- Bump hashbrown version

# 0.7.4

### Added
- `get_unchecked` and `get_mut_n` methods on `View` and `PreparedView` for overlapping borrows

### Changed
- Minimum supported Rust version is now explicit in Cargo.toml, currently 1.57
- Several internal optimizations thanks to Adam Reichold

### Fixed
- `derive(DynamicBundleClone)` had an unintended dependency on `DynamicClone` being in scope
- `World::len` was incorrect following `World::clear`
- Missing re-export of `PreparedView`

# 0.7.3

### Fixed
- Insufficiently strict hecs-macros version constraints

# 0.7.2

### Added
- `World::exchange` provides an optimized path for a remove immediately followed by an insert
- Efficient random access can be performed within queries using `QueryBorrow::view` and similar
  methods, a generalization of `Column`/`ColumnMut`.

### Changed
- `World::column`/`column_mut` deprecated in favor of views.

# 0.7.1

### Added
- `EntityBuilderClone::add_bundle` accepting bundles of `Clone` components

# 0.7.0

### Added
- `Or` query combinator, allowing a single query to select entities that satisfy at least one of two
  sub-queries.
- `EntityRef::has` to efficiently check for the presence of a component without borrowing it
- `EntityBuilderClone` helper for working with dynamic collections of
  components that may be used repeatedly
- `Satisfies` query combinator, which yields a `bool` without borrowing any components
- `World::column` and `column_mut` for efficient random access within
  a single component type
- `CommandBuffer` helper for recording operations on a world in advance

### Changed
- Added a niche to `Entity`, making `Option<Entity>` the same size as a bare `Entity`. As a
  consequence, `Entity::from_bits` is now fallible, and deserialization of `Entity` values from
  older versions may fail.

### Fixed
- Compatibility with 32-bit MIPS and PPC
- Missing `Batch` reexport
- Missing `ArchetypeColumn` reexport

# 0.6.5

### Changed
- Internal memory layout adjusted, reducing `TypeId`-to-pointer look-ups for a minor speedup in
  per-archetype processing and component insertion/removal.

# 0.6.4

### Fixed
- `EntityRef::entity` panicking on empty entities

# 0.6.3

### Fixed
- An invalid `Entity` could panic rather than returning the appropriate error, especially after
  `World::clear`
- Reexported `BatchIncomplete`

# 0.6.2

### Fixed
- Reexported `BatchWriter`

# 0.6.1

### Fixed
- Random-access lookup structures were not correctly populated by column deserialization

# 0.6.0

### Changed
- `World::iter` no longer returns entity IDs directly; they can now instead be fetched from the
  `EntityRef`
- `World::spawn_batch` and `World::reserve` now employ `Vec`-style amortized resizing, improving
  performance when called repeatedly.

### Added
- `EntityRef::query` as an alternative to `World::query_one` when you already have an `EntityRef`
- `EntityRef::entity` accessor for recovering the entity's handle
- `PreparedQuery` for improved performance when repeatedly issuing the same query

# 0.5.2

### Fixed
- `World::query_mut` did not prevent aliasing mutable borrows within the query

# 0.5.1

### Changed
- Documentation updates only

# 0.5.0

### Changed
- Improved performance for spawning, inserting, or removing statically-typed component bundles

### Fixed
- `World::archetypes_generation` not updated when a column batch spawn introduces a new archetype

# 0.4.0

### Changed
- Row-major serialization moved under `serialize::row` and behind the `row-serialize` cargo feature

### Added
- `EntityRef::len` to query how many components an entity has
- Support for serializers that require maps to be of known length
- An alternative column-major serialization layout for better performance and compressibility, in
  the `serialize::column` module behind the `column-serialize` feature

### Fixed
- Incorrect behavior when building a `ColumnBatch` of zero entities

# 0.3.2

### Added
- The `serde` feature, enabling serialization of `Entity` handles, and a `serialization` module to
  simplify (de)serializing worlds
- `World::len()` exposing the number of live entities
- Access to component data inside `Archetypes` to allow custom column-major operations
- `ColumnBatch` for efficiently spawning collections of entities with the same components when those
  components' types are not statically known

# 0.3.1 (November 9, 2020)

### Fixed
- Incorrect alignment handling in `EntityBuilder`

# 0.3.0 (November 8, 2020)

This release includes API extensions and many optimizations, particularly to query
iteration. Special thanks are due to contributors mjhostet for performance improvements, sdleffler
and AngelOfSol for API improvements and internal refactoring, Veykril for rewriting
`#[derive(Bundle)]`, and cart for coordinating with the bevy community. This release wouldn't have
been possible without their hard work!

### Added
- `#[derive(Query)]` for more ergonomic specification of complex queries
- Support for generic, tuple, and unit structs in `#[derive(Bundle)]`
- `World::query_mut` and `World::query_one_mut` reduce setup cost when dynamic borrow checks are
  unnecessary
- `QueryItem<'a, Q>` type alias identifying the output of the query `Q` borrowing for lifetime `'a`
- `World::find_entity_from_id` allows finding a live entity's `Entity` handle from its 32-bit `id`.
- `World::spawn_at` allows creating a new entity for an existing `Entity` handle, enabling easier
  deserialization.
- `Archetype::component_types` simplifies certain scripting patterns

### Fixed
- Panic when passing empty bundles to `World::remove`
- Misbehavior when using tuple bundles with multiple fields of the same type
