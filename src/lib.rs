//! A handy ECS
//!
//! hecs provides a high-performance, minimalist entity-component-system (ECS) world. It is a
//! library, not a framework. In place of an explicit "System" abstraction, a `World`'s entities are
//! easily queried from regular code. Organize your application however you like!
//!
//! In order of importance, hecs pursues:
//! - fast traversals
//! - a simple interface
//! - a small dependency closure
//! - exclusion of externally-implementable functionality
//!
//! ```
//! # use hecs::*;
//! let mut world = World::new();
//! // Component types can be defined with minimal boilerplate
//! struct Weight(u32);
//! impl Component for Weight {}
//! struct Price(u32);
//! impl Component for Price {}
//!
//! let a = world.spawn((Weight(12), Price(123)));
//! let b = world.spawn((Weight(38), Price(42)));
//! // Systems can be simple for loops
//! for (price, weight) in world.query_mut::<(&mut Price, &Weight)>() {
//!   if weight.0 < 20 { price.0 *= 2; }
//! }
//! // Random access is simple and safe
//! assert_eq!(world.get::<&Price>(a).unwrap().0, 246);
//! assert_eq!(world.get::<&Price>(b).unwrap().0, 42);
//! ```

#![warn(missing_docs)]
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "std")]
extern crate std;

#[doc(hidden)]
#[doc = include_str!("../README.md")]
mod readme_doctest {}

#[doc(hidden)]
pub extern crate alloc;
#[doc(hidden)]
pub extern crate spin;

macro_rules! reverse_apply {
    ($m: ident [] $($reversed:tt)*) => {
        $m!{$($reversed),*}  // base case
    };
    ($m: ident [$first:tt $($rest:tt)*] $($reversed:tt)*) => {
        reverse_apply!{$m [$($rest)*] $first $($reversed)*}
    };
}

/// Imagine macro parameters, but more like those Russian dolls.
///
/// Calls m!(), m!(A), m!(A, B), and m!(A, B, C) for i.e. (m, A, B, C)
/// where m is any macro, for any number of parameters.
macro_rules! smaller_tuples_too {
    ($m: ident, $next: tt) => {
        $m!{}
        $m!{$next}
    };
    ($m: ident, $next: tt, $($rest: tt),*) => {
        smaller_tuples_too!{$m, $($rest),*}
        reverse_apply!{$m [$next $($rest)*]}
    };
}

mod archetype;
mod batch;
mod borrow;
mod bundle;
mod change_tracker;
mod command_buffer;
mod entities;
mod entity_builder;
mod entity_ref;
mod query;
mod query_one;
#[cfg(any(feature = "row-serialize", feature = "column-serialize"))]
pub mod serialize;
mod take;
mod world;

pub use archetype::{Archetype, ArchetypeColumn, ArchetypeColumnMut, TypeIdMap, TypeInfo};
pub use batch::{BatchIncomplete, BatchWriter, ColumnBatch, ColumnBatchBuilder, ColumnBatchType};
pub use bundle::{
    bundle_satisfies_query, dynamic_bundle_satisfies_query, Bundle, DynamicBundle,
    DynamicBundleClone, MissingComponent,
};
pub use change_tracker::{ChangeTracker, Changes};
pub use command_buffer::CommandBuffer;
pub use entities::{Entity, NoSuchEntity};
pub use entity_builder::{BuiltEntity, BuiltEntityClone, EntityBuilder, EntityBuilderClone};
pub use entity_ref::{ComponentRef, ComponentRefShared, EntityRef, Ref, RefMut};
pub use query::{
    Access, Batch, BatchedIter, Or, PreparedQuery, PreparedQueryBorrow, PreparedQueryIter,
    PreparedView, Query, QueryBorrow, QueryIter, QueryMut, QueryShared, Satisfies, View,
    ViewBorrow, With, Without,
};
pub use query_one::QueryOne;
pub use take::TakenEntity;
pub use world::{
    ArchetypesGeneration, Component, ComponentError, Iter, QueryOneError, SpawnBatchIter,
    SpawnColumnBatchIter, World,
};

// Unstable implementation details needed by the macros
#[doc(hidden)]
pub use bundle::DynamicClone;
#[doc(hidden)]
pub use entities::EntityMeta;
#[doc(hidden)]
pub use query::Fetch;

#[cfg(feature = "macros")]
pub use hecs_macros::{Bundle, Component, DynamicBundleClone, Query};

fn align(x: usize, alignment: usize) -> usize {
    debug_assert!(alignment.is_power_of_two());
    (x + alignment - 1) & (!alignment + 1)
}
