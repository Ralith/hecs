use fxhash::FxHashMap;

use super::Storage;

pub struct HashStorage<T>(FxHashMap<u32, T>);

impl<T> Default for HashStorage<T> {
    fn default() -> Self {
        Self(FxHashMap::default())
    }
}

impl<T: Send + 'static> Storage for HashStorage<T> {
    type Component = T;

    unsafe fn insert(&mut self, i: u32, x: Self::Component) {
        self.0.insert(i, x);
    }

    unsafe fn remove(&mut self, i: u32) -> Self::Component {
        self.0.remove(&i).unwrap()
    }

    unsafe fn get(&self, i: u32) -> &Self::Component {
        self.0.get(&i).unwrap()
    }

    unsafe fn get_mut(&mut self, i: u32) -> &mut Self::Component {
        self.0.get_mut(&i).unwrap()
    }
}
