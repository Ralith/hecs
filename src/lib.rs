//! A handy ECS
//!
//! hecs provides a high-performance, minimalist ECS world. It is a library, not a framework. In
//! place of an explicit "System" abstraction, a `World`'s entities are easily queried from regular
//! code. Organize your application however you like!
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
mod query;
mod world;

pub use borrow::{EntityRef, Ref, RefMut};
pub use query::{Query, QueryIter};
pub use world::{Bundle, Component, Entity, EntityBuilder, Iter, NoSuchEntity, World};

// Detailed needed by the macros
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
