// Copyright 2019 Google LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

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
//! let a = world.spawn((123, true, "abc"));
//! let b = world.spawn((42, false));
//! for (id, (number, &flag)) in world.query::<(&mut i32, &bool)>() {
//!   if flag { *number *= 2; }
//! }
//! assert_eq!(*world.get::<i32>(a).unwrap(), 246);
//! assert_eq!(*world.get::<i32>(b).unwrap(), 42);
//! ```

#![warn(missing_docs)]

mod archetype;
mod borrow;
mod bundle;
mod entity_builder;
mod query;
mod world;

pub use borrow::{EntityRef, Ref, RefMut};
pub use bundle::{Bundle, DynamicBundle, MissingComponent};
pub use entity_builder::{BuiltEntity, EntityBuilder};
pub use query::{Query, QueryIter};
pub use world::{Component, ComponentError, Entity, Iter, NoSuchEntity, World};

// Unstable implementation details needed by the macros
#[doc(hidden)]
pub use archetype::{Archetype, TypeInfo};
#[doc(hidden)]
pub use borrow::BorrowState;
#[cfg(feature = "macros")]
#[doc(hidden)]
pub use once_cell;
#[doc(hidden)]
pub use query::Fetch;

#[cfg(feature = "macros")]
pub use hecs_macros::{Bundle, Query};

#[inline]
fn align(x: usize, alignment: usize) -> usize {
    debug_assert!(alignment.is_power_of_two());
    (x + alignment - 1) & (!alignment + 1)
}
