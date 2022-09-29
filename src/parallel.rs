//! Implements a parallel query iterator.
//!
//! Parallel execution of single systems is inherently unsafe without care being
//! taken to extend Rust borrow rules into the threading domain.  A very simple
//! overview of what this means is as follows:
//!
//! Assume you have a hecs world with 3 components and are running 4 systems
//! on the world.
//! Components: C0, C1, C2, C3
//! Systems:
//!  S0::<&C0, &mut C1>()
//!  S1::<&C0, &C2, &mut C3>()
//!  S2::<&C1, &mut C2>()
//!  S3::<&C3>()
//!
//! Extending borrow rules into the threading domain is simply the case of looking
//! at the accessed components and making sure there are no mutable references to
//! a component at the same time as any other reference to that component.  There
//! are a number of ways to approach this programically but for the purposes here
//! we'll do it manually assuming that Rayon will be used to execute the systems
//! in parallel.
//!
//! Generally speaking, we care nothing about the systems and what they are doing,
//! the only thing which matters is what components are being used.  An easy way
//! to manually organize systems is to list out the components of all systems in
//! a table and make a note of where mutability of components exist.  So,
//!
//! S0 | &C0 | &mut C1 |          |         |
//! S1 | &C0 |         |  &C2     | &mut C3 |
//! S2 |     | &C1     |  &mut C2 |         |
//! S3 |     |         |          | &C3     |
//!
//! If you go component by component you see that all use of component C0 is safe to
//! be done simultaneously as it is only accessed immutably.  But, C1 is accessed
//! mutably in S0 and immutably in S2, making those systems incompatible.  Looking
//! further, there is also incompatibility between systems S1 and S2 due to the
//! component C2 being used with differing mutability.  And finally systems S1 and
//! S3 are incompatible due to C3 access.
//!
//! A very simplistic approach to how to issue these systems safely in a threaded
//! manner is to start at the top and checking for incompatibilities.  So, looking
//! at S0 and S1, there are no conflicting borrows between those two, we know they
//! can safely execute simultaneously.  Moving to S2, we note that there is an
//! incompatible access to C2, so we now have our first grouping, issue S0 and S1
//! simultaneously but do not issue S2 or later until they complete.  With Rayon
//! this would be the following in pseudo code:
//!
//! rayon::scope(s) {
//!   s.spawn(||{S0});
//!   s.spawn(||{S1});
//! }
//!
//! Because rayon will block until both S0 and S1 complete, we start by issuing S2
//! and then consider if S3 is compatible.  There are no conflicting component
//! access needs, so we can issue S3 in parallel with S2.  So, the resulting schedule
//! for issuing the systems is as follows:
//!
//! rayon::scope(s) {
//!   s.spawn(|| {execute s0});
//!   s.spawn(|| {execute s1});
//! }
//! rayon::scope(s) {
//!   s.spawn(|| {execute s2});
//!   s.spawn(|| {execute s3});
//! }
//!
//! The above uses 2 threads safely and follows all of Rust's rules.  Most likely
//! this has doubled your performance.  Well, that's not good enough, you probably
//! have at least 4 cores on your CPU, why not use them all?  That is where the
//! ParallelIter extension is used.  The parallel iterator partitions up the
//! component arrays across multiple threads simultaneously.  So, even if the array
//! of components is mutable, there are no borrow rules being broken because at any
//! given time, only one thread owns references to a specific portion of the array.
//! The iterator takes care of all of this efficiently behind the scenes in conjunction
//! with the parallel_query.  All the user has to do is issue one job for each thread.
//! The example becomes:
//!
//! rayon::scope(s) {
//!   let i0 = ParallelIter::new();
//!   let i1 = ParallelIter::new();
//!   for _ in 0..rayon::current_thread_count() {
//!     s.spawn(|| {execute s0(i0.clone())});
//!     s.spawn(|| {execute s1(i1.clone())});
//!   }
//! }
//! rayon::scope(s) {
//!   let i0 = ParallelIter::new();
//!   let i1 = ParallelIter::new();
//!   for _ in 0..rayon::current_thread_count() {
//!     s.spawn(|| {execute s2(i0.clone())});
//!     s.spawn(|| {execute s3(i1.clone())});
//!   }
//! }
//!
//! Iteration of the hecs systems will now fully utilize the CPU of the system.
//! Obviously this is assuming a perfect world where everything is naturally
//! parallel.  That is not always the case, some patterns do not parallelize
//! and there is considerable infrastructure required to manage the mixture of
//! possible cases.  Thankfully ECS 'encourages' most systems to be parallel by
//! nature such that their are many benefits to be had.  For a more complete
//! (perhaps obnoxiously so) example, see parallel_ffa example.  There is a
//! complete outline of automating the above sorting and rules enforcement
//! within a framework which could be extended to cover many further use cases.
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

    pub(crate) fn execute_mut<'a, Q: Query>(
        &self,
        meta: &[EntityMeta],
        archetypes: &[Archetype],
        partition_size: usize,
        func: &mut dyn FnMut(Entity, QueryItem<'a, Q>),
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
