// Implements a parallel archetype iterator.
// NOTES:
//  The benefits of using parallel execution depend highly on the
// number of entities and the threading solution in use.  With Rayon
// the benefits are notable for relatively expensive systems of more
// than a few hundred entities.  With other threading solutions which
// support overlapping parallel execution the benefits can be extreme.
//  Caller *MUST* carefully schedule the system execution order and
// properly barrier between incompatible systems.
//  Uses u32 indexing for archetypes and entity indexing.  If this is
// not enough.....  Uh, may god help you.....  The reasoning for the
// limitation is so a u64 compare and exchange can be used.
//  Uses low level access to world internals which may not be desirable
// but I haven't found another solution without threading issues.
use {
    super::{entities::EntityMeta, Archetype, Entity, Fetch, Query, QueryItem},
    std::sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

#[derive(Debug, Copy, Clone)]
struct SharedParts {
    pub archetype: u32,
    pub index: u32,
}

impl SharedParts {
    pub fn with(archetype: usize, index: usize) -> Self {
        Self {
            archetype: archetype as u32,
            index: index as u32,
        }
    }

    pub fn archetype(&self) -> usize {
        self.archetype as usize
    }

    pub fn index(&self) -> usize {
        self.index as usize
    }
}

impl From<u64> for SharedParts {
    fn from(value: u64) -> Self {
        Self {
            archetype: (value >> 32) as u32,
            index: value as u32,
        }
    }
}

impl From<SharedParts> for u64 {
    fn from(value: SharedParts) -> Self {
        (value.archetype as u64) << 32 | value.index as u64
    }
}

#[derive(Debug)]
struct Shared(AtomicU64);

impl Shared {
    pub fn new() -> Self {
        Self(AtomicU64::new(0))
    }

    pub fn load(&self) -> SharedParts {
        let value = self.0.load(Ordering::Acquire);
        SharedParts {
            archetype: (value >> 32) as u32,
            index: value as u32,
        }
    }

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

impl From<SharedParts> for Shared {
    fn from(parts: SharedParts) -> Self {
        Self(AtomicU64::new(parts.into()))
    }
}

/// A parallel iterator.
#[derive(Clone)]
pub struct ParallelIter<'a, Q: Query> {
    // Shared constants between threads.
    meta: &'a [EntityMeta],
    archetypes: &'a [Archetype],
    partition: usize,

    // Per thread owned data.
    archetype_index: usize,
    range: (usize, usize),

    // Shared among threads.
    thread_shared: Arc<Shared>,

    // ----
    _phantom: std::marker::PhantomData<Q>,
}

// The iterator is safe to send between threads but not sync.
unsafe impl<'a, Q: Query> Send for ParallelIter<'a, Q> {}

impl<'a, Q: Query> ParallelIter<'a, Q> {
    pub(crate) fn new(
        meta: &'a [EntityMeta],
        archetypes: &'a [Archetype],
        partition: usize,
    ) -> Self {
        // Find the first valid archetype for the query.
        let mut archetype_index: usize = 0;
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
            range: (0, 0),
            thread_shared: Arc::new(SharedParts::with(archetype_index, 0).into()),
            partition,
            _phantom: std::marker::PhantomData,
        }
    }

    /// If the iterator has a valid range assigned, get the next query result.
    fn next_in_range(&mut self) -> Option<(Entity, QueryItem<'a, Q>)> {
        loop {
            if self.range.0 >= self.range.1 {
                // Take a partition worth of elements.
                if !self.take_partition() {
                    // Nothing left, all done.
                    return None;
                }
            } else {
                let index = self.range.0;
                self.range.0 += 1;

                let archetype = &self.archetypes[self.archetype_index];
                // Must be valid at this point, it was checked in the archetype stepping.
                let state = Q::Fetch::prepare(archetype).unwrap();
                let fetch = Q::Fetch::execute(archetype, state);
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

    fn take_partition(&mut self) -> bool {
        // Load the current value of the shared thread data and break it down.
        let mut shared = self.thread_shared.load();

        loop {
            // Break down the shared information.
            let mut archetype_index = shared.archetype();
            let mut start_element = shared.index();

            // Check if there are elements remaining to take.
            if start_element >= self.archetypes[archetype_index].len() as usize {
                // Check for a new archetype.
                if !self.next_archetype() {
                    // Iteration has completed.
                    return false;
                }

                // Try with the new data.
                shared = self.thread_shared.load();
                archetype_index = shared.archetype();
                start_element = shared.index();
            }

            // Attempt to take a partition worth of elements.
            let end_element = start_element + self.partition;
            match self.thread_shared.store(
                shared.into(),
                SharedParts::with(archetype_index, end_element).into(),
            ) {
                Ok(_) => {
                    // Successfully took a partition.
                    self.range.0 = start_element;
                    self.range.1 = end_element.min(self.archetypes[archetype_index].len() as usize);
                    return true;
                }
                Err(new_value) => {
                    // Store the latest shared info and try again.
                    shared = new_value.into();
                }
            }
        }
    }

    fn next_archetype(&mut self) -> bool {
        // Move to the next archetype.
        let mut index = self.archetype_index.wrapping_add(1);

        while index < self.archetypes.len() {
            // Check that the new index refers to a valid archetype for the query.
            let archetype = &self.archetypes[self.archetype_index];

            // TODO: Store the prepare state?  Probably.
            if let Some(_) = Q::Fetch::prepare(archetype) {
                // It is valid.
                self.archetype_index = index;
                return true;
            }

            // The archetype is not valid for the query, try the next.
            // Load the shared data.
            let thread_shared = self.thread_shared.load();
            let shared_index = thread_shared.archetype();

            // Move to next archetype.
            index = shared_index + 1;
            if index < self.archetypes.len() {
                // Attempt to store the new shared data.
                match self.thread_shared.store(
                    thread_shared.into(),
                    SharedParts::with(shared_index, 0).into(),
                ) {
                    Ok(_) => {
                        // Shared data successfully updated.
                        self.range = (0, 0);
                    }
                    Err(new) => {
                        // Shared data changed by another thread.  Try again.
                        index = SharedParts::from(new).archetype();
                        continue;
                    }
                };
            } else {
                // No more archetypes to process.
                break;
            }
        }

        false
    }
}

// Iterates the archetypes in partition or smaller sized chunks
// in a lockless cooperative parallel manner.
impl<'a, Q: Query> Iterator for ParallelIter<'a, Q> {
    type Item = (Entity, QueryItem<'a, Q>);

    fn next(&mut self) -> Option<Self::Item> {
        self.next_in_range()
    }
}
