use alloc::vec::Vec;
use core::cmp;
use core::convert::TryFrom;
use core::iter::ExactSizeIterator;
use core::num::{NonZeroU32, NonZeroU64};
use core::ops::Range;
use core::sync::atomic::{AtomicIsize, Ordering};
use core::{fmt, mem};
#[cfg(feature = "std")]
use std::error::Error;

/// Lightweight unique ID, or handle, of an entity
///
/// Obtained from `World::spawn`. Can be stored to refer to an entity in the future.
///
/// Enable the `serde` feature on the crate to make this `Serialize`able. Some applications may be
/// able to save space by only serializing the output of `Entity::id`.
#[derive(Clone, Copy, Hash, Eq, Ord, PartialEq, PartialOrd)]
pub struct Entity {
    pub(crate) generation: NonZeroU32,
    pub(crate) id: u32,
}

impl Entity {
    /// Convert to a form convenient for passing outside of rust
    ///
    /// No particular structure is guaranteed for the returned bits.
    ///
    /// Useful for storing entity IDs externally, or in conjunction with `Entity::from_bits` and
    /// `World::spawn_at` for easy serialization. Alternatively, consider `id` for more compact
    /// representation.
    pub fn to_bits(self) -> NonZeroU64 {
        unsafe {
            NonZeroU64::new_unchecked(u64::from(self.generation.get()) << 32 | u64::from(self.id))
        }
    }

    /// Reconstruct an `Entity` previously destructured with `to_bits` if the bitpattern is valid,
    /// else `None`
    ///
    /// Useful for storing entity IDs externally, or in conjunction with `Entity::to_bits` and
    /// `World::spawn_at` for easy serialization.
    pub fn from_bits(bits: u64) -> Option<Self> {
        Some(Self {
            generation: NonZeroU32::new((bits >> 32) as u32)?,
            id: bits as u32,
        })
    }

    /// Extract a transiently unique identifier
    ///
    /// No two simultaneously-live entities share the same ID, but dead entities' IDs may collide
    /// with both live and dead entities. Useful for compactly representing entities within a
    /// specific snapshot of the world, such as when serializing.
    ///
    /// See also `World::find_entity_from_id`.
    pub fn id(self) -> u32 {
        self.id
    }
}

impl fmt::Debug for Entity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}v{}", self.id, self.generation)
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for Entity {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.to_bits().serialize(serializer)
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Entity {
    fn deserialize<D>(deserializer: D) -> Result<Entity, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let bits = u64::deserialize(deserializer)?;

        match Entity::from_bits(bits) {
            Some(ent) => Ok(ent),
            None => Err(serde::de::Error::invalid_value(
                serde::de::Unexpected::Unsigned(bits),
                &"`a valid `Entity` bitpattern",
            )),
        }
    }
}

/// An iterator returning a sequence of Entity values from `Entities::reserve_entities`.
pub struct ReserveEntitiesIterator<'a> {
    // Metas, so we can recover the current generation for anything in the freelist.
    meta: &'a [EntityMeta],

    // Reserved IDs formerly in the freelist to hand out.
    id_iter: core::slice::Iter<'a, u32>,

    // New Entity IDs to hand out, outside the range of meta.len().
    id_range: core::ops::Range<u32>,
}

impl<'a> Iterator for ReserveEntitiesIterator<'a> {
    type Item = Entity;

    fn next(&mut self) -> Option<Self::Item> {
        self.id_iter
            .next()
            .map(|&id| Entity {
                generation: self.meta[id as usize].generation,
                id,
            })
            .or_else(|| {
                self.id_range.next().map(|id| Entity {
                    generation: NonZeroU32::new(1).unwrap(),
                    id,
                })
            })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.id_iter.len() + self.id_range.len();
        (len, Some(len))
    }
}

impl<'a> ExactSizeIterator for ReserveEntitiesIterator<'a> {}

#[derive(Default)]
pub(crate) struct Entities {
    pub meta: Vec<EntityMeta>,

    // The `pending` and `free_cursor` fields describe three sets of Entity IDs
    // that have been freed or are in the process of being allocated:
    //
    // - The `freelist` IDs, previously freed by `free()`. These IDs are available to any
    //   of `alloc()`, `reserve_entity()` or `reserve_entities()`. Allocation will
    //   always prefer these over brand new IDs.
    //
    // - The `reserved` list of IDs that were once in the freelist, but got
    //   reserved by `reserve_entities` or `reserve_entity()`. They are now waiting
    //   for `flush()` to make them fully allocated.
    //
    // - The count of new IDs that do not yet exist in `self.meta()`, but which
    //   we have handed out and reserved. `flush()` will allocate room for them in `self.meta()`.
    //
    // The contents of `pending` look like this:
    //
    // ```
    // ----------------------------
    // |  freelist  |  reserved   |
    // ----------------------------
    //              ^             ^
    //          free_cursor   pending.len()
    // ```
    //
    // As IDs are allocated, `free_cursor` is atomically decremented, moving
    // items from the freelist into the reserved list by sliding over the boundary.
    //
    // Once the freelist runs out, `free_cursor` starts going negative.
    // The more negative it is, the more IDs have been reserved starting exactly at
    // the end of `meta.len()`.
    //
    // This formulation allows us to reserve any number of IDs first from the freelist
    // and then from the new IDs, using only a single atomic subtract.
    //
    // Once `flush()` is done, `free_cursor` will equal `pending.len()`.
    pending: Vec<u32>,
    free_cursor: AtomicIsize,
    len: u32,
}

impl Entities {
    /// Reserve entity IDs concurrently
    ///
    /// Storage for entity generation and location is lazily allocated by calling `flush`.
    pub fn reserve_entities(&self, count: u32) -> ReserveEntitiesIterator {
        // Use one atomic subtract to grab a range of new IDs. The range might be
        // entirely nonnegative, meaning all IDs come from the freelist, or entirely
        // negative, meaning they are all new IDs to allocate, or a mix of both.
        let range_end = self
            .free_cursor
            .fetch_sub(count as isize, Ordering::Relaxed);
        let range_start = range_end - count as isize;

        let freelist_range = range_start.max(0) as usize..range_end.max(0) as usize;

        let (new_id_start, new_id_end) = if range_start >= 0 {
            // We satisfied all requests from the freelist.
            (0, 0)
        } else {
            // We need to allocate some new Entity IDs outside of the range of self.meta.
            //
            // `range_start` covers some negative territory, e.g. `-3..6`.
            // Since the nonnegative values `0..6` are handled by the freelist, that
            // means we need to handle the negative range here.
            //
            // In this example, we truncate the end to 0, leaving us with `-3..0`.
            // Then we negate these values to indicate how far beyond the end of `meta.end()`
            // to go, yielding `meta.len()+0 .. meta.len()+3`.
            let base = self.meta.len() as isize;

            let new_id_end = u32::try_from(base - range_start).expect("too many entities");

            // `new_id_end` is in range, so no need to check `start`.
            let new_id_start = (base - range_end.min(0)) as u32;

            (new_id_start, new_id_end)
        };

        ReserveEntitiesIterator {
            meta: &self.meta[..],
            id_iter: self.pending[freelist_range].iter(),
            id_range: new_id_start..new_id_end,
        }
    }

    /// Reserve one entity ID concurrently
    ///
    /// Equivalent to `self.reserve_entities(1).next().unwrap()`, but more efficient.
    pub fn reserve_entity(&self) -> Entity {
        let n = self.free_cursor.fetch_sub(1, Ordering::Relaxed);
        if n > 0 {
            // Allocate from the freelist.
            let id = self.pending[(n - 1) as usize];
            Entity {
                generation: self.meta[id as usize].generation,
                id,
            }
        } else {
            // Grab a new ID, outside the range of `meta.len()`. `flush()` must
            // eventually be called to make it valid.
            //
            // As `self.free_cursor` goes more and more negative, we return IDs farther
            // and farther beyond `meta.len()`.
            Entity {
                generation: NonZeroU32::new(1).unwrap(),
                id: u32::try_from(self.meta.len() as isize - n).expect("too many entities"),
            }
        }
    }

    /// Check that we do not have pending work requiring `flush()` to be called.
    fn verify_flushed(&mut self) {
        debug_assert!(
            !self.needs_flush(),
            "flush() needs to be called before this operation is legal"
        );
    }

    /// Allocate an entity ID directly
    ///
    /// Location should be written immediately.
    pub fn alloc(&mut self) -> Entity {
        self.verify_flushed();

        self.len += 1;
        if let Some(id) = self.pending.pop() {
            let new_free_cursor = self.pending.len() as isize;
            self.free_cursor.store(new_free_cursor, Ordering::Relaxed); // Not racey due to &mut self
            Entity {
                generation: self.meta[id as usize].generation,
                id,
            }
        } else {
            let id = u32::try_from(self.meta.len()).expect("too many entities");
            self.meta.push(EntityMeta::EMPTY);
            Entity {
                generation: NonZeroU32::new(1).unwrap(),
                id,
            }
        }
    }

    /// Allocate and set locations for many entity IDs laid out contiguously in an archetype
    ///
    /// `self.finish_alloc_many()` must be called after!
    pub fn alloc_many(&mut self, n: u32, archetype: u32, mut first_index: u32) -> AllocManyState {
        self.verify_flushed();

        let fresh = (n as usize).saturating_sub(self.pending.len()) as u32;
        assert!(
            (self.meta.len() + fresh as usize) < u32::MAX as usize,
            "too many entities"
        );
        let pending_end = self.pending.len().saturating_sub(n as usize);
        for &id in &self.pending[pending_end..] {
            self.meta[id as usize].location = Location {
                archetype,
                index: first_index,
            };
            first_index += 1;
        }

        let fresh_start = self.meta.len() as u32;
        self.meta.extend(
            (first_index..(first_index + fresh)).map(|index| EntityMeta {
                generation: NonZeroU32::new(1).unwrap(),
                location: Location { archetype, index },
            }),
        );

        self.len += n;

        AllocManyState {
            fresh: fresh_start..(fresh_start + fresh),
            pending_end,
        }
    }

    /// Remove entities used by `alloc_many` from the freelist
    ///
    /// This is an awkward separate function to avoid borrowck issues in `SpawnColumnBatchIter`.
    pub fn finish_alloc_many(&mut self, pending_end: usize) {
        self.pending.truncate(pending_end);
    }

    /// Allocate a specific entity ID, overwriting its generation
    ///
    /// Returns the location of the entity currently using the given ID, if any. Location should be written immediately.
    pub fn alloc_at(&mut self, entity: Entity) -> Option<Location> {
        self.verify_flushed();

        let loc = if entity.id as usize >= self.meta.len() {
            self.pending.extend((self.meta.len() as u32)..entity.id);
            let new_free_cursor = self.pending.len() as isize;
            self.free_cursor.store(new_free_cursor, Ordering::Relaxed); // Not racey due to &mut self
            self.meta.resize(entity.id as usize + 1, EntityMeta::EMPTY);
            self.len += 1;
            None
        } else if let Some(index) = self.pending.iter().position(|item| *item == entity.id) {
            self.pending.swap_remove(index);
            let new_free_cursor = self.pending.len() as isize;
            self.free_cursor.store(new_free_cursor, Ordering::Relaxed); // Not racey due to &mut self
            self.len += 1;
            None
        } else {
            Some(mem::replace(
                &mut self.meta[entity.id as usize].location,
                EntityMeta::EMPTY.location,
            ))
        };

        self.meta[entity.id as usize].generation = entity.generation;

        loc
    }

    /// Destroy an entity, allowing it to be reused
    ///
    /// Must not be called while reserved entities are awaiting `flush()`.
    pub fn free(&mut self, entity: Entity) -> Result<Location, NoSuchEntity> {
        self.verify_flushed();

        let meta = self.meta.get_mut(entity.id as usize).ok_or(NoSuchEntity)?;
        if meta.generation != entity.generation {
            return Err(NoSuchEntity);
        }

        meta.generation = NonZeroU32::new(u32::from(meta.generation).wrapping_add(1))
            .unwrap_or_else(|| NonZeroU32::new(1).unwrap());

        let loc = mem::replace(&mut meta.location, EntityMeta::EMPTY.location);

        self.pending.push(entity.id);

        let new_free_cursor = self.pending.len() as isize;
        self.free_cursor.store(new_free_cursor, Ordering::Relaxed); // Not racey due to &mut self
        self.len -= 1;

        Ok(loc)
    }

    /// Ensure at least `n` allocations can succeed without reallocating
    pub fn reserve(&mut self, additional: u32) {
        self.verify_flushed();

        let freelist_size = self.free_cursor.load(Ordering::Relaxed);
        let shortfall = additional as isize - freelist_size;
        if shortfall > 0 {
            self.meta.reserve(shortfall as usize);
        }
    }

    pub fn contains(&self, entity: Entity) -> bool {
        // Note that out-of-range IDs are considered to be "contained" because
        // they must be reserved IDs that we haven't flushed yet.
        match self.meta.get(entity.id as usize) {
            Some(meta) => meta.generation == entity.generation,
            None => {
                // Check if this could have been obtained from `reserve_entity`
                let free = self.free_cursor.load(Ordering::Relaxed);
                entity.generation.get() == 1
                    && free < 0
                    && (entity.id as isize) < (free.abs() + self.meta.len() as isize)
            }
        }
    }

    pub fn clear(&mut self) {
        self.meta.clear();
        self.pending.clear();
        self.free_cursor.store(0, Ordering::Relaxed); // Not racey due to &mut self
        self.len = 0;
    }

    /// Access the location storage of an entity
    ///
    /// Must not be called on pending entities.
    pub fn get_mut(&mut self, entity: Entity) -> Result<&mut Location, NoSuchEntity> {
        let meta = self.meta.get_mut(entity.id as usize).ok_or(NoSuchEntity)?;
        if meta.generation == entity.generation {
            Ok(&mut meta.location)
        } else {
            Err(NoSuchEntity)
        }
    }

    /// Returns `Ok(Location { archetype: 0, index: undefined })` for pending entities
    pub fn get(&self, entity: Entity) -> Result<Location, NoSuchEntity> {
        if self.meta.len() <= entity.id as usize {
            return Ok(Location {
                archetype: 0,
                index: u32::max_value(),
            });
        }
        let meta = &self.meta[entity.id as usize];
        if meta.generation != entity.generation {
            return Err(NoSuchEntity);
        }
        Ok(meta.location)
    }

    /// Panics if the given id would represent an index outside of `meta`.
    ///
    /// # Safety
    /// Must only be called for currently allocated `id`s.
    pub unsafe fn resolve_unknown_gen(&self, id: u32) -> Entity {
        let meta_len = self.meta.len();

        if meta_len > id as usize {
            let meta = &self.meta[id as usize];
            Entity {
                generation: meta.generation,
                id,
            }
        } else {
            // See if it's pending, but not yet flushed.
            let free_cursor = self.free_cursor.load(Ordering::Relaxed);
            let num_pending = cmp::max(-free_cursor, 0) as usize;

            if meta_len + num_pending > id as usize {
                // Pending entities will have generation 0.
                Entity {
                    generation: NonZeroU32::new(1).unwrap(),
                    id,
                }
            } else {
                panic!("entity id is out of range");
            }
        }
    }

    fn needs_flush(&mut self) -> bool {
        // Not racey due to &mut self
        self.free_cursor.load(Ordering::Relaxed) != self.pending.len() as isize
    }

    /// Allocates space for entities previously reserved with `reserve_entity` or
    /// `reserve_entities`, then initializes each one using the supplied function.
    pub fn flush(&mut self, mut init: impl FnMut(u32, &mut Location)) {
        // Not racey due because of self is &mut.
        let free_cursor = self.free_cursor.load(Ordering::Relaxed);

        let new_free_cursor = if free_cursor >= 0 {
            free_cursor as usize
        } else {
            let old_meta_len = self.meta.len();
            let new_meta_len = old_meta_len + -free_cursor as usize;
            self.meta.resize(new_meta_len, EntityMeta::EMPTY);

            self.len += -free_cursor as u32;
            for (id, meta) in self.meta.iter_mut().enumerate().skip(old_meta_len) {
                init(id as u32, &mut meta.location);
            }

            self.free_cursor.store(0, Ordering::Relaxed);
            0
        };

        self.len += (self.pending.len() - new_free_cursor) as u32;
        for id in self.pending.drain(new_free_cursor..) {
            init(id, &mut self.meta[id as usize].location);
        }
    }

    #[inline]
    pub fn len(&self) -> u32 {
        self.len
    }
}

#[derive(Copy, Clone)]
pub(crate) struct EntityMeta {
    pub generation: NonZeroU32,
    pub location: Location,
}

impl EntityMeta {
    const EMPTY: EntityMeta = EntityMeta {
        generation: match NonZeroU32::new(1) {
            Some(x) => x,
            None => unreachable!(),
        },
        location: Location {
            archetype: 0,
            index: u32::max_value(), // dummy value, to be filled in
        },
    };
}

#[derive(Copy, Clone)]
pub(crate) struct Location {
    pub archetype: u32,
    pub index: u32,
}

/// Error indicating that no entity with a particular ID exists
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct NoSuchEntity;

impl fmt::Display for NoSuchEntity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad("no such entity")
    }
}

#[cfg(feature = "std")]
impl Error for NoSuchEntity {}

#[derive(Clone)]
pub(crate) struct AllocManyState {
    pub pending_end: usize,
    fresh: Range<u32>,
}

impl AllocManyState {
    pub fn next(&mut self, entities: &Entities) -> Option<u32> {
        if self.pending_end < entities.pending.len() {
            let id = entities.pending[self.pending_end];
            self.pending_end += 1;
            Some(id)
        } else {
            self.fresh.next()
        }
    }

    pub fn len(&self, entities: &Entities) -> usize {
        self.fresh.len() + (entities.pending.len() - self.pending_end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hashbrown::{HashMap, HashSet};
    use rand::{rngs::StdRng, Rng, SeedableRng};

    #[test]
    fn entity_bits_roundtrip() {
        let e = Entity {
            generation: NonZeroU32::new(0xDEADBEEF).unwrap(),
            id: 0xBAADF00D,
        };
        assert_eq!(Entity::from_bits(e.to_bits().into()).unwrap(), e);
    }

    #[test]
    fn alloc_and_free() {
        let mut rng = StdRng::seed_from_u64(0xFEEDFACEDEADF00D);

        let mut e = Entities::default();
        let mut first_unused = 0u32;
        let mut id_to_gen: HashMap<u32, u32> = Default::default();
        let mut free_set: HashSet<u32> = Default::default();
        let mut len = 0;

        for _ in 0..100 {
            let alloc = rng.gen_bool(0.7);
            if alloc || first_unused == 0 {
                let entity = e.alloc();
                len += 1;

                let id = entity.id;
                if !free_set.is_empty() {
                    // This should have come from the freelist.
                    assert!(free_set.remove(&id));
                } else if id >= first_unused {
                    first_unused = id + 1;
                }

                e.get_mut(entity).unwrap().index = 37;

                assert!(id_to_gen.insert(id, entity.generation.get()).is_none());
            } else {
                // Free a random ID, whether or not it's in use, and check for errors.
                let id = rng.gen_range(0..first_unused);

                let generation = id_to_gen.remove(&id);
                let entity = Entity {
                    id,
                    generation: NonZeroU32::new(
                        generation.unwrap_or_else(|| NonZeroU32::new(1).unwrap().get()),
                    )
                    .unwrap(),
                };

                assert_eq!(e.free(entity).is_ok(), generation.is_some());
                if generation.is_some() {
                    len -= 1;
                }

                free_set.insert(id);
            }
            assert_eq!(e.len(), len);
        }
    }

    #[test]
    fn alloc_at() {
        let mut e = Entities::default();

        let mut old = Vec::new();

        for _ in 0..2 {
            let entity = e.alloc();
            old.push(entity);
            e.free(entity).unwrap();
        }

        assert_eq!(e.len(), 0);

        let id = old.first().unwrap().id();
        assert!(old.iter().all(|entity| entity.id() == id));

        let entity = *old.last().unwrap();
        // The old ID shouldn't exist at this point, and should exist
        // in the pending list.
        assert!(!e.contains(entity));
        assert!(e.pending.contains(&entity.id()));
        // Allocating an entity at an unused location should not cause a location to be returned.
        assert!(e.alloc_at(entity).is_none());
        assert!(e.contains(entity));
        // The entity in question should not exist in the free-list once allocated.
        assert!(!e.pending.contains(&entity.id()));
        assert_eq!(e.len(), 1);
        // Allocating at the same id again should cause a location to be returned
        // this time around.
        assert!(e.alloc_at(entity).is_some());
        assert!(e.contains(entity));
        assert_eq!(e.len(), 1);

        // Allocating an Entity should cause the new empty locations
        // to be located in the free list.
        assert_eq!(e.meta.len(), 1);
        assert!(e
            .alloc_at(Entity {
                id: 3,
                generation: NonZeroU32::new(2).unwrap(),
            })
            .is_none());
        assert_eq!(e.pending.len(), 2);
        assert_eq!(&e.pending, &[1, 2]);
        assert_eq!(e.meta.len(), 4);
    }

    #[test]
    fn contains() {
        let mut e = Entities::default();

        for _ in 0..2 {
            let entity = e.alloc();
            assert!(e.contains(entity));

            e.free(entity).unwrap();
            assert!(!e.contains(entity));
        }

        // Reserved but not flushed are still "contained".
        for _ in 0..3 {
            let entity = e.reserve_entity();
            assert!(e.contains(entity));
            assert!(!e.contains(Entity {
                id: entity.id,
                generation: NonZeroU32::new(2).unwrap(),
            }));
            assert!(!e.contains(Entity {
                id: entity.id + 1,
                generation: NonZeroU32::new(1).unwrap(),
            }));
        }
    }

    // Shared test code parameterized by how we want to allocate an Entity block.
    fn reserve_test_helper(reserve_n: impl FnOnce(&mut Entities, u32) -> Vec<Entity>) {
        let mut e = Entities::default();

        // Allocate 10 items.
        let mut v1: Vec<Entity> = (0..10).map(|_| e.alloc()).collect();
        assert_eq!(v1.iter().map(|e| e.id).max(), Some(9));
        for &entity in v1.iter() {
            assert!(e.contains(entity));
            e.get_mut(entity).unwrap().index = 37;
        }

        // Put the last 4 on the freelist.
        for entity in v1.drain(6..) {
            e.free(entity).unwrap();
        }
        assert_eq!(e.free_cursor.load(Ordering::Relaxed), 4);

        // Reserve 10 entities, so 4 will come from the freelist.
        // This means we will have allocated 10 + 10 - 4 total items, so max id is 15.
        let v2 = reserve_n(&mut e, 10);
        assert_eq!(v2.iter().map(|e| e.id).max(), Some(15));

        // Reserved IDs still count as "contained".
        assert!(v2.iter().all(|&entity| e.contains(entity)));

        // We should have exactly IDs 0..16
        let mut v3: Vec<Entity> = v1.iter().chain(v2.iter()).copied().collect();
        assert_eq!(v3.len(), 16);
        v3.sort_by_key(|entity| entity.id);
        for (i, entity) in v3.into_iter().enumerate() {
            assert_eq!(entity.id, i as u32);
        }

        // 6 will come from pending.
        assert_eq!(e.free_cursor.load(Ordering::Relaxed), -6);

        let mut flushed = Vec::new();
        e.flush(|id, _| flushed.push(id));
        flushed.sort_unstable();

        assert_eq!(flushed, (6..16).collect::<Vec<_>>());
    }

    #[test]
    fn reserve_entity() {
        reserve_test_helper(|e, n| (0..n).map(|_| e.reserve_entity()).collect())
    }

    #[test]
    fn reserve_entities() {
        reserve_test_helper(|e, n| e.reserve_entities(n).collect())
    }

    #[test]
    fn reserve_grows() {
        let mut e = Entities::default();
        let _ = e.reserve_entity();
        e.flush(|_, _| {});
        assert_eq!(e.len(), 1);
    }

    #[test]
    fn reserve_grows_mixed() {
        let mut e = Entities::default();
        let a = e.alloc();
        e.alloc();
        e.free(a).unwrap();
        let _ = e.reserve_entities(3);
        e.flush(|_, _| {});
        assert_eq!(e.len(), 4);
    }
}
