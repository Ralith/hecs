//! A handy ECS
//!
//! hecs provides a high-performance, minimalist ECS abstraction.
//!
//! In order of importance, hecs pursues:
//! - fast iteration
//! - a simple interface
//! - a small dependency closure
//!
//! ```
//! # use hecs::*;
//! let mut world = World::new();
//! let a = world.spawn((123, true, "abc"));
//! let b = world.spawn((42, false, "def"));
//! for (id, (number, &flag)) in world.query::<(&mut i32, &bool)>() {
//!   if flag { *number *= 2; }
//! }
//! assert_eq!(*world.get::<i32>(a).unwrap(), 246);
//! assert_eq!(*world.get::<i32>(b).unwrap(), 42);
//! ```


#![warn(missing_docs)]

mod archetype;
mod query;
mod world;

pub use query::{Query, QueryIter};
pub use world::{Component, ComponentSet, Entity, EntityBuilder, NoSuchEntity, World};
