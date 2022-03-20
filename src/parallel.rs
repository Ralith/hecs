//! Implements a parallel query iterator.
//!
//! Parallel execution of single systems is inherently unsafe without care being
//! taken to extend Rust borrow rules into the threading domain.
//!
use {
    super::{entities::EntityMeta, Archetype, Entity, Fetch, Query, QueryItem},
    std::sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

// Inner representation of the iterator.  This simply maintains two separate
// indices which are accessed by multiple threads to coordinate which entity
// ranges each thread will execute on.  Every entity matching the query will
// be iterated exactly once no matter how many threads the iterator is given
// to.
pub(crate) struct ParIterParts {
    archetype: u32,
    index: u32,
}

impl ParIterParts {
    /// Create a shared parts structure.
    /// `archetype` is the index of the archetype in `self.archetypes` which is being iterated.
    /// `index` is the entry index into the current archetype.
    pub fn with(archetype: usize, index: usize) -> Self {
        Self {
            archetype: archetype as u32,
            index: index as u32,
        }
    }

    /// Extract the archetype index from the parts.
    #[inline]
    pub fn archetype(&self) -> usize {
        self.archetype as usize
    }

    /// Extract the index from the parts.
    #[inline]
    pub fn index(&self) -> usize {
        self.index as usize
    }
}

/// The u64 is convertible to the iterator parts structure.
impl From<u64> for ParIterParts {
    fn from(value: u64) -> Self {
        Self {
            archetype: (value >> 32) as u32,
            index: value as u32,
        }
    }
}

/// The iterator parts structure is convertible to a u64.
impl From<ParIterParts> for u64 {
    fn from(parts: ParIterParts) -> Self {
        (parts.archetype as u64) << 32 | parts.index as u64
    }
}

/// An opaque parallel iteration handle.  The
/// contained atomic u64 is the in memory representation
/// of the ParIterParts structure.
#[derive(Debug, Clone)]
pub struct ParallelIter(Arc<AtomicU64>);

impl ParallelIter {
    /// Create a parallel iterator.
    /// This is an opaque tracking type which does not maintain any borrows,
    /// does not contain the query type or anything else.  Versus the iterator
    /// solution this makes it extremely 'safe'.
    pub fn new() -> Self {
        Self(Arc::new(AtomicU64::new(0)))
    }

    /// Perform an atomic load and return a shared parts structure.
    #[inline]
    pub(crate) fn load(&self) -> ParIterParts {
        let value = self.0.load(Ordering::Acquire);
        value.into()
    }

    /// Store a new value into the atomic.
    #[inline]
    pub fn store(&self, old: u64, new: u64) {
        let _ = self
            .0
            .compare_exchange(old, new, Ordering::Acquire, Ordering::Relaxed);
    }

    /// Execute the query.
    pub(crate) fn execute<'a, Q: Query>(
        &self,
        meta: &[EntityMeta],
        archetypes: &[Archetype],
        partition_size: usize,
        func: &dyn Fn(Entity, QueryItem<'a, Q>),
    ) {
        // Loop until we run out of archetypes to process.
        loop {
            let shared = self.load();
            let mut shared_archetype = shared.archetype();

            if shared_archetype < archetypes.len() {
                if let Some(archetype_state) = Q::Fetch::prepare(&archetypes[shared_archetype]) {
                    // This is a valid archetype to iterate, attempt to claim a range of entries to process.
                    let archetype = &archetypes[shared_archetype];
                    let start = shared.index();

                    if start < archetype.len() as usize {
                        // Compute the end, clamping it to the end of the archetype.
                        let end = (start + partition_size).min(archetype.len() as usize);

                        // Attempt to take the range.
                        match self.0.compare_exchange(
                            shared.into(),
                            ParIterParts::with(shared_archetype, end).into(),
                            Ordering::Acquire,
                            Ordering::Relaxed,
                        ) {
                            Ok(_) => {
                                // All good.
                                // Iterate the entities+components.
                                let fetch = Q::Fetch::execute(archetype, archetype_state);
                                let entities = archetype.entities().as_ptr();

                                for index in start..end {
                                    unsafe {
                                        let entity = *entities.add(index);
                                        let entity = Entity {
                                            id: entity,
                                            generation: meta
                                                .get_unchecked(entity as usize)
                                                .generation,
                                        };
                                        (func)(entity, fetch.get(index));
                                    }
                                }
                                continue;
                            }
                            Err(_) => {
                                // Try again from the top.
                                continue;
                            }
                        }
                    }
                }

                // Invalid archetype.  Try the next one.
                shared_archetype += 1;
                // Success or fail, restart the loop.
                self.store(
                    shared.into(),
                    ParIterParts::with(shared_archetype, 0).into(),
                );
            } else {
                // Done with the iteration.
                break;
            }
        }
    }
}
