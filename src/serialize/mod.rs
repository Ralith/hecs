//! Convenience tools for serializing [`World`](crate::World)s
//!
//! [`Component`](crate::Component)s are not necessarily serializable, so we cannot directly
//! implement [`serde::Serialize`] for [`World`](crate::World). The helpers defined in this module
//! allow serialization and deserialization based on purpose-defined traits to control the
//! procedures explicitly.
//!
//! Backwards-incompatible changes to the serde data models herein are subject to the same semantic
//! versioning stability guarantees as the hecs API.

#[cfg(feature = "column-serialize")]
#[cfg_attr(docsrs, doc(cfg(feature = "column-serialize")))]
pub mod column;
#[cfg(feature = "row-serialize")]
#[cfg_attr(docsrs, doc(cfg(feature = "row-serialize")))]
pub mod row;
