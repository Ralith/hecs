use core::any::TypeId;
use std::ops::{Deref, DerefMut};

use hecs::{Archetype, ColumnBatchBuilder, ColumnBatchType, Component, World};

fn main() {
    let mut world = CloneableWorld {
        inner: World::new(),
        clone_registry: CloneRegistry::default()
            .register::<String>()
            .register::<u32>(),
    };

    world.spawn((4u32,));
    world.spawn((8u32, "test".to_string()));
    world.spawn(("test".to_string(),));

    let cloned = world.clone();

    assert_eq!(
        world.query::<&u32>().iter().count(),
        cloned.query::<&u32>().iter().count()
    );
    assert_eq!(
        world.query::<&String>().iter().count(),
        cloned.query::<&String>().iter().count()
    );

    for (left, right) in world
        .query::<&u32>()
        .iter()
        .zip(cloned.query::<&u32>().iter())
    {
        assert_eq!(left, right);
    }

    for (left, right) in world
        .query::<&String>()
        .iter()
        .zip(cloned.query::<&String>().iter())
    {
        assert_eq!(left, right);
    }
}

/// An opaque registry that holds data that helps a World clone itself.
#[derive(Clone, Default)]
struct CloneRegistry(Vec<CloneEntry>);

impl CloneRegistry {
    /// Registers `T` with the registry, enabling `T` to be cloned in any
    /// archetypes that contain it.
    pub fn register<T: Clone + Component>(mut self) -> Self {
        if !self.0.iter().any(|item| item.type_id == TypeId::of::<T>()) {
            self.0.push(register::<T>());
        }
        self
    }
}

#[derive(Clone)]
struct CloneEntry {
    type_id: TypeId,
    add_type: fn(&mut ColumnBatchType) -> (),
    add_values: fn(&mut ColumnBatchBuilder, &Archetype) -> (),
}
fn register<T: Component + Clone>() -> CloneEntry {
    CloneEntry {
        type_id: TypeId::of::<T>(),
        add_type: |batch_type| {
            batch_type.add::<T>();
        },
        add_values: |batch, arch| {
            let mut writer = batch.writer::<T>().unwrap();
            for item in arch.get::<T>().unwrap().iter() {
                if writer.push(item.clone()).is_err() {
                    panic!()
                }
            }
        },
    }
}

pub struct CloneableWorld {
    inner: World,
    clone_registry: CloneRegistry,
}

impl Deref for CloneableWorld {
    type Target = World;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
impl DerefMut for CloneableWorld {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl Clone for CloneableWorld {
    fn clone(&self) -> Self {
        let mut new_world = Self {
            inner: World::new(),
            clone_registry: self.clone_registry.clone(),
        };

        for archetype in self.archetypes() {
            debug_assert!(archetype.component_types().all(|item| self
                .clone_registry
                .0
                .iter()
                .any(|register| register.type_id == item)));
            let mut types = ColumnBatchType::new();
            for entry in self
                .clone_registry
                .0
                .iter()
                .filter(|item| archetype.has_dynamic(item.type_id))
            {
                (entry.add_type)(&mut types);
            }
            let mut batch = types.into_batch(archetype.len());
            for entry in self
                .clone_registry
                .0
                .iter()
                .filter(|item| archetype.has_dynamic(item.type_id))
            {
                (entry.add_values)(&mut batch, archetype);
            }
            let entities: Box<[_]> = archetype
                .ids()
                .iter()
                .map(|id| unsafe { self.find_entity_from_id(*id) })
                .collect();
            new_world.spawn_column_batch_at(&entities, batch.build().unwrap());
        }

        new_world
    }
}
