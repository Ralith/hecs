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

use hecs::serialize::column::{
    deserialize_column, try_serialize, try_serialize_id, DeserializeContext, SerializeContext,
};
use hecs::{Archetype, ColumnBatchBuilder, ColumnBatchType, World};
use serde::{Deserialize, Serialize};
use std::any::TypeId;
use std::fs::File;
use std::io::{BufReader, Write};
use std::path::Path;

// Identifiers for the components we want to include in the serialization process:
#[derive(Serialize, Deserialize)]
enum ComponentId {
    ComponentA,
    ComponentB,
}

// We need to implement context types for the hecs serialization process:
#[derive(Default)]
struct SaveContextSerialize {}
#[derive(Default)]
struct SaveContextDeserialize {
    components: Vec<ComponentId>,
}

// Components of our world.
// Only Serialize and Deserialize derives are necessary.
#[derive(Serialize, Deserialize, Eq, PartialEq, Debug, Clone)]
struct ComponentA {
    data: usize,
}

#[derive(Serialize, Deserialize, Eq, PartialEq, Debug, Clone)]
struct ComponentB {
    some_other_data: String,
}

impl DeserializeContext for SaveContextDeserialize {
    fn deserialize_component_ids<'de, A>(&mut self, mut seq: A) -> Result<ColumnBatchType, A::Error>
    where
        A: serde::de::SeqAccess<'de>,
    {
        self.components.clear(); // Discard data from the previous archetype
        let mut batch = ColumnBatchType::new();
        while let Some(id) = seq.next_element()? {
            match id {
                ComponentId::ComponentA => {
                    batch.add::<ComponentA>();
                }
                ComponentId::ComponentB => {
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
                ComponentId::ComponentA => {
                    deserialize_column::<ComponentA, _>(entity_count, &mut seq, batch)?;
                }
                ComponentId::ComponentB => {
                    deserialize_column::<ComponentB, _>(entity_count, &mut seq, batch)?;
                }
            }
        }
        Ok(())
    }
}

impl SerializeContext for SaveContextSerialize {
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
        try_serialize_id::<ComponentA, _, _>(archetype, &ComponentId::ComponentA, &mut out)?;
        try_serialize_id::<ComponentB, _, _>(archetype, &ComponentId::ComponentB, &mut out)?;
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

fn main() {
    // initialize world:
    let mut world = World::new();
    let input_data1 = ComponentA { data: 42 };
    let input_data2 = ComponentB {
        some_other_data: "Hello".to_string(),
    };
    world.spawn((input_data1.clone(),));
    world.spawn((input_data2.clone(),));

    let save_file_name = "saved_world.world";

    // serialize and save our world to disk:
    let mut buffer: Vec<u8> = Vec::new();
    let options = bincode::options();
    let mut serializer = bincode::Serializer::new(&mut buffer, options);
    hecs::serialize::column::serialize(
        &world,
        &mut SaveContextSerialize::default(),
        &mut serializer,
    )
    .expect("Failed to serialize");
    let path = Path::new(save_file_name);
    let mut file = match File::create(path) {
        Err(why) => panic!("couldn't create {}: {}", path.display(), why),
        Ok(file) => file,
    };
    file.write(&buffer)
        .unwrap_or_else(|_| panic!("Failed to write file: {save_file_name}"));
    println!("Saved world \'{}\' to disk.", path.display());

    // load our world from disk and deserialize it back as world:
    let open = File::open(path).expect("not found!");
    let reader = BufReader::new(open);
    let mut deserializer = bincode::Deserializer::with_reader(reader, options);
    match hecs::serialize::column::deserialize(
        &mut SaveContextDeserialize::default(),
        &mut deserializer,
    ) {
        Ok(world) => {
            // we loaded world from disk successfully, let us confirm that its data is still
            // the same:
            println!("Loaded world \'{}\' from disk.", path.display());
            print!("Validating world data... ");
            assert_eq!(world.len(), 2);
            for (_, t) in &mut world.query::<&ComponentA>() {
                assert_eq!(t, &input_data1);
            }
            for (_, t) in &mut world.query::<&ComponentB>() {
                assert_eq!(t, &input_data2);
            }
            println!("Ok!");
        }
        Err(err) => {
            println!("Failed to deserialize world: {err}");
        }
    }
}
