// Implements a parallel query iterator.
//
// When individual systems become fairly heavy weight or the entity
// counts are in the 10k+ ranges, parallel iteration can be used to
// break the single threaded bottlenecks of long running systems.
// The benefits depend grealy on the underlying threading solution
// in use and what alternatives to synchronization points are
// available.
//
// This literally breaks every borrow and lifetime rule in the
// language and is absolutely without a doubt unsafe.  Some of this
// can be cleaned up but the first likely change will make things
// even less safe.  The intention at the moment is to get feedback on
// some of the ways I worked around issues and try to get the core
// behaviors cleaned up.  Additionally, the primary thinking about
// not being safe here is that someone actually using it in potentially
// unsafe ways better know what they are doing anyway.  The user is
// going to be responsible for creating a scheduling system which
// replicates the borrow/lifetime rules but rather than the language
// rules done on control flow, they will be replicating them in the
// time domain of the threading system and job lifetimes.
// 
use {
    super::{entities::EntityMeta, Archetype, Entity, Fetch, Query, QueryItem},
    std::sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

/// Represents the two components of the shared data in the iterator.
#[derive(Debug, Copy, Clone)]
#[repr(C)]
struct SharedParts {
    archetype: u32,
    index: u32,
}

impl SharedParts {
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

/// `u64` is the atomic representation, it can be converted to a parts structure.
impl From<u64> for SharedParts {
    fn from(value: u64) -> Self {
        Self {
            archetype: (value >> 32) as u32,
            index: value as u32,
        }
    }
}

/// SharedParts can be converted back to the raw u64 representation.
impl From<SharedParts> for u64 {
    fn from(value: SharedParts) -> Self {
        (value.archetype as u64) << 32 | value.index as u64
    }
}

/// New type around the atomic u64.
#[derive(Debug)]
struct Shared(AtomicU64);

impl Shared {
    /// Perform an atomic load and return a shared parts structure.
    #[inline]
    pub fn load(&self) -> SharedParts {
        let value = self.0.load(Ordering::Acquire);
        SharedParts {
            archetype: (value >> 32) as u32,
            index: value as u32,
        }
    }

    /// Store a new value into the atomic.
    /// Returns:
    /// Ok(`new`) if the exchange was successful.
    /// Err(`x`) if the exchange failed, `x` is the changed value found within the atomic.
    #[inline]
    pub fn store(&self, old: u64, new: u64) -> Result<u64, u64> {
        match self
            .0
            .compare_exchange(old, new, Ordering::Acquire, Ordering::Relaxed)
        {
            // Successfully stored.
            Ok(new) => Ok(new),
            // Failed, return the new value found.
            Err(new) => Err(new),
        }
    }
}

/// SharedParts can be turned into a Shared type wrapper.
impl From<SharedParts> for Shared {
    fn from(parts: SharedParts) -> Self {
        Self(AtomicU64::new(parts.into()))
    }
}

/// A parallel iterator.
pub struct ParallelIter<'a, Q: Query> {
    // Shared constants between threads.
    // Copying these locally is generally better for the caches but
    // not always.  Will have to experiment with variations based on
    // how Rust accesses the data.
    meta: &'a [EntityMeta],
    archetypes: &'a [Archetype],
    partition: usize,

    // Per thread owned state.
    archetype_index: usize,
    state: Option<<Q::Fetch as Fetch<'a>>::State>,
    range: (usize, usize),

    // Shared among threads.
    thread_shared: Arc<Shared>,

    // ----
    _phantom: std::marker::PhantomData<Q>,
}

impl<'a, Q: Query> Clone for ParallelIter<'a, Q> {
    fn clone(&self) -> Self {
        Self {
            meta: self.meta,
            archetypes: self.archetypes,
            partition: self.partition,
            archetype_index: self.archetype_index,
            state: Q::Fetch::prepare(&self.archetypes[self.archetype_index]),
            range: self.range,
            thread_shared: self.thread_shared.clone(),
            _phantom: self._phantom,
        }
    }
}

// The iterator is safe to send between threads but not sync.
unsafe impl<'a, Q: Query> Send for ParallelIter<'a, Q> {}

impl<'a, Q: Query> ParallelIter<'a, Q> {
    /// Create a new parallel iter.
    pub(crate) fn new(
        meta: &'a [EntityMeta],
        archetypes: &'a [Archetype],
        partition: usize,
    ) -> Self {
        // Find the first valid archetype for the query.
        // Doing this here prevents a bunch of first run tests.
        let mut archetype_index: usize = !0;
        for (index, archetype) in archetypes.iter().enumerate() {
            if let Some(_) = Q::Fetch::prepare(archetype) {
                archetype_index = index;
                break;
            }
        }

        Self {
            meta,
            archetypes,
            archetype_index,
            state: None,
            range: (0, 0),
            thread_shared: Arc::new(SharedParts::with(archetype_index, 0).into()),
            partition,
            _phantom: std::marker::PhantomData,
        }
    }

    /// If the iterator has a valid range assigned, get the next query result.
    /// TODO: Likely there are a number of optimizations and cleanups here, it's
    /// a functional first pass only.
    fn next_in_range(&mut self) -> Option<(Entity, QueryItem<'a, Q>)> {
        loop {
            if self.range.0 >= self.range.1 {
                // Take a partition worth of elements.
                if !self.take_range() {
                    // Nothing left, all done.
                    return None;
                }
            } else {
                // The range is valid so keep stepping through the elements.
                let index = self.range.0;
                self.range.0 += 1;

                let archetype = &self.archetypes[self.archetype_index];
                let fetch = Q::Fetch::execute(archetype, self.state.unwrap());
                let entities = archetype.entities().as_ptr();

                unsafe {
                    let entity = *entities.add(index);
                    let entity = Entity {
                        id: entity,
                        generation: self.meta.get_unchecked(entity as usize).generation,
                    };
                    return Some((entity, fetch.get(index)));
                }
            }
        }
    }

    /// The iterator does not have a valid range, attempt to take one.
    fn take_range(&mut self) -> bool {
        // As long as this threads archetype index is valid, keep trying.
        while self.archetype_index < self.archetypes.len() {
            // Load the current value of the shared thread data and break it down.
            let shared = self.thread_shared.load();

            // Get the current shared start index.
            let start_element = shared.index();

            // Check that the archetypes still match and that start element is valid.
            if shared.archetype() != self.archetype_index
                || start_element >= self.archetypes[self.archetype_index].len() as usize
            {
                // Move to the next archetype.
                if !self.next_archetype() {
                    // Iteration has completed.
                    return false;
                }

                // Try with the new data.
                continue;
            }

            // Attempt to take a slice with partion worth of elements.
            let end_element = start_element + self.partition;
            match self.thread_shared.store(
                shared.into(),
                SharedParts::with(self.archetype_index, end_element).into(),
            ) {
                Ok(_) => {
                    // Successfully reserved the new range.
                    self.range = (
                        start_element,
                        end_element.min(self.archetypes[self.archetype_index].len() as usize),
                    );
                    return true;
                }
                Err(_) => {}
            }
        }

        false
    }

    /// Update the internal fetch state if the archetype is valid for the query.
    fn update_state(&mut self) -> bool {
        if self.archetype_index < self.archetypes.len() {
            self.state = Q::Fetch::prepare(&self.archetypes[self.archetype_index]);
            return self.state.is_some();
        }
        false
    }

    /// The iterator refers to an archetype which has been completely iterated, attempt to
    /// find the next valid archetype.
    fn next_archetype(&mut self) -> bool {
        // Loop till we get a valid archetype or end the iteration.
        while self.archetype_index < self.archetypes.len() {
            let thread_shared = self.thread_shared.load();

            // Check if another thread has already advanced the archetype index.
            if self.archetype_index < thread_shared.archetype() {
                self.archetype_index = thread_shared.archetype();
                return self.update_state();
            }

            // Find the next valid archetype.
            let index = self.archetype_index + 1;
            if index >= self.archetypes.len() {
                // Done iterating.
                return false;
            } else {
                self.state = Q::Fetch::prepare(&self.archetypes[index]);
                if self.state.is_none() {
                    self.archetype_index = index;
                    continue;
                }
            }

            // We have found a valid archetype, try to store the updated shared data.
            match self
                .thread_shared
                .store(thread_shared.into(), SharedParts::with(index, 0).into())
            {
                Ok(_) => {
                    // Shared data successfully updated.
                    self.archetype_index = index;
                    return true;
                }
                Err(_) => {}
            };
        }

        false
    }
}

// Iterates the archetypes in partition or smaller sized chunks
// in a lockless cooperative manner.
impl<'a, Q: Query> Iterator for ParallelIter<'a, Q> {
    type Item = (Entity, QueryItem<'a, Q>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.next_in_range()
    }
}
