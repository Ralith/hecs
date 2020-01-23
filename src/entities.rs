use alloc::vec::Vec;
use core::fmt;
#[cfg(feature = "std")]
use std::error::Error;

/// Lightweight unique ID of an entity
///
/// Obtained from `World::spawn`. Can be stored to refer to an entity in the future.
#[derive(Clone, Copy, Hash, Eq, Ord, PartialEq, PartialOrd)]
pub struct Entity {
    pub(crate) generation: u32,
    pub(crate) id: u32,
}

impl Entity {
    /// Convert to a form convenient for passing outside of rust
    ///
    /// Only useful for identifying entities within the same instance of an application. Do not use
    /// for serialization between runs.
    ///
    /// No particular structure is guaranteed for the returned bits.
    pub fn to_bits(self) -> u64 {
        u64::from(self.generation) << 32 | u64::from(self.id)
    }

    /// Reconstruct an `Entity` previously destructured with `to_bits`
    ///
    /// Only useful when applied to results from `to_bits` in the same instance of an application.
    pub fn from_bits(bits: u64) -> Self {
        Self {
            generation: (bits >> 32) as u32,
            id: bits as u32,
        }
    }

    /// Extract a transiently unique identifier
    ///
    /// No two simultaneously-live entities share the same ID, but dead entities' IDs may collide
    /// with both live and dead entities. Useful for compactly representing entities within a
    /// specific snapshot of the world, such as when serializing.
    pub fn id(self) -> u32 {
        self.id
    }
}

impl fmt::Debug for Entity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}v{}", self.id, self.generation)
    }
}

#[derive(Default)]
pub(crate) struct Entities {
    pub meta: Vec<EntityMeta>,
    free: Vec<u32>,
}

impl Entities {
    pub fn alloc(&mut self) -> Entity {
        match self.free.pop() {
            Some(i) => Entity {
                generation: self.meta[i as usize].generation,
                id: i,
            },
            None => {
                let i = self.meta.len() as u32;
                self.meta.push(EntityMeta {
                    generation: 0,
                    location: Location {
                        archetype: 0,
                        index: 0,
                    },
                });
                Entity {
                    generation: 0,
                    id: i,
                }
            }
        }
    }

    pub fn free(&mut self, entity: Entity) -> Result<Location, NoSuchEntity> {
        let meta = &mut self.meta[entity.id as usize];
        if meta.generation != entity.generation {
            return Err(NoSuchEntity);
        }
        meta.generation += 1;
        self.free.push(entity.id);
        Ok(meta.location)
    }

    pub fn clear(&mut self) {
        self.meta.clear();
        self.free.clear();
    }

    pub fn get_mut(&mut self, entity: Entity) -> Result<&mut Location, NoSuchEntity> {
        let meta = &mut self.meta[entity.id as usize];
        if meta.generation != entity.generation {
            return Err(NoSuchEntity);
        }
        Ok(&mut meta.location)
    }

    pub fn get(&self, entity: Entity) -> Result<Location, NoSuchEntity> {
        let meta = &self.meta[entity.id as usize];
        if meta.generation != entity.generation {
            return Err(NoSuchEntity);
        }
        Ok(meta.location)
    }
}

#[derive(Copy, Clone)]
pub(crate) struct EntityMeta {
    pub generation: u32,
    pub location: Location,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_bits_roundtrip() {
        let e = Entity {
            generation: 0xDEADBEEF,
            id: 0xBAADF00D,
        };
        assert_eq!(Entity::from_bits(e.to_bits()), e);
    }
}
