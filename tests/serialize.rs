//! Round-trip and malformed-input properties for `hecs::serialize::{row, column}`.
//!
//! Both formats reinstate entities with `spawn_at`, so a round trip must
//! preserve the exact `Entity` handles (id and generation) as well as the
//! components. Malformed input is the other half: these are the only hecs entry
//! points that take untrusted bytes, so the properties here assert that a
//! stream that is not a valid serialization is rejected rather than accepted or
//! panicked on, and that a rejected stream does not leak the components it had
//! already parsed. `D` is drop-tracked and deserialized through its own
//! constructor, which is what makes the leak half checkable.

use std::any::TypeId;
use std::panic::{catch_unwind, AssertUnwindSafe};

use bincode::Options;
use fixtures::*;
use hecs::serialize::{column, row};
use hecs::{
    Archetype, ColumnBatchBuilder, ColumnBatchType, Entity, EntityBuilder, EntityRef, Query, World,
};
use hegel::generators as gs;
use serde::{Deserialize, Serialize};

/// Wire identifiers for the component universe. The bincode encoding of these
/// discriminants is `A = 0 .. D = 3`, so anything above 3 is an unknown-variant
/// error on the way back in.
#[derive(Clone, Copy, Serialize, Deserialize)]
enum Id {
    A,
    B,
    C,
    D,
}

// ---- row format ----

struct Row;

impl row::SerializeContext for Row {
    fn serialize_entity<S>(&mut self, entity: EntityRef<'_>, mut map: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::SerializeMap,
    {
        row::try_serialize::<A, _, _>(&entity, &Id::A, &mut map)?;
        row::try_serialize::<B, _, _>(&entity, &Id::B, &mut map)?;
        row::try_serialize::<C, _, _>(&entity, &Id::C, &mut map)?;
        row::try_serialize::<D, _, _>(&entity, &Id::D, &mut map)?;
        map.end()
    }

    /// Length-prefixed formats such as bincode need this.
    fn component_count(&self, entity: EntityRef<'_>) -> Option<usize> {
        Some(entity.len())
    }
}

impl row::DeserializeContext for Row {
    fn deserialize_entity<'de, M>(
        &mut self,
        mut map: M,
        entity: &mut EntityBuilder,
    ) -> Result<(), M::Error>
    where
        M: serde::de::MapAccess<'de>,
    {
        while let Some(key) = map.next_key()? {
            match key {
                Id::A => entity.add::<A>(map.next_value()?),
                Id::B => entity.add::<B>(map.next_value()?),
                Id::C => entity.add::<C>(map.next_value()?),
                Id::D => entity.add::<D>(map.next_value()?),
            };
        }
        Ok(())
    }
}

fn row_bytes(world: &World) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut ser = bincode::Serializer::new(&mut buf, bincode::options());
    row::serialize(world, &mut Row, &mut ser).expect("row serialize");
    buf
}

fn row_satisfying_bytes<Q: Query>(world: &World) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut ser = bincode::Serializer::new(&mut buf, bincode::options());
    row::serialize_satisfying::<Q, _, _>(world, &mut Row, &mut ser)
        .expect("row serialize_satisfying");
    buf
}

fn row_world(bytes: &[u8]) -> Result<World, bincode::Error> {
    let mut de = bincode::Deserializer::with_reader(bytes, bincode::options());
    row::deserialize(&mut Row, &mut de)
}

// ---- column format ----

struct ColumnSer;

impl column::SerializeContext for ColumnSer {
    fn component_count(&self, archetype: &Archetype) -> usize {
        archetype
            .component_types()
            .filter(|&t| {
                t == TypeId::of::<A>()
                    || t == TypeId::of::<B>()
                    || t == TypeId::of::<C>()
                    || t == TypeId::of::<D>()
            })
            .count()
    }

    fn serialize_component_ids<S: serde::ser::SerializeTuple>(
        &mut self,
        archetype: &Archetype,
        mut out: S,
    ) -> Result<S::Ok, S::Error> {
        column::try_serialize_id::<A, _, _>(archetype, &Id::A, &mut out)?;
        column::try_serialize_id::<B, _, _>(archetype, &Id::B, &mut out)?;
        column::try_serialize_id::<C, _, _>(archetype, &Id::C, &mut out)?;
        column::try_serialize_id::<D, _, _>(archetype, &Id::D, &mut out)?;
        out.end()
    }

    fn serialize_components<S: serde::ser::SerializeTuple>(
        &mut self,
        archetype: &Archetype,
        mut out: S,
    ) -> Result<S::Ok, S::Error> {
        column::try_serialize::<A, _>(archetype, &mut out)?;
        column::try_serialize::<B, _>(archetype, &mut out)?;
        column::try_serialize::<C, _>(archetype, &mut out)?;
        column::try_serialize::<D, _>(archetype, &mut out)?;
        out.end()
    }
}

#[derive(Default)]
struct ColumnDe {
    components: Vec<Id>,
}

impl column::DeserializeContext for ColumnDe {
    fn deserialize_component_ids<'de, S>(&mut self, mut seq: S) -> Result<ColumnBatchType, S::Error>
    where
        S: serde::de::SeqAccess<'de>,
    {
        self.components.clear();
        let mut batch = ColumnBatchType::new();
        while let Some(id) = seq.next_element()? {
            match id {
                Id::A => batch.add::<A>(),
                Id::B => batch.add::<B>(),
                Id::C => batch.add::<C>(),
                Id::D => batch.add::<D>(),
            };
            self.components.push(id);
        }
        Ok(batch)
    }

    fn deserialize_components<'de, S>(
        &mut self,
        entity_count: u32,
        mut seq: S,
        batch: &mut ColumnBatchBuilder,
    ) -> Result<(), S::Error>
    where
        S: serde::de::SeqAccess<'de>,
    {
        for id in &self.components {
            match id {
                Id::A => column::deserialize_column::<A, _>(entity_count, &mut seq, batch)?,
                Id::B => column::deserialize_column::<B, _>(entity_count, &mut seq, batch)?,
                Id::C => column::deserialize_column::<C, _>(entity_count, &mut seq, batch)?,
                Id::D => column::deserialize_column::<D, _>(entity_count, &mut seq, batch)?,
            }
        }
        Ok(())
    }
}

fn column_bytes(world: &World) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut ser = bincode::Serializer::new(&mut buf, bincode::options());
    column::serialize(world, &mut ColumnSer, &mut ser).expect("column serialize");
    buf
}

fn column_satisfying_bytes<Q: Query>(world: &World) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut ser = bincode::Serializer::new(&mut buf, bincode::options());
    column::serialize_satisfying::<Q, _, _>(world, &mut ColumnSer, &mut ser)
        .expect("column serialize_satisfying");
    buf
}

fn column_world(bytes: &[u8]) -> Result<World, bincode::Error> {
    let mut de = bincode::Deserializer::with_reader(bytes, bincode::options());
    column::deserialize(&mut ColumnDe::default(), &mut de)
}

/// bincode-encode one value with the options used everywhere here, for
/// hand-writing malformed streams.
fn encode<T: Serialize>(value: &T) -> Vec<u8> {
    bincode::options().serialize(value).expect("encode")
}

/// Round trips want enough entities to spread over many archetypes. Under Miri
/// the serde machinery is by far the slowest thing in this suite, so the bound
/// there drops to the smallest size that still makes the handle-preservation
/// claim non-trivial.
#[cfg(miri)]
const WORLD_SIZE: u32 = 3;
#[cfg(not(miri))]
const WORLD_SIZE: u32 = 24;

#[hegel::test(settings())]
fn row_format_roundtrips(tc: hegel::TestCase) {
    assert_d_balanced_at_start();
    let history = tc.draw(histories(0, WORLD_SIZE));
    let world = build_world(&history);
    let restored = row_world(&row_bytes(&world)).expect("row deserialize");
    assert_eq!(
        fingerprint(&world),
        fingerprint(&restored),
        "row round trip"
    );
}

#[hegel::test(settings())]
fn column_format_roundtrips(tc: hegel::TestCase) {
    assert_d_balanced_at_start();
    let history = tc.draw(histories(0, WORLD_SIZE));
    let world = build_world(&history);
    let restored = column_world(&column_bytes(&world)).expect("column deserialize");
    assert_eq!(
        fingerprint(&world),
        fingerprint(&restored),
        "column round trip"
    );
}

/// `serialize_satisfying::<Q>` writes exactly the entities matching `Q`, and
/// writes all of their components rather than only the ones `Q` mentions.
#[hegel::test(settings())]
fn row_serialize_satisfying_keeps_exactly_the_matching_entities(tc: hegel::TestCase) {
    assert_d_balanced_at_start();
    let history = tc.draw(histories(0, WORLD_SIZE));
    let world = build_world(&history);
    let restored = row_world(&row_satisfying_bytes::<&A>(&world)).expect("row deserialize");
    let want: Fingerprint = fingerprint(&world)
        .into_iter()
        .filter(|(_, s)| s.a.is_some())
        .collect();
    assert_eq!(
        want,
        fingerprint(&restored),
        "row serialize_satisfying::<&A>"
    );
}

#[hegel::test(settings())]
fn column_serialize_satisfying_keeps_exactly_the_matching_entities(tc: hegel::TestCase) {
    assert_d_balanced_at_start();
    let history = tc.draw(histories(0, WORLD_SIZE));
    let world = build_world(&history);
    let restored =
        column_world(&column_satisfying_bytes::<&A>(&world)).expect("column deserialize");
    let want: Fingerprint = fingerprint(&world)
        .into_iter()
        .filter(|(_, s)| s.a.is_some())
        .collect();
    assert_eq!(
        want,
        fingerprint(&restored),
        "column serialize_satisfying::<&A>"
    );
}

/// A strict prefix of a valid stream is rejected, and nothing the failed parse
/// had already built survives. bincode parsing is deterministic, so a prefix
/// follows the full parse byte for byte until it runs out of input.
#[hegel::test(settings())]
fn row_truncated_input_is_rejected_without_leaking(tc: hegel::TestCase) {
    assert_d_balanced_at_start();
    let history = tc.draw(histories(0, WORLD_SIZE));
    let world = build_world(&history);
    let bytes = row_bytes(&world);
    assert_eq!(
        fingerprint(&world),
        fingerprint(&row_world(&bytes).expect("row deserialize")),
        "row round trip"
    );
    let cut = tc.draw(
        gs::integers::<usize>()
            .min_value(0)
            .max_value(bytes.len() - 1),
    );
    let baseline = d_live();
    assert!(
        row_world(&bytes[..cut]).is_err(),
        "row prefix {cut}/{} parsed",
        bytes.len()
    );
    assert_eq!(
        d_live(),
        baseline,
        "row prefix {cut} leaked or double-dropped a component"
    );
}

/// The column counterpart. A failed column parse leaves the components it had
/// already read in a `ColumnBatchBuilder` that is then dropped; that builder
/// leaked them all (issue #450), so this is the regression witness for the
/// deserialize path.
#[hegel::test(settings())]
fn column_truncated_input_is_rejected_without_leaking(tc: hegel::TestCase) {
    assert_d_balanced_at_start();
    let history = tc.draw(histories(0, WORLD_SIZE));
    let world = build_world(&history);
    let bytes = column_bytes(&world);
    assert_eq!(
        fingerprint(&world),
        fingerprint(&column_world(&bytes).expect("column deserialize")),
        "column round trip"
    );
    let cut = tc.draw(
        gs::integers::<usize>()
            .min_value(0)
            .max_value(bytes.len() - 1),
    );
    let baseline = d_live();
    assert!(
        column_world(&bytes[..cut]).is_err(),
        "column prefix {cut}/{} parsed",
        bytes.len()
    );
    assert_eq!(
        d_live(),
        baseline,
        "column prefix {cut} leaked or double-dropped a component"
    );
}

/// One to three `(position, mask)` pairs flipping the low two bits of a byte in
/// a stream of `len` bytes. This still reaches unknown enum discriminants,
/// invalid entity bit patterns, colliding entity ids, bincode length-marker
/// mutations and corrupted payloads, while bounding every count and id the
/// parser can derive from the stream: both deserializers size allocations from
/// untrusted integers before validating them (row grows the entity metadata
/// table up to the deserialized id, column reserves from `entity_count`), so a
/// full-byte overwrite can ask for tens of gigabytes.
fn corruptions(len: usize) -> impl gs::PrintableGenerator<Vec<(usize, u8)>> {
    gs::vecs(hegel::tuples!(
        gs::integers::<usize>().min_value(0).max_value(len - 1),
        gs::integers::<u8>().min_value(1).max_value(3),
    ))
    .min_size(1)
    .max_size(3)
}

fn corrupt(bytes: &mut [u8], flips: &[(usize, u8)]) {
    for &(at, mask) in flips {
        bytes[at] ^= mask;
    }
}

/// Corrupting a stream must produce either an error or a usable world — never a
/// panic, and never a world with a duplicated handle. `Ok` is legitimate:
/// flipping bits of a payload byte yields a different valid stream.
///
/// Not run under Miri: a corrupted entity id makes `spawn_at` grow the metadata
/// table to that id, which is a few tens of megabytes at worst here but is far
/// beyond what the interpreter can track in reasonable time.
#[cfg_attr(miri, ignore = "corrupted ids reach allocations Miri cannot afford")]
#[hegel::test(settings())]
fn row_corrupted_input_is_rejected_or_usable(tc: hegel::TestCase) {
    assert_d_balanced_at_start();
    let history = tc.draw(histories(0, WORLD_SIZE));
    let world = build_world(&history);
    let mut bytes = row_bytes(&world);
    let flips = tc.draw(corruptions(bytes.len()));
    corrupt(&mut bytes, &flips);
    let outcome = catch_unwind(AssertUnwindSafe(|| row_world(&bytes)));
    match outcome {
        Ok(Ok(restored)) => drop(fingerprint(&restored)),
        Ok(Err(_)) => {}
        Err(_) => panic!("row deserialize panicked on a corrupted stream"),
    }
}

/// The column counterpart. Corrupting the entity-id region routinely produces
/// colliding ids, which used to reach an out-of-bounds write in
/// `spawn_column_batch_at` (issue #449), so this doubles as the regression
/// witness for that fix. Not run under Miri, for the same reason as its row
/// counterpart: `entity_count` drives a `reserve` before it is validated.
#[cfg_attr(miri, ignore = "corrupted counts reach allocations Miri cannot afford")]
#[hegel::test(settings())]
fn column_corrupted_input_is_rejected_or_usable(tc: hegel::TestCase) {
    assert_d_balanced_at_start();
    let history = tc.draw(histories(0, WORLD_SIZE));
    let world = build_world(&history);
    let mut bytes = column_bytes(&world);
    let flips = tc.draw(corruptions(bytes.len()));
    corrupt(&mut bytes, &flips);
    let outcome = catch_unwind(AssertUnwindSafe(|| column_world(&bytes)));
    match outcome {
        Ok(Ok(restored)) => drop(fingerprint(&restored)),
        Ok(Err(_)) => {}
        Err(_) => panic!("column deserialize panicked on a corrupted stream"),
    }
}

/// Streams that are well-formed bincode but not valid hecs serializations must
/// be rejected: an entity bit pattern with generation 0, and a component id
/// outside the context's enum.
#[test]
fn row_rejects_streams_it_cannot_represent() {
    let mut zero_generation = Vec::new();
    zero_generation.extend(encode(&1u64)); // one entity
    zero_generation.extend(encode(&0u64)); // entity bits: generation 0
    zero_generation.extend(encode(&0u64)); // no components
    assert!(
        row_world(&zero_generation).is_err(),
        "accepted a generation-0 entity"
    );

    let mut unknown_component = Vec::new();
    unknown_component.extend(encode(&1u64)); // one entity
    unknown_component.extend(encode(&(1u64 << 32))); // generation 1, id 0
    unknown_component.extend(encode(&1u64)); // one component
    unknown_component.extend(encode(&99u32)); // no such Id variant
    unknown_component.extend(encode(&0i32));
    assert!(
        row_world(&unknown_component).is_err(),
        "accepted an unknown component id"
    );
}

/// The column counterparts: an archetype that declares the same component
/// twice (the second column has nowhere to go, because `ColumnBatchType`
/// deduplicates the type), and an entity list shorter than `entity_count`.
#[test]
fn column_rejects_streams_it_cannot_represent() {
    let mut duplicate_column = Vec::new();
    duplicate_column.extend(encode(&1u64)); // one archetype
    duplicate_column.extend(encode(&1u32)); // entity_count
    duplicate_column.extend(encode(&2u32)); // component_count
    duplicate_column.extend(encode(&0u32)); // Id::A
    duplicate_column.extend(encode(&0u32)); // Id::A again
    duplicate_column.extend(encode(&(1u64 << 32))); // generation 1, id 0
    duplicate_column.extend(encode(&0i32)); // first A column
    duplicate_column.extend(encode(&1i32)); // second A column, no space left
    assert!(
        column_world(&duplicate_column).is_err(),
        "accepted a duplicated component column"
    );

    let mut short_entity_list = Vec::new();
    short_entity_list.extend(encode(&1u64)); // one archetype
    short_entity_list.extend(encode(&2u32)); // entity_count = 2
    short_entity_list.extend(encode(&0u32)); // component_count = 0
    short_entity_list.extend(encode(&(1u64 << 32))); // only one entity id
    assert!(
        column_world(&short_entity_list).is_err(),
        "accepted a short entity list"
    );
}

/// A column archetype whose entity-id list repeats an id is accepted, with the
/// last row for that id winning and the world left self-consistent — the same
/// semantics `spawn_column_batch_at_redundant` in src/world.rs pins. Before the
/// fix for issue #449 this reached `Archetype::remove(u32::MAX)`.
#[test]
fn column_deserialize_tolerates_repeated_entity_ids() {
    let bits = 1u64 << 32; // generation 1, id 0
    let mut bytes = Vec::new();
    bytes.extend(encode(&1u64)); // one archetype
    bytes.extend(encode(&2u32)); // entity_count = 2
    bytes.extend(encode(&1u32)); // component_count = 1
    bytes.extend(encode(&0u32)); // Id::A
    bytes.extend(encode(&bits));
    bytes.extend(encode(&bits)); // the same id twice
    bytes.extend(encode(&7i32));
    bytes.extend(encode(&9i32));

    let world = column_world(&bytes).expect("repeated ids are deduplicated, not rejected");
    let e = Entity::from_bits(bits).unwrap();
    assert_eq!(
        world.len(),
        1,
        "a repeated id produced more than one entity"
    );
    assert_eq!(
        fingerprint(&world).get(&e).and_then(|s| s.a),
        Some(9),
        "the last row must win"
    );
    check_archetypes(&world, "deduplicated world");
}
