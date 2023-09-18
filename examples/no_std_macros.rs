#![no_std]

use hecs::{Bundle, Query};

#[derive(Bundle)]
pub struct Foo {
    pub foo: i32,
}

#[derive(Bundle)]
pub struct Bar {
    pub foo: i32,
}

#[derive(Bundle)]
pub struct Baz {
    pub foo: i32,
    pub baz: &'static str,
}

#[derive(Query)]
pub struct Quux<'a> {
    pub foo: &'a i32,
    pub bar: &'a mut bool,
}

#[panic_handler]
#[cfg(target_os = "none")]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

struct NoAlloc;

unsafe impl core::alloc::GlobalAlloc for NoAlloc {
    unsafe fn alloc(&self, _: core::alloc::Layout) -> *mut u8 {
        unreachable!()
    }
    unsafe fn dealloc(&self, _: *mut u8, _: core::alloc::Layout) {
        unreachable!()
    }
}

#[global_allocator]
static ALLOC: NoAlloc = NoAlloc;
