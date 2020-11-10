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
