//! This example demonstrates how to save [hecs::World] instance
//! to disk and load it back to memory using serialization. It can be useful for implementing
//! a save mechanism for a game.
//!
//! The example creates a sample `World`, serializes it, saves it to disk as `saved_world.world` file,
//! loads it back from the disk, deserializes the loaded world data and validates that component
//! data of the worlds match.
//!
//! Run this example from crate root with:
//! `cargo run --example serialize_to_disk --features "column-serialize"`

#[cfg(feature = "column-serialize")]
mod serialize_to_disk_example {
    pub use hecs::serialize::column::{
        deserialize_column, try_serialize, try_serialize_id, DeserializeContext, SerializeContext,
    };
    pub use hecs::{Archetype, ColumnBatchBuilder, ColumnBatchType, World};
    pub use serde::{Deserialize, Serialize};
    use std::any::TypeId;
    pub use std::fs::File;
    pub use std::io::{BufReader, Write};
    pub use std::path::Path;

    // Identifiers for the components we want to include in the serialization process:
    #[derive(Serialize, Deserialize)]
    enum ComponentId {
        TestComponent1,
        TestComponent2,
    }

    // We need to implement a context type for the hecs serialization process:
    #[derive(Default)]
    pub struct SaveContext {
        pub components: Vec<ComponentId>,
    }

    // Components of our world.
    // Only Serialize and Deserialize derives are necessary.
    #[derive(Serialize, Deserialize, Eq, PartialEq, Debug, Clone)]
    pub struct ComponentA {
        pub data: usize,
    }

    #[derive(Serialize, Deserialize, Eq, PartialEq, Debug, Clone)]
    pub struct ComponentB {
        pub some_other_data: String,
    }

    #[cfg(feature = "column-serialize")]
    impl DeserializeContext for SaveContext {
        fn deserialize_component_ids<'de, A>(
            &mut self,
            mut seq: A,
        ) -> Result<ColumnBatchType, A::Error>
        where
            A: serde::de::SeqAccess<'de>,
        {
            self.components.clear(); // Discard data from the previous archetype
            let mut batch = ColumnBatchType::new();
            while let Some(id) = seq.next_element()? {
                match id {
                    ComponentId::TestComponent1 => {
                        batch.add::<ComponentA>();
                    }
                    ComponentId::TestComponent2 => {
                        batch.add::<ComponentB>();
                    }
                }
                self.components.push(id);
            }
            Ok(batch)
        }

        fn deserialize_components<'de, A>(
            &mut self,
            entity_count: u32,
            mut seq: A,
            batch: &mut ColumnBatchBuilder,
        ) -> Result<(), A::Error>
        where
            A: serde::de::SeqAccess<'de>,
        {
            // Decode component data in the order that the component IDs appeared
            for component in &self.components {
                match *component {
                    ComponentId::TestComponent1 => {
                        deserialize_column::<ComponentA, _>(entity_count, &mut seq, batch)?;
                    }
                    ComponentId::TestComponent2 => {
                        deserialize_column::<ComponentB, _>(entity_count, &mut seq, batch)?;
                    }
                }
            }
            Ok(())
        }
    }

    impl SerializeContext for SaveContext {
        fn component_count(&self, archetype: &Archetype) -> usize {
            archetype
                .component_types()
                .filter(|&t| t == TypeId::of::<ComponentA>() || t == TypeId::of::<ComponentB>())
                .count()
        }

        fn serialize_component_ids<S: serde::ser::SerializeTuple>(
            &mut self,
            archetype: &Archetype,
            mut out: S,
        ) -> Result<S::Ok, S::Error> {
            try_serialize_id::<ComponentA, _, _>(
                archetype,
                &ComponentId::TestComponent1,
                &mut out,
            )?;
            try_serialize_id::<ComponentB, _, _>(
                archetype,
                &ComponentId::TestComponent2,
                &mut out,
            )?;
            out.end()
        }

        fn serialize_components<S: serde::ser::SerializeTuple>(
            &mut self,
            archetype: &Archetype,
            mut out: S,
        ) -> Result<S::Ok, S::Error> {
            try_serialize::<ComponentA, _>(archetype, &mut out)?;
            try_serialize::<ComponentB, _>(archetype, &mut out)?;
            out.end()
        }
    }
}

use serialize_to_disk_example::*;
pub fn main() {
    // initialize world:
    let mut world = World::new();
    let input_data1 = ComponentA { data: 42 };
    let input_data2 = ComponentB {
        some_other_data: "Hello".to_string(),
    };
    world.spawn((input_data1.clone(),));
    world.spawn((input_data2.clone(),));

    let save_file_name = "saved_world.world";
    let mut context = SaveContext::default();

    // serialize and save our world to disk:
    let mut buffer: Vec<u8> = Vec::new();
    let mut serializer = serde_json::Serializer::new(buffer);
    hecs::serialize::column::serialize(&world, &mut context, &mut serializer);
    let path = Path::new(save_file_name);
    let mut file = match File::create(&path) {
        Err(why) => panic!("couldn't create {}: {}", path.display(), why),
        Ok(file) => file,
    };
    file.write(&serializer.into_inner())
        .expect(&format!("Failed to write file: {}", save_file_name));
    println!("Saved world \'{}\' to disk.", path.display());

    // load our world from disk and deserialize it back as world:
    let open = File::open(path).expect("not found!");
    let reader = BufReader::new(open);
    let mut deserializer = serde_json::Deserializer::from_reader(reader);
    match hecs::serialize::column::deserialize(&mut context, &mut deserializer) {
        Ok(world) => {
            // we loaded world from disk successfully, let us confirm that its data is still
            // the same:
            println!("Loaded world \'{}\' from disk.", path.display());

            print!("Validating world data... ");
            for (e, (t)) in &mut world.query::<(&ComponentA)>() {
                assert_eq!(t, &input_data1);
            }
            for (e, (t)) in &mut world.query::<(&ComponentB)>() {
                assert_eq!(t, &input_data2);
            }
            println!("Ok!");
        }
        Err(err) => {
            println!("Failed to deserialize world: {}", err);
        }
    }
}
