use core::mem;

use alloc::vec::Vec;

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
/// Always use exactly one `ChangeTracker` per [`World`] per component type of interest. Using
/// multiple trackers of the same `T` on the same world, or using the same tracker across multiple
/// worlds, will produce unpredictable results.
pub struct ChangeTracker<T: Component> {
    added: PreparedQuery<Without<&'static T, &'static Previous<T>>>,
    changed: PreparedQuery<(&'static T, &'static mut Previous<T>)>,
    removed: PreparedQuery<Without<With<(), &'static Previous<T>>, &'static T>>,

    added_components: Vec<(Entity, T)>,
    removed_components: Vec<Entity>,
}

impl<T: Component> ChangeTracker<T> {
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
    pub fn track<'a>(&'a mut self, world: &'a mut World) -> Changes<'a, T>
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

impl<T: Component> Default for ChangeTracker<T> {
    fn default() -> Self {
        Self::new()
    }
}

struct Previous<T>(T);

/// Collection of iterators over changes in `T` components
pub struct Changes<'a, T>
where
    T: Component + Clone + PartialEq,
{
    tracker: &'a mut ChangeTracker<T>,
    world: &'a mut World,
    added: bool,
    changed: bool,
    removed: bool,
}

impl<'a, T> Changes<'a, T>
where
    T: Component + Clone + PartialEq,
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
                .map(|e| (e, self.world.remove_one::<Previous<T>>(e).unwrap().0)),
        )
    }
}

impl<'a, T: Component> Drop for Changes<'a, T>
where
    T: Component + Clone + PartialEq,
{
    fn drop(&mut self) {
        if !self.added {
            _ = self.added();
        }
        for (entity, component) in self.tracker.added_components.drain(..) {
            self.world.insert_one(entity, Previous(component)).unwrap();
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

        let mut tracker = ChangeTracker::<i32>::new();
        {
            let mut changes = tracker.track(&mut world);
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
            let mut changes = tracker.track(&mut world);
            assert_eq!(changes.removed().collect::<Vec<_>>(), [(a, 42)]);
            assert_eq!(changes.changed().collect::<Vec<_>>(), [(b, 17, &26)]);
            assert_eq!(changes.added().collect::<Vec<_>>(), [(c, &74)]);
        }
        {
            let mut changes = tracker.track(&mut world);
            assert_eq!(changes.removed().collect::<Vec<_>>(), []);
            assert_eq!(changes.changed().collect::<Vec<_>>(), []);
            assert_eq!(changes.added().collect::<Vec<_>>(), []);
        }
    }
}
