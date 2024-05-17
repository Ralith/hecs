//! This module provides a mechanism to efficiently clone a given [World](crate::world::World).
//!
//! As each component for each entity in a World is stored in a type-erased fashion, cloning entity
//! components requires registering each component with a [Cloner].
//!
//! See the documentation for [World::try_clone()](crate::world::World::try_clone()) for example usage.

use crate::archetype::TypeIdMap;
use core::any::TypeId;
use core::ptr;

/// A type erased way to copy or clone multiple instances of a type.
pub(crate) struct BulkCloneFunction(unsafe fn(*const u8, *mut u8, usize));

impl BulkCloneFunction {
    pub(crate) unsafe fn call(&self, src: *const u8, dst: *mut u8, count: usize) {
        (self.0)(src, dst, count)
    }
}

/// Maps component types which can be cloned or copied to their relevant cloning
/// or copying function.
///
/// Populating such an object with all component types present in the world via
/// [`add_copyable`](Self::add_copyable) or
/// [`add_clonable`](Self::add_cloneable)
/// is required to
/// use [`World::try_clone`][crate::World::try_clone].
///
/// A Cloner instance is safe to reuse.
///
/// Registering types which are unused is allowed.
#[derive(Default)]
pub struct Cloner {
    /// Type erased cloner: fn(src: *const u8, dst: *mut u8, len: usize)
    pub(crate) typeid_to_clone_fn: TypeIdMap<BulkCloneFunction>,
}

impl Cloner {
    /// Creates a new [Cloner].
    ///
    /// The cloner is not aware of any types out of the box: all types present
    /// in the [World](crate::world::World) as components (even built in types
    /// such as [i32](core::i32)) must be added using
    /// [`add_copyable`](Self::add_copyable) or
    /// [`add_clonable`](Self::add_cloneable) before using this Cloner.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Cloner {
    /// Adds a component type which is copyable.
    pub fn add_copyable<C>(&mut self)
    where
        C: Copy + 'static,
    {
        unsafe fn clone<C>(src: *const u8, dst: *mut u8, count: usize)
        where
            C: Copy,
        {
            let src = src.cast::<C>();
            let dst = dst.cast::<C>();

            ptr::copy_nonoverlapping(src, dst, count);
        }

        self.typeid_to_clone_fn
            .insert(TypeId::of::<C>(), BulkCloneFunction(clone::<C>));
    }

    /// Adds a component type which is cloneable.
    ///
    /// If `C` is actually copyable, using [`add_copyable`][Self::add_copyable]
    /// is more efficient.
    pub fn add_cloneable<C>(&mut self)
    where
        C: Clone + 'static,
    {
        unsafe fn clone<C>(src: *const u8, dst: *mut u8, count: usize)
        where
            C: Clone,
        {
            let src = src.cast::<C>();
            let dst = dst.cast::<C>();

            for idx in 0..count {
                let val = (*src.add(idx)).clone();
                dst.add(idx).write(val);
            }
        }

        self.typeid_to_clone_fn
            .insert(TypeId::of::<C>(), BulkCloneFunction(clone::<C>));
    }
}

/// Error returned when the [`Cloner`] has not had [`Cloner::add_cloneable`] or
/// [`Cloner::add_copyable`] called for the contained type.
///
/// When compiled with debug assertions enabled, this error will include the
/// name of the missing type in its [Display](std::fmt::Display) output.
#[derive(Debug)]
pub struct TypeUnknownToCloner {
    /// The name of the type which was unrecognized.
    ///
    /// This is subject to the same guarantees and caveats as [`core::any::type_name()`].
    #[cfg(debug_assertions)]
    pub type_name: &'static str,

    /// The id of the type which was unrecognized.
    ///
    /// This is subject to the same guarantees and caveats as [`core::any::TypeId::of()`].
    pub type_id: TypeId,
}

#[cfg(feature = "std")]
impl std::fmt::Display for TypeUnknownToCloner {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        #[cfg(debug_assertions)]
        let type_name = self.type_name;
        #[cfg(not(debug_assertions))]
        let type_name = "<type name is not available when compiled without debug_assertions>";
        write!(
            f,
            "Type unknown to Cloner: {type_name} (TypeId: {:?})",
            self.type_id
        )
    }
}

#[cfg(feature = "std")]
impl std::error::Error for TypeUnknownToCloner {}

#[cfg(test)]
mod tests {
    use alloc::{borrow::ToOwned, string::String};

    use super::*;
    use crate::*;

    #[derive(PartialEq, Debug, Copy, Clone)]
    struct Position([f32; 2]);

    #[derive(PartialEq, Debug, Clone)]
    struct CallSign(String);

    #[derive(Copy, Clone)]
    struct AZeroSizedType;

    #[test]
    fn cloning_works_with_basic_types() {
        // copyable types
        let int0 = 0;
        let int1 = 1;
        let p0 = Position([0.0, 0.0]);
        let p1 = Position([1.0, 1.0]);
        // cloneable types
        let str0 = "Ada".to_owned();
        let str1 = "Bob".to_owned();
        let n0 = CallSign("Zebra".into());
        let n1 = CallSign("Yankee".into());

        let mut world0 = World::new();
        let entity0 = world0.spawn((int0, p0, str0, n0));
        let entity1 = world0.spawn((int1, p1, str1, n1));

        let mut cloner = Cloner::new();
        cloner.add_copyable::<i32>();
        cloner.add_copyable::<Position>();
        cloner.add_cloneable::<String>();
        cloner.add_cloneable::<CallSign>();

        let world1 = world0.try_clone(&cloner).expect("clone should succeed");

        assert_eq!(
            world0.len(),
            world1.len(),
            "cloned world should have same entity count as original world"
        );

        type AllComponentsQuery = (
            &'static i32,
            &'static Position,
            &'static String,
            &'static CallSign,
        );

        for entity in [entity0, entity1] {
            let w0_e = world0.entity(entity).expect("w0 entity should exist");
            let w1_e = world1.entity(entity).expect("w1 entity should exist");
            assert!(w0_e.satisfies::<AllComponentsQuery>());
            assert!(w1_e.satisfies::<AllComponentsQuery>());

            assert_eq!(
                w0_e.query::<AllComponentsQuery>().get().unwrap(),
                w1_e.query::<AllComponentsQuery>().get().unwrap()
            );
        }
    }

    #[test]
    fn cloning_works_with_zero_sized_types() {
        let mut world0 = World::new();
        let entity_zst_only = world0.spawn((AZeroSizedType,));
        let entity_mixed = world0.spawn(("John".to_owned(), AZeroSizedType));

        let mut cloner = Cloner::new();
        // (Zero sized type does not need to be registered, as it never needs to actually be cloned)
        cloner.add_cloneable::<String>();

        let world1 = world0.try_clone(&cloner).expect("clone should succeed");

        assert!(world1
            .entity(entity_zst_only)
            .expect("entity should exist in cloned world")
            .has::<AZeroSizedType>());
        assert!(world1
            .entity(entity_mixed)
            .expect("entity should exist in cloned world")
            .satisfies::<(&String, &AZeroSizedType)>());
    }

    #[test]
    fn cloning_gives_identical_entity_ids() {
        // This test ensures that a cloned world's spawned entity ids do not diverge from entity ids
        // created by the original world - i.e. that cloning does not break determinism.

        let mut world0 = World::new();
        let p0 = Position([1.0, 1.0]);

        // add & remove an entity to catch errors related to entities being given different ids
        let e0 = world0.spawn((p0.clone(),));
        let _e1 = world0.spawn((p0,));
        world0.despawn(e0).expect("despawn should succeed");

        let mut cloner = Cloner::new();
        cloner.add_cloneable::<CallSign>();
        cloner.add_copyable::<Position>();

        let mut world1 = world0.try_clone(&cloner).expect("clone should succeed");

        let world0_e2 = world0.spawn((p0,));
        let world1_e2 = world1.spawn((p0,));
        assert_eq!(
            world0_e2, world1_e2,
            "entity id for two worlds should be equal for newly spawned entity"
        );
    }

    #[test]
    fn cloner_having_unused_types_registered_is_okay() {
        let mut world0 = World::new();
        world0.spawn((1,));

        let mut cloner = Cloner::new();
        cloner.add_copyable::<i32>();
        cloner.add_cloneable::<String>(); // unused type

        let world1 = world0.try_clone(&cloner).unwrap();
        assert_eq!(world0.len(), world1.len());
    }

    #[test]
    fn cloner_can_be_reused() {
        let mut world0 = World::new();
        world0.spawn((1,));

        let mut cloner = Cloner::new();
        cloner.add_copyable::<i32>();

        let world1 = world0.try_clone(&cloner).unwrap();
        let mut world2 = world0.try_clone(&cloner).unwrap();
        let world3 = world2.try_clone(&cloner).unwrap();

        for cloned in [world1, world2, world3] {
            assert_eq!(world0.len(), cloned.len());
        }
    }

    #[test]
    fn unknown_type_is_reported() {
        let mut world0 = World::new();
        world0.spawn((Position([1.0, 1.0]),));

        let cloner = Cloner::new();

        match world0.try_clone(&cloner) {
            Ok(_) => {
                panic!("cloning should have failed because Position was not registered with Cloner")
            }
            Err(err) => {
                #[cfg(debug_assertions)]
                assert!(err.type_name.contains("Position"));
            }
        };
    }
}
