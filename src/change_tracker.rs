use core::mem;

use alloc::vec::Vec;
use core::marker::PhantomData;

use crate::{Component, Entity, PreparedQuery, With, Without, World};

/// Helper to track changes in `T` components
///
/// For each entity with a `T` component, a private component is inserted which stores the value as
/// of the most recent call to `track`. This provides robust, exact change detection at the cost of
/// visiting each possibly-changed entity. It is a good fit for entities that will typically be
/// visited regardless, and components having fast [`Clone`] and [`PartialEq`] impls. For components
/// which are expensive to compare and/or clone, consider instead tracking changes manually, e.g.
/// by setting a flag in the component's `DerefMut` implementation.
///
/// `ChangeTracker` expect a `Flag` type parameter which is used to distinguish between multiple
/// trackers of the same component type. This is necessary to prevent multiple trackers from
/// interfering with each other. The `Flag` type should be a unique type like an empty struct.
///
/// Using the same tracker across multiple worlds, will produce unpredictable results.
///
/// # Example
/// ```rust
/// # use hecs::*;
/// let mut world = World::new();
///
/// // Create a change tracker for `i32` components with a unique id
/// struct Flag;
/// let mut tracker = ChangeTracker::<i32, Flag>::new();
///
/// // Spawn an entity with an `i32` component
/// {
///     world.spawn((42_i32,));
///     let mut changes = tracker.track(&mut world);
///     let added = changes.added().map(|(_, &value)| value).collect::<Vec<_>>();
///     assert_eq!(added, [42]);
/// }
///
/// // Modify the component
/// {
///     for (_, value) in world.query_mut::<&mut i32>() {
///         *value += 1;
///     }
///     let mut changes = tracker.track(&mut world);
///     let changes = changes.changed().map(|(_, old, &new)| (old, new)).collect::<Vec<_>>();
///     assert_eq!(changes, [(42, 43)]);
/// }
/// ```
pub struct ChangeTracker<T: Component, F: Send + Sync + 'static> {
    added: PreparedQuery<Without<&'static T, &'static Previous<T, F>>>,
    changed: PreparedQuery<(&'static T, &'static mut Previous<T, F>)>,
    removed: PreparedQuery<Without<With<(), &'static Previous<T, F>>, &'static T>>,

    added_components: Vec<(Entity, T)>,
    removed_components: Vec<Entity>,
}

impl<T: Component, F: Send + Sync + 'static> ChangeTracker<T, F> {
    /// Create a change tracker for `T` components
    pub fn new() -> Self {
        Self {
            added: PreparedQuery::new(),
            changed: PreparedQuery::new(),
            removed: PreparedQuery::new(),

            added_components: Vec::new(),
            removed_components: Vec::new(),
        }
    }

    /// Determine the changes in `T` components in `world` since the previous call
    pub fn track<'a>(&'a mut self, world: &'a mut World) -> Changes<'a, T, F>
    where
        T: Clone + PartialEq,
    {
        Changes {
            tracker: self,
            world,
            added: false,
            changed: false,
            removed: false,
        }
    }
}

impl<T: Component, F: Send + Sync + 'static> Default for ChangeTracker<T, F> {
    fn default() -> Self {
        Self::new()
    }
}

struct Previous<T, F>(T, PhantomData<F>);

/// Collection of iterators over changes in `T` components given a flag `F`
pub struct Changes<'a, T, F>
where
    T: Component + Clone + PartialEq,
    F: Send + Sync + 'static,
{
    tracker: &'a mut ChangeTracker<T, F>,
    world: &'a mut World,
    added: bool,
    changed: bool,
    removed: bool,
}

impl<'a, T, F> Changes<'a, T, F>
where
    T: Component + Clone + PartialEq,
    F: Send + Sync + 'static,
{
    /// Iterate over entities which were given a new `T` component after the preceding
    /// [`track`](ChangeTracker::track) call, including newly spawned entities
    pub fn added(&mut self) -> impl ExactSizeIterator<Item = (Entity, &T)> + '_ {
        self.tracker.added_components.clear();
        self.added = true;
        DrainOnDrop(
            self.tracker
                .added
                .query_mut(self.world)
                .inspect(|&(e, x)| self.tracker.added_components.push((e, x.clone()))),
        )
    }

    /// Iterate over `(entity, old, new)` for entities whose `T` component has changed according to
    /// [`PartialEq`] after the preceding [`track`](ChangeTracker::track) call
    pub fn changed(&mut self) -> impl Iterator<Item = (Entity, T, &T)> + '_ {
        self.changed = true;
        DrainOnDrop(
            self.tracker
                .changed
                .query_mut(self.world)
                .filter_map(|(e, (new, old))| {
                    (*new != old.0).then(|| {
                        let old = mem::replace(&mut old.0, new.clone());
                        (e, old, new)
                    })
                }),
        )
    }

    /// Iterate over entities which lost their `T` component after the preceding
    /// [`track`](ChangeTracker::track) call, excluding any entities which were despawned
    pub fn removed(&mut self) -> impl ExactSizeIterator<Item = (Entity, T)> + '_ {
        self.tracker.removed_components.clear();
        self.removed = true;
        // TODO: We could make this much more efficient by introducing a mechanism for queries to
        // take ownership of components directly.
        self.tracker
            .removed_components
            .extend(self.tracker.removed.query_mut(self.world).map(|(e, ())| e));
        DrainOnDrop(
            self.tracker
                .removed_components
                .drain(..)
                .map(|e| (e, self.world.remove_one::<Previous<T, F>>(e).unwrap().0)),
        )
    }
}

impl<'a, T: Component, F: Send + Sync + 'static> Drop for Changes<'a, T, F>
where
    T: Component + Clone + PartialEq,
{
    fn drop(&mut self) {
        if !self.added {
            _ = self.added();
        }
        for (entity, component) in self.tracker.added_components.drain(..) {
            self.world
                .insert_one(entity, Previous(component, PhantomData::<F>))
                .unwrap();
        }
        if !self.changed {
            _ = self.changed();
        }
        if !self.removed {
            _ = self.removed();
        }
    }
}

/// Helper to ensure an iterator visits every element so that we can rely on the iterator's side
/// effects
struct DrainOnDrop<T: Iterator>(T);

impl<T: Iterator> Iterator for DrainOnDrop<T> {
    type Item = T::Item;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }
}

impl<T: ExactSizeIterator> ExactSizeIterator for DrainOnDrop<T> {
    fn len(&self) -> usize {
        self.0.len()
    }
}

impl<T: Iterator> Drop for DrainOnDrop<T> {
    fn drop(&mut self) {
        for _ in &mut self.0 {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke() {
        let mut world = World::new();

        let a = world.spawn((42,));
        let b = world.spawn((17, false));
        let c = world.spawn((true,));

        struct Flag1;
        struct Flag2;
        let mut tracker1 = ChangeTracker::<i32, Flag1>::new();
        let mut tracker2 = ChangeTracker::<i32, Flag2>::new();

        {
            let mut changes = tracker1.track(&mut world);
            let added = changes.added().collect::<Vec<_>>();
            assert_eq!(added.len(), 2);
            assert!(added.contains(&(a, &42)));
            assert!(added.contains(&(b, &17)));
            assert_eq!(changes.changed().count(), 0);
            assert_eq!(changes.removed().count(), 0);
        }
        {
            let mut changes = tracker2.track(&mut world);
            let added = changes.added().collect::<Vec<_>>();
            assert_eq!(added.len(), 2);
            assert!(added.contains(&(a, &42)));
            assert!(added.contains(&(b, &17)));
            assert_eq!(changes.changed().count(), 0);
            assert_eq!(changes.removed().count(), 0);
        }

        world.remove_one::<i32>(a).unwrap();
        *world.get::<&mut i32>(b).unwrap() = 26;
        world.insert_one(c, 74).unwrap();
        {
            let mut changes = tracker1.track(&mut world);
            assert_eq!(changes.removed().collect::<Vec<_>>(), [(a, 42)]);
            assert_eq!(changes.changed().collect::<Vec<_>>(), [(b, 17, &26)]);
            assert_eq!(changes.added().collect::<Vec<_>>(), [(c, &74)]);
        }
        {
            let mut changes = tracker1.track(&mut world);
            assert_eq!(changes.removed().collect::<Vec<_>>(), []);
            assert_eq!(changes.changed().collect::<Vec<_>>(), []);
            assert_eq!(changes.added().collect::<Vec<_>>(), []);
        }

        {
            let mut changes = tracker2.track(&mut world);
            assert_eq!(changes.removed().collect::<Vec<_>>(), [(a, 42)]);
            assert_eq!(changes.changed().collect::<Vec<_>>(), [(b, 17, &26)]);
            assert_eq!(changes.added().collect::<Vec<_>>(), [(c, &74)]);
        }
        {
            let mut changes = tracker2.track(&mut world);
            assert_eq!(changes.removed().collect::<Vec<_>>(), []);
            assert_eq!(changes.changed().collect::<Vec<_>>(), []);
            assert_eq!(changes.added().collect::<Vec<_>>(), []);
        }
    }
}
