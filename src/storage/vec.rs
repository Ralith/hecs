use std::mem::MaybeUninit;
use std::ptr;

use super::Storage;

pub struct VecStorage<T>(Vec<MaybeUninit<T>>);

impl<T> Default for VecStorage<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<T: Send + 'static> Storage for VecStorage<T> {
    type Component = T;

    unsafe fn insert(&mut self, i: u32, x: Self::Component) {
        let i = i as usize;
        if i >= self.0.len() {
            self.0.resize_with(i + 1, || MaybeUninit::uninit());
        }
        ptr::write(self.0[i].as_mut_ptr(), x);
    }

    unsafe fn remove(&mut self, i: u32) -> Self::Component {
        ptr::read(self.0[i as usize].as_ptr())
    }

    unsafe fn get(&self, i: u32) -> &Self::Component {
        &*self.0[i as usize].as_ptr()
    }

    unsafe fn get_mut(&mut self, i: u32) -> &mut Self::Component {
        &mut *self.0[i as usize].as_mut_ptr()
    }
}
