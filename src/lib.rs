// Copyright 2019 Google LLC
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

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
//! // Nearly any type can be used as a component with zero boilerplate
//! let a = world.spawn((123, true, "abc"));
//! let b = world.spawn((42, false));
//! // Systems can be simple for loops
//! for (id, (number, &flag)) in world.query_mut::<(&mut i32, &bool)>() {
//!   if flag { *number *= 2; }
//! }
//! // Random access is simple and safe
//! assert_eq!(*world.get::<i32>(a).unwrap(), 246);
//! assert_eq!(*world.get::<i32>(b).unwrap(), 42);
//! ```

#![warn(missing_docs)]
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "std")]
extern crate std;

extern crate alloc;

/// Imagine macro parameters, but more like those Russian dolls.
///
/// Calls m!(A, B, C), m!(A, B), m!(B), and m!() for i.e. (m, A, B, C)
/// where m is any macro, for any number of parameters.
macro_rules! smaller_tuples_too {
    ($m: ident, $ty: ident) => {
        $m!{}
        $m!{$ty}
    };
    ($m: ident, $ty: ident, $($tt: ident),*) => {
        smaller_tuples_too!{$m, $($tt),*}
        $m!{$ty, $($tt),*}
    };
}

mod archetype;
mod batch;
mod borrow;
mod bundle;
mod column;
mod command_buffer;
mod entities;
mod entity_builder;
mod entity_ref;
mod query;
mod query_one;
#[cfg(any(feature = "row-serialize", feature = "column-serialize"))]
pub mod serialize;
mod world;

pub use archetype::{Archetype, ArchetypeColumn};
pub use batch::{BatchIncomplete, BatchWriter, ColumnBatch, ColumnBatchBuilder, ColumnBatchType};
pub use bundle::{Bundle, DynamicBundle, DynamicBundleClone, MissingComponent};
pub use column::{Column, ColumnMut};
pub use command_buffer::CommandBuffer;
pub use entities::{Entity, NoSuchEntity};
pub use entity_builder::{BuiltEntity, BuiltEntityClone, EntityBuilder, EntityBuilderClone};
pub use entity_ref::{EntityRef, Ref, RefMut};
pub use query::{
    Access, Batch, BatchedIter, Or, PreparedQuery, PreparedQueryBorrow, PreparedQueryIter, Query,
    QueryBorrow, QueryItem, QueryIter, QueryMut, QueryShared, Satisfies, View, With, Without,
};
pub use query_one::QueryOne;
pub use world::{
    ArchetypesGeneration, Component, ComponentError, Iter, QueryOneError, SpawnBatchIter,
    SpawnColumnBatchIter, World,
};

// Unstable implementation details needed by the macros
#[doc(hidden)]
pub use archetype::TypeInfo;
#[doc(hidden)]
pub use bundle::DynamicClone;
#[cfg(feature = "macros")]
#[doc(hidden)]
pub use lazy_static;
#[doc(hidden)]
pub use query::Fetch;

#[cfg(feature = "macros")]
pub use hecs_macros::{Bundle, Query};

fn align(x: usize, alignment: usize) -> usize {
    debug_assert!(alignment.is_power_of_two());
    (x + alignment - 1) & (!alignment + 1)
}
