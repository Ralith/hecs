mod dynamic;
mod storage;

pub use dynamic::*;
pub use storage::*;

use hibitset::{BitSet, BitSetLike, BitSetNot};

#[derive(Clone, Copy, Debug, Hash, Eq, Ord, PartialEq, PartialOrd)]
pub struct Entity {
    generation: u32,
    index: u32,
}

#[derive(Default)]
struct Entities {
    occupancy: BitSet,
    generations: Vec<u32>,
}

impl Entities {
    fn spawn(&mut self) -> Entity {
        let index = BitSetNot(&self.occupancy).iter().next().unwrap();
        self.occupancy.add(index);
        if index as usize >= self.generations.len() {
            self.generations.resize(index as usize + 1, 0);
        }
        let generation = self.generations[index as usize];
        Entity { generation, index }
    }

    /// Whether `entity` currently exists
    fn contains(&self, entity: Entity) -> bool {
        self.occupancy.contains(entity.index)
            && self.generations[entity.index as usize] == entity.generation
    }

    /// Destroy an entity and all associated components
    ///
    /// Returns `false` iff the entity was previously destroyed
    fn despawn(&mut self, entity: Entity) -> bool {
        if !self.contains(entity) {
            return false;
        }
        self.generations[entity.index as usize] =
            self.generations[entity.index as usize].wrapping_add(1);
        self.occupancy.remove(entity.index);
        true
    }
}

pub trait Fetch<T> {
    type Ref;
    fn fetch(self) -> Self::Ref;
}

macro_rules! tuple_impl {
    ($($name:ident),*) => {
        impl<'a, T, $($name),*> Fetch<($($name),*)> for &'a T
            where $(Self: Fetch<$name>),*
        {
            type Ref = ($(<Self as Fetch<$name>>::Ref),*);
            fn fetch(self) -> Self::Ref {
                ($(<Self as Fetch<$name>>::fetch(self)),*)
            }
        }
    }
}

tuple_impl!(A, B);
tuple_impl!(A, B, C);
tuple_impl!(A, B, C, D);
tuple_impl!(A, B, C, D, E);
tuple_impl!(A, B, C, D, E, F);
tuple_impl!(A, B, C, D, E, F, G);
tuple_impl!(A, B, C, D, E, F, G, H);
tuple_impl!(A, B, C, D, E, F, G, H, I);
tuple_impl!(A, B, C, D, E, F, G, H, I, J);
tuple_impl!(A, B, C, D, E, F, G, H, I, J, K);
tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L);
tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M);
tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N);
tuple_impl!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O);

#[macro_export]
macro_rules! world {
    {
        $(#[$meta:meta])*
        $vis:vis struct $name:ident {
            $($field:ident : $ty:ty),*$(,)?
        }
    } => {
        #[derive(Default)]
        struct Storages {
            $($field : std::sync::Mutex<$crate::Masked<$ty>>,)*
        }

        $(#[$meta])*
        #[derive(Default)]
        $vis struct $name {
            entities: Entities,
            storages: Storages,
        }

        #[allow(dead_code)]
        impl $name {
            pub fn new() -> Self { Self::default() }

            /// Access one or more storages
            pub fn get<'a, T>(&'a self) -> <&'a Self as Fetch<T>>::Ref
            where
                &'a Self: Fetch<T>,
            {
                self.fetch()
            }

            /// Create a new `Entity`
            pub fn spawn(&mut self) -> Entity {
                self.entities.spawn()
            }

            /// Whether `entity` currently exists
            pub fn contains(&self, entity: Entity) -> bool {
                self.entities.contains(entity)
            }

            /// Destroy an entity and all associated components
            ///
            /// Returns `false` iff the entity was previously destroyed
            pub fn despawn(&mut self, entity: Entity) -> bool {
                let was_live = self.entities.despawn(entity);
                if was_live {
                    $(
                        let mut storage = self.storages.$field.try_lock().expect("storage already borrowed");
                        storage.free(entity.index);
                    )*
                }
                was_live
            }
        }

        $(
            impl<'a> Fetch<$ty> for &'a $name {
                type Ref = std::sync::MutexGuard<'a, Masked<$ty>>;
                fn fetch(self) -> std::sync::MutexGuard<'a, Masked<$ty>> {
                    self
                        .storages
                        .$field
                        .try_lock()
                        .expect(concat!("storage ", stringify!($field), " already borrowed"))
                }
            }
        )*
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    world! {
        struct Static {
            position: VecStorage<u32>,
        }
    }

    #[test]
    fn smoke() {
        let mut world = DynWorld::new();
        world.register::<VecStorage<u32>>();
        world.register::<VecStorage<u16>>();
        let entity = world.spawn();
        world.insert::<VecStorage<u32>>(entity, 32);
        world.insert::<VecStorage<u16>>(entity, 16);

        {
            let s = world.get::<VecStorage<u32>>();
            assert_eq!(s.iter().cloned().collect::<Vec<_>>(), [32]);

            assert_eq!((&s, &s, &s).join().collect::<Vec<_>>(), [(&32, &32, &32)]);
        }

        {
            let (s, mut t) = world.get::<(VecStorage<u32>, VecStorage<u16>)>();
            assert_eq!((&s, &mut t).join().collect::<Vec<_>>(), [(&32, &mut 16)]);
        }

        assert!(world.contains(entity));
        assert_eq!(world.remove::<VecStorage<u32>>(entity), Some(32));
        assert_eq!(world.remove::<VecStorage<u32>>(entity), None);
        assert!(world.despawn(entity));
        assert!(!world.contains(entity));
    }

    #[test]
    #[should_panic(expected = "already borrowed")]
    fn double_borrow() {
        let mut world = DynWorld::new();
        world.register::<VecStorage<u32>>();
        world.get::<(VecStorage<u32>, VecStorage<u32>)>();
    }
}
