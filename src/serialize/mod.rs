//! Convenience tools for serializing [`World`](crate::World)s; requires the `serde` feature
//!
//! [`Component`](crate::Component)s are not necessarily serializable, so we cannot directly
//! implement [`serde::Serialize`] for [`World`](crate::World). The helpers defined in this module
//! allow serialization and deserialization based on purpose-defined traits to control the
//! procedures explicitly.
//!
//! Backwards-incompatible changes to the serde data models herein are subject to the same semantic
//! versioning stability guarantees as the hecs API.

pub mod column;
pub mod row;
