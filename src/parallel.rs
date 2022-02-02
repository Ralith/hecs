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
struct Inner {
    pub archetype: u32,
    pub index: u32,
}

impl Inner {
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

impl From<u64> for Inner {
    fn from(value: u64) -> Self {
        Self {
            archetype: (value >> 32) as u32,
            index: value as u32,
        }
    }
}

#[derive(Debug)]
struct InnerStore(AtomicU64);

impl InnerStore {
    pub fn load(&self) -> Inner {
        let value = self.0.load(Ordering::Acquire);
        Inner {
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

impl From<Inner> for u64 {
    fn from(inner: Inner) -> Self {
        (inner.archetype as u64) << 32 | inner.index as u64
    }
}

struct ParallelIter<'a, Q: Query> {
    meta: &'a [EntityMeta],
    archetypes: &'a [Archetype],
    archetype: usize,
    range: (usize, usize),
    store: Arc<InnerStore>,
    partition: usize,
    _phantom: std::marker::PhantomData<Q>,
}

impl<'a, Q: Query> ParallelIter<'a, Q> {
    pub fn new(
        meta: &'a [EntityMeta],
        archetypes: &'a [Archetype],
        store: Arc<InnerStore>,
        partition: usize,
    ) -> Self {
        Self {
            meta,
            archetypes,
            archetype: 0,
            range: (0, 0),
            store,
            partition,
            _phantom: std::marker::PhantomData,
        }
    }
}

// Iterates the archetypes in partition or smaller sized chunks
// in a lockless cooperative parallel manner.
impl<'a, Q: Query> Iterator for ParallelIter<'a, Q> {
    type Item = (Entity, QueryItem<'a, Q>);

    fn next(&mut self) -> Option<Self::Item> {
        // Load the current tracking data.
        let mut store = self.store.load();

        loop {
            // Check if the range is valid and there are further elements
            // to iterate.
            if self.range.1 > self.range.0 {
                let index = self.range.0;
                self.range.0 += 1;

                let archetype = &self.archetypes[self.archetype];
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
            } else {
                // No valid range reserved currently, get one.
                let archetype = &self.archetypes[self.archetype];

                if self.archetype == store.archetype() {
                    if let Some(_state) = Q::Fetch::prepare(archetype) {
                        // Compatible archetype to take work from, if there is any.
                        if store.index() < archetype.len() as usize {
                            // And there is potentially remaining data.
                            let next: u64 = Inner::with(
                                self.archetype,
                                (store.index() + self.partition).min(archetype.len() as usize),
                            )
                            .into();

                            // Attempt to store the new inner.
                            match self.store.store(store.into(), next) {
                                Err(new_store) => {
                                    // Did not store the new data.
                                    // Store the changed iterator and try again.
                                    store = new_store.into();
                                    continue;
                                }
                                Ok(new_store) => {
                                    // Stored the value successfully, update for
                                    // iteration.
                                    self.range.0 = store.index();
                                    store = new_store.into();
                                    self.range.1 = store.index();
                                    self.archetype = store.archetype();
                                    continue;
                                }
                            }
                        }
                    }

                    // Current archetype is not compatible or has no elements left to take.
                    let old = store;
                    store.archetype += 1;
                    if store.archetype() >= self.archetypes.len() {
                        // All done.
                        return None;
                    } else {
                        let archetype = &self.archetypes[store.archetype()];
                        if let Some(_state) = Q::Fetch::prepare(archetype) {
                            // It's compatible so try to take some elements.
                            store.index = self.partition.min(archetype.len() as usize) as u32;
                            match self.store.store(old.into(), store.into()) {
                                Err(new_store) => {
                                    // We didn't make the exchange, try again.
                                    store = new_store.into();
                                }
                                Ok(_) => {
                                    // Exchange was successful.
                                    self.range.0 = 0;
                                    self.range.1 = store.index();
                                    self.archetype = store.archetype();
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
